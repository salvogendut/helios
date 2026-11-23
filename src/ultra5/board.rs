#[cfg(test)]
mod countdown_tests;

use super::dram::Dram;
use super::keyboard_uart::KeyboardUart;
use super::pbm::{Pbm, PCI_CONFIG};
use super::pcio::{PciFunction, BAR1 as EBUS_BAR1};
use super::serial::Serial;
use super::simba::Simba;
use super::{BootFlash, FlashFetch, ResetNvram, FLASH_BASE};
use crate::v9::{CpuFault, Translation, V9Cpu, V9DataBus};

const APB_TICK: u64 = PCI_CONFIG + 0x08b0; // primary bus, device 1 function 0
const PCI_MEMORY: u64 = 0x01ff_0000_0000;
const SIO_INDEX: u64 = 0x30_0398;
const SIO_DATA: u64 = SIO_INDEX + 1;
const KEYBOARD_UART: u64 = 0x30_83f8;
const MOUSE_UART: u64 = 0x30_62f8;
const AUXIO_LED: u64 = PCI_MEMORY + EBUS_BAR1 as u64 + 0x72_6000;
const MEM_CONTROL0: u64 = 0x01fe_0000_f010;
const MEM_CONTROL1: u64 = 0x01fe_0000_f018;
const RESET_CONTROL: u64 = 0x01fe_0000_f020;
// UltraSPARC-IIi table 17-2. The three external causes are write-one-to-clear.
const POR: u64 = 1 << 31;
const SOFT_RESET: u64 = (1 << 30) | (1 << 29);
const EXTERNAL_RESET: u64 = POR | (1 << 28) | (1 << 27);

/// A retired board instruction fetched from flash or installed DRAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardInstruction {
    pub physical_address: u64,
    pub instruction: u32,
}

/// A trap enters the guest handler without retiring the faulting instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardStep {
    Retired(BoardInstruction),
    InterruptTaken { pc: u64, level: u8, vector: u64 },
    TrapTaken { fault_pc: u64, tt: u16, vector: u64 },
}

/// The Ultra 5 reset-time bus. Flash supplies both instructions and the
/// bytes in OBP's serial initialization table. All devices persist across steps.
pub struct Ultra5ResetBoard {
    flash: BootFlash,
    nvram: ResetNvram,
    dram: Dram,
    pbm: Pbm,
    host_primary: u8,
    host_subordinate: u8,
    apb: Simba,
    pci_functions: [PciFunction; 4],
    ebus_setup: [u32; 3],
    disk: Option<super::ata::Disk>,
    dma_writes: Vec<(u64, usize)>,
    cdrom: Option<super::atapi::Cdrom>,
    hme: super::hme::Mif,
    display: super::mach64::Mach64,
    audio: crate::devices::cs4231::Cs4231,
    floppy_aux: u32,
    ebus_dma: [[u32; 3]; 4],
    floppy: crate::devices::floppy::Floppy,
    floppy_last_ns: u64,
    front_panel_led: bool,
    reset_cause: u64,
    ffb_requests: u8,
    mem_control0: u32,
    mem_control1: u32,
    super_io: SuperIoConfig,
    keyboard: KeyboardUart,
    mouse_uart: KeyboardUart,
    serial: Serial,
    elapsed_ns: u64,
    tick_origin: u32,
}

impl Ultra5ResetBoard {
    pub fn new(flash: BootFlash, nvram: ResetNvram) -> Self {
        Self {
            flash,
            nvram,
            dram: Dram::default(),
            pbm: Pbm::default(),
            host_primary: 0,
            host_subordinate: 0,
            apb: Default::default(),
            pci_functions: PciFunction::onboard(),
            ebus_setup: [0; 3],
            disk: None,
            dma_writes: Vec::new(),
            cdrom: None,
            hme: Default::default(),
            display: Default::default(),
            audio: Default::default(),
            floppy_aux: 0,
            ebus_dma: [[0; 3]; 4],
            floppy: Default::default(),
            floppy_last_ns: 0,
            front_panel_led: false,
            reset_cause: POR,
            // IIi tables 18-3 and 18-7: DIMM pairs, refresh interval and DRAM timings.
            ffb_requests: 1,
            mem_control0: 0x0f30,
            mem_control1: 0x004a_2950,
            super_io: SuperIoConfig::default(),
            keyboard: KeyboardUart::default(),
            mouse_uart: KeyboardUart::default(),
            serial: Serial::default(),
            elapsed_ns: 0,
            tick_origin: 0,
        }
    }

    /// Step within flash and report the ROM offset. Use `step_mapped` to
    /// follow firmware as it relocates into RAM.
    pub fn step(&mut self, cpu: &mut V9Cpu) -> Result<FlashFetch, String> {
        let mapping = cpu.instruction_address()?;
        if mapping.cacheable {
            return Err(format!(
                "Ultra 5 flash step reached DRAM at VA={:#x} PA={:#x}",
                cpu.pc(),
                mapping.pa
            ));
        }
        let (pc, npc) = (cpu.pc(), cpu.npc());
        let flash_pa = self.decode_pci_memory(mapping.pa)?;
        let flash_offset = flash_pa
            .checked_sub(FLASH_BASE)
            .filter(|&offset| offset < super::FLASH_SIZE as u64)
            .ok_or_else(|| format!("Ultra 5 instruction outside flash at {flash_pa:#x}"))?
            as usize;
        self.snoop_dma_writes(cpu);
        let fetch = self.step_at(cpu, mapping)?;
        Ok(FlashFetch {
            pc,
            npc,
            physical_address: mapping.pa,
            flash_offset,
            instruction: fetch.instruction,
        })
    }

    pub fn step_mapped(&mut self, cpu: &mut V9Cpu) -> Result<BoardInstruction, String> {
        let mapping = cpu.instruction_address()?;
        self.snoop_dma_writes(cpu);
        Ok(self.step_at(cpu, mapping)?)
    }

    /// Advance through a resident, side-effect-free countdown, up to the caller's
    /// remaining instruction budget. The CPU stops short of its next timer event.
    pub fn repeat_countdown(&mut self, cpu: &mut V9Cpu, budget: u64) -> u64 {
        self.snoop_dma_writes(cpu);
        if self.sample_vector(cpu).is_some() || budget < 2 {
            return 0;
        }
        let Ok(fetch) = cpu.instruction_address() else {
            return 0;
        };
        self.repeat_at(cpu, fetch, budget)
    }

    fn countdown_budget(&self, budget: u64) -> u64 {
        [
            self.serial.next_timeout(),
            Some(self.display.raster_deadline()),
            self.keyboard.next_event(self.elapsed_ns),
            self.mouse_uart.next_event(self.elapsed_ns),
        ]
        .into_iter()
        .flatten()
        .min()
        .map_or(budget, |deadline| {
            budget.min(
                deadline
                    .saturating_sub(self.elapsed_ns)
                    .div_ceil(crate::machine::STEP_NS),
            )
        })
    }

    fn repeat_at(&mut self, cpu: &mut V9Cpu, fetch: Translation, budget: u64) -> u64 {
        let retired = cpu.repeat_mapped_countdown(fetch, || self.countdown_budget(budget));
        if retired != 0 {
            self.elapsed_ns = self
                .elapsed_ns
                .saturating_add(retired.saturating_mul(crate::machine::STEP_NS));
        }
        retired
    }

    /// Service one instruction boundary. Returns one for an ordinary retirement
    /// or trap entry, or the number of retirements in a resident countdown.
    /// A zero budget leaves the CPU and devices untouched.
    pub fn step_or_repeat(&mut self, cpu: &mut V9Cpu, budget: u64) -> Result<u64, String> {
        if budget == 0 {
            return Ok(0);
        }
        self.snoop_dma_writes(cpu);
        if self.take_vector(cpu)?.is_some() {
            return Ok(1);
        }
        let mapping = cpu.instruction_address();
        if let Ok(fetch) = mapping {
            if budget >= 2 {
                let count = self.repeat_at(cpu, fetch, budget);
                if count != 0 {
                    return Ok(count);
                }
            }
        }
        self.step_translated(cpu, mapping).map(|_| 1)
    }

    /// Run one retirement, supported synchronous trap or eligible software interrupt.
    /// The retirement-only step_mapped API stops before trap or interrupt entry.
    pub fn step_outcome(&mut self, cpu: &mut V9Cpu) -> Result<BoardStep, String> {
        if let Some(event) = self.take_vector(cpu)? {
            return Ok(event);
        }
        let mapping = cpu.instruction_address();
        if mapping.is_ok() {
            self.snoop_dma_writes(cpu);
        }
        self.step_translated(cpu, mapping)
    }

    fn sample_vector(&mut self, cpu: &V9Cpu) -> Option<u8> {
        self.sample_interrupts();
        if cpu.can_take_vector() {
            self.pbm.pending_vector()
        } else {
            None
        }
    }

    fn take_vector(&mut self, cpu: &mut V9Cpu) -> Result<Option<BoardStep>, String> {
        let Some(ino) = self.sample_vector(cpu) else {
            return Ok(None);
        };
        let fault_pc = cpu.pc();
        cpu.enter_vector(ino)?;
        self.pbm.sent_vector(ino);
        self.elapsed_ns = self.elapsed_ns.saturating_add(crate::machine::STEP_NS);
        Ok(Some(BoardStep::TrapTaken {
            fault_pc,
            tt: 0x60,
            vector: cpu.pc(),
        }))
    }

    fn step_translated(
        &mut self,
        cpu: &mut V9Cpu,
        mapping: Result<Translation, CpuFault>,
    ) -> Result<BoardStep, String> {
        let fault_pc = cpu.pc();
        let fetch = mapping.as_ref().ok().copied();
        let result = match mapping {
            Ok(fetch) => self.step_at(cpu, fetch),
            Err(fault) => Err(fault),
        };
        let event = match result {
            Ok(step) => return Ok(BoardStep::Retired(step)),
            Err(CpuFault::DataAccessError) => {
                cpu.enter_data_access_error()?;
                BoardStep::TrapTaken {
                    fault_pc,
                    tt: 0x32,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::Synchronous(tt)) => {
                cpu.enter_synchronous(tt, fetch)?;
                BoardStep::TrapTaken {
                    fault_pc,
                    tt,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::Window(tt)) => {
                cpu.enter_window_trap(tt, fetch)?;
                BoardStep::TrapTaken {
                    fault_pc,
                    tt,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::DivisionByZero) => {
                cpu.enter_division_by_zero(fetch)?;
                BoardStep::TrapTaken {
                    fault_pc,
                    tt: 0x28,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::MmuProtection { tag, status, large }) => {
                let tt = cpu.enter_mmu_protection(tag, status, large, fetch)?;
                BoardStep::TrapTaken {
                    fault_pc,
                    tt,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::TlbMiss(miss)) => {
                let tt = cpu.enter_mmu_miss(miss, fetch)?;
                BoardStep::TrapTaken {
                    fault_pc,
                    tt,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::Interrupt(level)) => {
                cpu.enter_interrupt(level)?;
                BoardStep::InterruptTaken {
                    pc: fault_pc,
                    level,
                    vector: cpu.pc(),
                }
            }
            Err(CpuFault::Stop(message)) => return Err(message),
        };
        self.elapsed_ns = self.elapsed_ns.saturating_add(crate::machine::STEP_NS);
        Ok(event)
    }

    fn sample_interrupts(&mut self) {
        self.serial.advance_to(self.elapsed_ns);
        if self.ebus_dma[3][0] & 0x10 != 0 && self.floppy_controller().irq() {
            self.pbm.assert_onboard(0x27);
        }
        if self.display.video_interrupt(self.elapsed_ns) {
            self.pbm.assert_pci(0x0f);
        }
        if self.keyboard.interrupt(self.elapsed_ns) {
            self.pbm.assert_onboard(0x29);
        }
        if self.mouse_uart.interrupt(self.elapsed_ns) {
            self.pbm.assert_onboard(0x2a);
        }
        if self.serial.interrupt() {
            self.pbm.assert_onboard(0x2b);
        }
        let ata = self.disk.as_ref().is_some_and(super::ata::Disk::interrupt);
        let atapi = self
            .cdrom
            .as_ref()
            .is_some_and(super::atapi::Cdrom::interrupt);
        if self.pci_functions[3].ide_interrupts(ata, atapi) {
            self.pbm.assert_onboard(0x20);
        }
    }

    fn step_at(
        &mut self,
        cpu: &mut V9Cpu,
        mapping: Translation,
    ) -> Result<BoardInstruction, CpuFault> {
        let instruction = if mapping.cacheable {
            cpu.retire_ram_instruction(mapping, self)?
        } else {
            let flash_pa = self.decode_pci_memory(mapping.pa)?;
            let word = self.flash.read_word(flash_pa)?;
            cpu.retire_fetch(mapping, word, self)?;
            word
        };
        self.elapsed_ns = self.elapsed_ns.saturating_add(crate::machine::STEP_NS);
        Ok(BoardInstruction {
            physical_address: mapping.pa,
            instruction,
        })
    }

    fn pci_cycles(&self) -> u32 {
        // Ultra 5 primary PCI clock, 66 MHz. Preserve fractional cycles across
        // steps and counter clears; the low 32 bits wrap independently.
        ((self.elapsed_ns / 500) * 33 + (self.elapsed_ns % 500) * 33 / 500) as u32
    }

    fn tick(&self) -> u32 {
        self.pci_cycles().wrapping_sub(self.tick_origin)
    }

    fn clear_tick(&mut self, value: u64) -> Result<(), String> {
        if value != 0 {
            return Err(format!(
                "unsupported nonzero Ultra 5 APB tick write {value:#x}"
            ));
        }
        self.tick_origin = self.pci_cycles();
        Ok(())
    }

    /// SAB 82532 channel A/B configuration, indexed in board address order.
    pub fn serial_ccr0(&self) -> [u8; 2] {
        self.serial.ccr0()
    }

    /// Deliver a console byte on SAB channel A when its receiver has space.
    pub fn receive_serial_a(&mut self, byte: u8) -> bool {
        self.serial.advance_to(self.elapsed_ns);
        self.serial.receive(0, byte, self.elapsed_ns)
    }

    /// Deliver a complete serial BREAK on ttya.
    pub fn receive_serial_break(&mut self) -> bool {
        self.serial.receive_break_a()
    }

    /// Connect a Type 5 keyboard to the onboard Super I/O UART.
    pub fn attach_keyboard(&mut self) {
        self.keyboard.attach(self.elapsed_ns);
        self.mouse_uart.attach_mouse();
    }

    pub fn key(&mut self, code: u8, down: bool) -> bool {
        self.keyboard.key(code, down, self.elapsed_ns)
    }

    pub fn mouse_motion(&mut self, dx: i32, dy: i32) {
        self.mouse_uart
            .mouse_input(self.elapsed_ns, |m| m.motion(dx, dy));
    }
    pub fn mouse_button(&mut self, button: u8, down: bool) {
        self.mouse_uart
            .mouse_input(self.elapsed_ns, |m| m.button(button, down));
    }

    pub fn framebuffer_size(&self) -> Option<(usize, usize)> {
        self.display.size()
    }
    pub fn framebuffer_generation(&self) -> u64 {
        self.display.generation
    }
    pub fn framebuffer_pixels(&self, out: &mut [u32]) {
        self.display.pixels(out)
    }
    pub fn nvram_image(&self) -> Vec<u8> {
        let mut bytes = self.nvram.image.to_vec();
        self.nvram
            .clock
            .save(&mut bytes[super::M48T59_REGISTERS..], self.elapsed_ns);
        bytes
    }

    /// Drain bytes actually transmitted by the guest, channel A followed by B.
    pub fn take_serial_output(&mut self) -> [Vec<u8>; 2] {
        self.serial.take_output()
    }

    /// Whether the host bridge is selected by an incoming PCI single-address
    /// memory cycle. Actual DMA still requires a bus master and IOMMU.
    pub fn selects_pci_memory(&self, address: u32) -> bool {
        self.pbm.selects_pci_memory(address)
    }

    fn floppy_controller(&mut self) -> &mut crate::devices::floppy::Floppy {
        self.floppy
            .advance_ns(self.elapsed_ns - self.floppy_last_ns);
        self.floppy_last_ns = self.elapsed_ns;
        &mut self.floppy
    }

    fn ebus_dma_register(pa: u64) -> Option<(usize, usize)> {
        let off = Self::ebus_offset(pa)?.checked_sub(0x700000)?;
        let sub = off % 0x2000;
        (off < 0x8000 && sub <= 8 && sub % 4 == 0)
            .then_some(((off / 0x2000) as usize, (sub / 4) as usize))
    }

    fn ebus_offset(pa: u64) -> Option<u64> {
        pa.checked_sub(PCI_MEMORY + u64::from(EBUS_BAR1))
            .filter(|&off| off < 0x80_0000)
    }

    // Translate guest-assigned PCI memory BARs into the board peripheral's
    // canonical chip-select address. Other physical address spaces pass through.
    fn decode_pci_memory(&self, pa: u64) -> Result<u64, String> {
        let Some(address) = pa
            .checked_sub(PCI_MEMORY)
            .and_then(|p| u32::try_from(p).ok())
        else {
            return Ok(pa);
        };
        if !self.apb.forwards_onboard_memory(address) {
            return Err(format!(
                "Ultra 5 Simba does not route PCI memory {address:#010x} to onboard bus B"
            ));
        }
        let ebus = &self.pci_functions[0];
        for (bar, size, canonical) in [
            (0, 0x100_0000, FLASH_BASE),
            (1, 0x80_0000, super::RESET_NVRAM_BASE),
        ] {
            if let Some(offset) = address
                .checked_sub(ebus.bar_address(bar))
                .filter(|&v| v < size)
            {
                if !ebus.memory_enabled() {
                    return Err(format!("Ultra 5 PCIO memory decoding disabled at {pa:#x}"));
                }
                return Ok(canonical + u64::from(offset));
            }
        }
        Err(format!("unmapped Ultra 5 PCI memory at {pa:#x}"))
    }

    fn onboard_mem_offset(&self, function: usize, pa: u64, size: u32) -> Option<u32> {
        let addr = u32::try_from(pa.checked_sub(PCI_MEMORY)?).ok()?;
        let device = &self.pci_functions[function];
        if !self.apb.forwards_onboard_memory(addr) || !device.memory_enabled() {
            return None;
        }
        addr.checked_sub(device.bar_address(0))
            .filter(|&off| off < size)
    }

    /// Attach an existing 512-byte-sector disk as primary IDE master (OpenBoot unit 0).
    pub fn attach_disk(
        &mut self,
        path: impl AsRef<std::path::Path>,
        writable: bool,
    ) -> std::io::Result<()> {
        if self.disk.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Ultra 5 IDE disk already attached",
            ));
        }
        self.disk = Some(super::ata::Disk::open(path.as_ref(), writable)?);
        Ok(())
    }

    /// Cumulative media read/write activity for the primary master/slave and
    /// secondary master/slave, in that order. Absent positions read as zero.
    pub fn ide_activity(&self) -> [(u64, u64); 4] {
        [
            self.disk
                .as_ref()
                .map_or((0, 0), super::ata::Disk::activity),
            (0, 0),
            self.cdrom
                .as_ref()
                .map_or((0, 0), super::atapi::Cdrom::activity),
            (0, 0),
        ]
    }

    /// Attach a read-only image at the secondary IDE master (OpenBoot unit 2).
    pub fn attach_cdrom(&mut self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        if self.cdrom.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "CD-ROM already attached",
            ));
        }
        self.cdrom = Some(super::atapi::Cdrom::open(path.as_ref())?);
        Ok(())
    }

    fn onboard_io(&self, pa: u64) -> Option<u32> {
        let a = pa
            .checked_sub(0x01fe_0200_0000)
            .filter(|a| *a < 0x1000000)? as u32;
        if !self.apb.forwards_onboard_io(a) {
            return None;
        }
        Some(a)
    }

    fn ata_port(&self, pa: u64) -> Option<(usize, u8)> {
        self.pci_functions[3].ata_port(self.onboard_io(pa)?)
    }

    fn ata_bus_master(&self, pa: u64) -> Option<usize> {
        self.pci_functions[3].bus_master_offset(self.onboard_io(pa)?)
    }

    fn dma_page(&mut self, va: u32, write: bool) -> Result<u64, String> {
        if !self.pbm.selects_pci_memory(va) {
            return Err(format!("DMA target outside PBM aperture {va:#x}"));
        }
        let pa = self
            .pbm
            .iommu
            .translate(va, write, &self.dram, self.mem_control0)?;
        if pa > u64::from(u32::MAX) {
            return Err(format!("DMA target outside Ultra 5 DRAM {pa:#x}"));
        }
        Ok(pa)
    }

    fn disk_dma_segments(
        &mut self,
        mut prd: u32,
        mut remaining: usize,
        write: bool,
    ) -> Result<Vec<(u64, usize)>, String> {
        let mut result = Vec::new();
        let table_page = prd >> 16;
        while remaining != 0 {
            if prd >> 16 != table_page || prd & 0xffff > 0xfff8 {
                return Err("CMD646 descriptor table crossed 64 KiB".into());
            }
            let mut desc = [0u8; 8];
            for (i, half) in desc.chunks_exact_mut(4).enumerate() {
                let pa = self.dma_page(prd + 4 * i as u32, false)?;
                self.dram.dma_bytes(pa, self.mem_control0, half, false)?;
            }
            let mut address = u32::from_le_bytes(desc[..4].try_into().unwrap());
            let flags = u32::from_le_bytes(desc[4..].try_into().unwrap());
            let length = if flags as u16 == 0 {
                65536
            } else {
                usize::from(flags as u16)
            };
            if address & 1 != 0 || length & 1 != 0 || (address as usize & 65535) + length > 65536 {
                return Err("invalid CMD646 DMA segment".into());
            }
            let mut count = length.min(remaining);
            remaining -= count;
            while count != 0 {
                let piece = count.min(8192 - (address as usize & 8191));
                result.push((self.dma_page(address, write)?, piece));
                count -= piece;
                address = address.wrapping_add(piece as u32);
            }
            if remaining != 0 && flags >> 31 != 0 {
                return Err("short CMD646 descriptor table".into());
            }
            prd = prd.wrapping_add(8);
        }
        Ok(result)
    }

    fn snoop_dma_writes(&mut self, cpu: &mut V9Cpu) {
        if !self.dma_writes.is_empty() {
            for (pa, n) in self.dma_writes.drain(..) {
                cpu.snoop_dma_write(pa, n);
            }
        }
    }

    fn service_disk_dma(&mut self) {
        let Some((read, size)) = self.disk.as_ref().and_then(super::ata::Disk::dma_request) else {
            return;
        };
        let Some((to_ram, prd)) = self.pci_functions[3].dma_command() else {
            return;
        };
        let transfer = || -> Result<(), String> {
            if read != to_ram {
                return Err("CMD646 DMA direction mismatch".into());
            }
            let segments = self.disk_dma_segments(prd, size, to_ram)?;
            let mut bytes = vec![0; size];
            if to_ram && !self.disk.as_mut().unwrap().dma_transfer(&mut bytes) {
                return Err("ATA read error".into());
            }
            let mut offset = 0;
            for (pa, n) in segments {
                self.dram.dma_bytes(
                    pa,
                    self.mem_control0,
                    &mut bytes[offset..offset + n],
                    to_ram,
                )?;
                if to_ram {
                    self.dma_writes.push((pa, n));
                }
                offset += n;
            }
            if !to_ram && !self.disk.as_mut().unwrap().dma_transfer(&mut bytes) {
                return Err("ATA write error".into());
            }
            Ok(())
        };
        let mut transfer = transfer;
        let failed = transfer().is_err();
        if failed {
            self.disk.as_mut().unwrap().dma_abort();
        }
        self.pci_functions[3].dma_finished(failed);
    }
    fn read_config<const N: usize>(&self, pa: u64) -> Option<[u8; N]> {
        self.pbm
            .header
            .read(pa)
            .or_else(|| self.apb.read(pa))
            .or_else(|| {
                let (idx, offset) = self.apb.onboard_config(pa)?;
                self.pci_functions[idx].read(offset)
            })
    }

    fn write_config<const N: usize>(&mut self, pa: u64, bytes: [u8; N]) -> bool {
        if self.pbm.header.write(pa, bytes) || self.apb.write(pa, bytes) {
            return true;
        }
        let Some((idx, offset)) = self.apb.onboard_config(pa) else {
            return false;
        };
        let written = self.pci_functions[idx].write(offset, bytes);
        if written && idx == 3 && (offset..offset + N).contains(&0x70) {
            self.service_disk_dma();
        }
        written
    }

    fn config_byte(&mut self, offset: u64) -> Result<&mut u8, String> {
        match offset {
            0x40 => Ok(&mut self.host_primary),
            0x41 => Ok(&mut self.host_subordinate),
            _ => Err(format!(
                "unmapped Ultra 5 PCI configuration byte {offset:#x}"
            )),
        }
    }
}

// National Super I/O configuration protocol (§2.2).
#[derive(Default)]
struct SuperIoConfig {
    accessed_index: bool,
    index: u8,
    pending: Option<u8>,
    registers: [Option<u8>; 8],
}

impl SuperIoConfig {
    fn keyboard_register(&self, offset: u64) -> Result<u8, String> {
        // FER.1 enables UART1, FAR[3:2]=0 selects COM1, PTR.0/4 select
        // software power-down/test mode. Board reset straps are unverified.
        match self.registers {
            [Some(enable), Some(address), Some(power), ..]
                if enable & 2 != 0 && address & 0x0c == 0 && power & 0x11 == 0 =>
            {
                Ok((offset - KEYBOARD_UART) as u8)
            }
            _ => Err(format!(
                "Ultra 5 keyboard UART unavailable at EBus offset {offset:#x}"
            )),
        }
    }

    fn mouse_register(&self, offset: u64) -> Result<u8, String> {
        match self.registers {
            [Some(enable), Some(address), Some(power), ..]
                if enable & 4 != 0 && address & 0x30 == 0x30 && power & 0x11 == 0 =>
            {
                Ok((offset - MOUSE_UART) as u8)
            }
            _ => Err(format!(
                "Ultra 5 mouse UART unavailable at EBus offset {offset:#x}"
            )),
        }
    }
    fn read_index(&mut self) -> u8 {
        self.pending = None;
        if std::mem::replace(&mut self.accessed_index, true) {
            self.index
        } else {
            0x88
        }
    }

    fn select(&mut self, value: u8) -> Result<(), String> {
        let index = value & 0x8f; // index bits 6:4 read zero
        if !matches!(index, 0..=4 | 6..=7) {
            return Err(format!("unsupported Ultra 5 Super I/O index {index:#x}"));
        }
        self.accessed_index = true;
        self.index = index;
        self.pending = None;
        Ok(())
    }

    fn write_data(&mut self, value: u8) {
        if self.pending == Some(value) {
            self.registers[self.index as usize] = Some(value);
            self.pending = None;
        } else {
            self.pending = Some(value);
        }
    }
}

impl V9DataBus for Ultra5ResetBoard {
    fn read_integer(
        &mut self,
        pa: u64,
        width: crate::v9::IntegerWidth,
        cacheable: bool,
    ) -> Result<u64, String> {
        if !cacheable && (0x01fc_0000_0000..0x01fe_0000_0000).contains(&pa) {
            // IIi §6.2.4: an empty UPA64S slot acknowledges the probe without a trap.
            return Ok(0);
        }
        self.read_physical_integer(pa, width, cacheable)
    }

    fn pci_read_aborted(&mut self, pa: u64) -> bool {
        let Some(offset) = pa.checked_sub(PCI_CONFIG).filter(|&off| off < 1 << 24) else {
            return false;
        };
        let bdf = offset >> 8;
        if matches!(bdf, 0 | 8 | 9) || self.apb.onboard_config(pa).is_some() {
            return false;
        }
        self.pbm.header.master_abort();
        true
    }

    fn read_bypass_half(&mut self, pa: u64) -> Result<u16, String> {
        if let Some(off @ (0x400000..=0x40001f | 0x400040..=0x40005f)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            return self
                .serial
                .read_pool((off - 0x400000) as u8)
                .map(u16::from_be_bytes);
        }
        if self.ata_port(pa) == Some((0, 0)) {
            if let Some(drive) = self.disk.as_mut() {
                return drive.read_data().map(u16::from_be_bytes);
            }
        } else if self.ata_port(pa) == Some((1, 0)) {
            if let Some(drive) = self.cdrom.as_mut() {
                return drive.read_data().map(u16::from_be_bytes);
            }
        }
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .read_at(off, self.elapsed_ns)
                .map(u16::from_be_bytes);
        }
        let pa = self.decode_pci_memory(pa)?;
        if let Some(off @ (0x40003a | 0x40007a)) = Self::ebus_offset(pa) {
            self.serial.advance_to(self.elapsed_ns);
            let reg = (off - 0x400000) as u8;
            return Ok(u16::from_be_bytes([
                self.serial.read(reg)?,
                self.serial.read(reg + 1)?,
            ]));
        }
        if let Some(off) = pa
            .checked_sub(FLASH_BASE)
            .filter(|&off| off < super::FLASH_SIZE as u64 && off % 2 == 0)
        {
            return Ok(u16::from_be_bytes(
                self.flash.image()[off as usize..off as usize + 2]
                    .try_into()
                    .unwrap(),
            ));
        }
        if let Some(bytes) = self.read_config(pa) {
            return Ok(u16::from_be_bytes(bytes));
        }
        Err(format!(
            "unmapped Ultra 5 physical halfword read at {pa:#x}"
        ))
    }
    fn write_bypass_half(&mut self, pa: u64, value: u16) -> Result<(), String> {
        if let Some(off @ (0x400000..=0x40001f | 0x400040..=0x40005f)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            return self
                .serial
                .write_pool((off - 0x400000) as u8, &value.to_be_bytes());
        }
        if self.ata_port(pa) == Some((0, 0)) {
            if let Some(drive) = self.disk.as_mut() {
                return drive.write_data(value.to_be_bytes());
            }
        } else if self.ata_port(pa) == Some((1, 0)) {
            if let Some(drive) = self.cdrom.as_mut() {
                return drive.write_data(value.to_be_bytes());
            }
        }
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .write_at(off, value.to_be_bytes(), self.elapsed_ns);
        }
        if let Some(off @ (0x40003a | 0x40007a)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            let reg = (off - 0x400000) as u8;
            let [imr0, imr1] = value.to_be_bytes();
            self.serial.write(reg, imr0)?;
            return self.serial.write(reg + 1, imr1);
        }
        if self.write_config(pa, value.to_be_bytes()) {
            return Ok(());
        }
        Err(format!(
            "unmapped Ultra 5 physical halfword write at {pa:#x} value={value:#06x}"
        ))
    }
    fn read_cacheable_xword(&mut self, pa: u64) -> Result<u64, String> {
        self.dram.read(pa, self.mem_control0)
    }
    fn write_cacheable_xword(&mut self, pa: u64, value: u64) -> Result<(), String> {
        self.dram.write(pa, self.mem_control0, value)
    }
    fn write_bypass_xword(&mut self, pa: u64, value: u64) -> Result<(), String> {
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .write_at(off, value.to_be_bytes(), self.elapsed_ns);
        }
        if let Some(off @ (0x400000..=0x40001f | 0x400040..=0x40005f)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            return self
                .serial
                .write_pool((off - 0x400000) as u8, &value.to_be_bytes());
        }
        let pa = self.decode_pci_memory(pa)?;
        if self.write_config(pa, value.to_be_bytes()) {
            return Ok(());
        }
        if pa == APB_TICK {
            return self.clear_tick(value.swap_bytes());
        }
        match pa {
            RESET_CONTROL => {
                if value & SOFT_RESET != 0 {
                    return Err(format!(
                        "unsupported IIi software reset request {value:#018x}"
                    ));
                }
                self.reset_cause &= EXTERNAL_RESET & !value;
                return Ok(());
            }
            0x01fe_0000_f000 => {
                self.ffb_requests = ((value >> 24) & 15) as u8;
                return Ok(());
            }
            MEM_CONTROL0 => {
                // SME1430 RefEnable, ECCEnable, board timing, geometry and refresh fields.
                self.mem_control0 = value as u32 & 0x9e00_1fff;
                return Ok(());
            }
            MEM_CONTROL1 => {
                self.mem_control1 = value as u32;
                return Ok(());
            }
            _ => {}
        }
        if let Some(offset) = Pbm::offset(pa) {
            return self.pbm.write(offset, value);
        }
        if matches!(Self::ebus_offset(pa), Some(0..=0x0f_ffff)) {
            let i = self.nvram.sram_index(pa, 8)?;
            self.nvram.image[i..i + 8].copy_from_slice(&value.to_be_bytes());
            return Ok(());
        }
        Err(format!(
            "unmapped Ultra 5 physical xword write at {pa:#x} value={value:#018x}"
        ))
    }
    fn read_bypass_word(&mut self, pa: u64) -> Result<u32, String> {
        if let Some(off @ (0x400000..=0x40001f | 0x400040..=0x40005f)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            return self
                .serial
                .read_pool((off - 0x400000) as u8)
                .map(u32::from_be_bytes);
        }
        if let Some(reg) = self.ata_bus_master(pa) {
            return self.pci_functions[3]
                .read(reg)
                .map(u32::from_be_bytes)
                .ok_or_else(|| "invalid CMD646 bus master register".into());
        }
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .read_at(off, self.elapsed_ns)
                .map(u32::from_be_bytes);
        }
        if let Some(off) = self.onboard_mem_offset(1, pa, 0x8000) {
            return self
                .hme
                .read(off)
                .map(u32::swap_bytes)
                .ok_or_else(|| format!("unsupported HME read +{off:#x}"));
        }
        let pa = self.decode_pci_memory(pa)?;
        if let Some(bytes) = self.read_config(pa) {
            return Ok(u32::from_be_bytes(bytes));
        }
        if (FLASH_BASE..FLASH_BASE + super::FLASH_SIZE as u64).contains(&pa) {
            return self.flash.read_word(pa);
        }
        if let Some((channel, reg)) = Self::ebus_dma_register(pa) {
            return Ok(self.ebus_dma[channel][reg].swap_bytes());
        }
        if pa == APB_TICK {
            return Ok(self.tick().swap_bytes());
        }
        if Self::ebus_offset(pa) == Some(0x720000) {
            return Ok(self.floppy_aux);
        }
        if pa == AUXIO_LED {
            // PCIO bit 0 is in the first byte on the little-endian EBus interface.
            return Ok(u32::from(self.front_panel_led).swap_bytes());
        }
        if matches!(Self::ebus_offset(pa), Some(0..=0x0f_ffff)) {
            let i = self.nvram.sram_index(pa, 4)?;
            return Ok(u32::from_be_bytes(
                self.nvram.image[i..i + 4].try_into().unwrap(),
            ));
        }
        Err(format!("unmapped Ultra 5 physical word read at {pa:#x}"))
    }
    fn write_bypass_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        if let Some(off @ (0x400000..=0x40001f | 0x400040..=0x40005f)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            return self
                .serial
                .write_pool((off - 0x400000) as u8, &value.to_be_bytes());
        }
        if let Some(reg) = self.ata_bus_master(pa) {
            if !matches!(reg, 0x74 | 0x7c) {
                return Err("unsupported wide CMD646 control write".into());
            }
            self.pci_functions[3].write(reg, value.to_be_bytes());
            return Ok(());
        }
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .write_at(off, value.to_be_bytes(), self.elapsed_ns);
        }
        if let Some(off) = self.onboard_mem_offset(1, pa, 0x8000) {
            return self
                .hme
                .write(off, value.swap_bytes())
                .then_some(())
                .ok_or_else(|| {
                    format!(
                        "unsupported HME write +{off:#x} value={:#x}",
                        value.swap_bytes()
                    )
                });
        }
        let pa = self.decode_pci_memory(pa)?;
        if self.write_config(pa, value.to_be_bytes()) {
            return Ok(());
        }
        if let Some((channel, reg)) = Self::ebus_dma_register(pa) {
            let value = value.swap_bytes();
            if reg == 0 && value & 0x80 != 0 {
                self.ebus_dma[channel] = [0; 3];
            } else if reg == 1
                && self.ebus_dma[channel][0] & 0x200 != 0
                && self.ebus_dma[channel][2] != 0
            {
                return Err(format!("unsupported active EBus DMA on channel {channel}"));
            } else {
                self.ebus_dma[channel][reg] = if reg == 0 { value & 0x01dd_2310 } else { value };
            }
            return Ok(());
        }
        if pa == APB_TICK {
            return self.clear_tick(u64::from(value.swap_bytes()));
        }
        if Self::ebus_offset(pa) == Some(0x720000) {
            // PCIO floppy AUXIO density-select output (bit 1), no disk inserted.
            self.floppy_aux = value & 2;
            return Ok(());
        }
        if pa == AUXIO_LED {
            self.front_panel_led = value.swap_bytes() & 1 != 0;
            return Ok(());
        }
        if !matches!(Self::ebus_offset(pa), Some(0..=0x0f_ffff)) {
            return Err(format!(
                "unmapped Ultra 5 physical word write at {pa:#x} value={value:#010x}"
            ));
        }
        let i = self.nvram.sram_index(pa, 4)?;
        self.nvram.image[i..i + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .read_at(off, self.elapsed_ns)
                .map(u64::from_be_bytes);
        }
        if let Some(off @ (0x400000..=0x40001f | 0x400040..=0x40005f)) =
            self.decode_pci_memory(pa).ok().and_then(Self::ebus_offset)
        {
            return self
                .serial
                .read_pool((off - 0x400000) as u8)
                .map(u64::from_be_bytes);
        }
        let pa = self.decode_pci_memory(pa)?;
        if let Some(bytes) = self.read_config(pa) {
            return Ok(u64::from_be_bytes(bytes));
        }
        if let Some(offset) = pa
            .checked_sub(FLASH_BASE)
            .filter(|&o| o < self.flash.image().len() as u64)
        {
            if offset & 7 != 0 {
                return Err(format!("misaligned Ultra 5 flash xword at {pa:#x}"));
            }
            let start = offset as usize;
            return Ok(u64::from_be_bytes(
                self.flash.image()[start..start + 8].try_into().unwrap(),
            ));
        }
        if pa == APB_TICK {
            // OBP reads the low PCI dword with a little-endian LDXA.
            return Ok(u64::from(self.tick()).swap_bytes());
        }
        match pa {
            0x01fe_0000_f000 => return Ok((u64::from(self.ffb_requests) << 24) | (1 << 14)),
            MEM_CONTROL0 => return Ok(u64::from(self.mem_control0)),
            MEM_CONTROL1 => return Ok(u64::from(self.mem_control1)),
            RESET_CONTROL => return Ok(self.reset_cause),
            _ => {}
        }
        if let Some(offset) = Pbm::offset(pa) {
            return self.pbm.read(offset);
        }
        if matches!(Self::ebus_offset(pa), Some(0..=0x0f_ffff)) {
            return self.nvram.read_bypass_xword(pa);
        }
        Err(format!("unmapped Ultra 5 physical xword read at {pa:#x}"))
    }

    fn read_bypass_byte(&mut self, pa: u64) -> Result<u8, String> {
        if let Some(reg) = self.ata_bus_master(pa) {
            return Ok(self.pci_functions[3].read::<1>(reg).unwrap()[0]);
        }
        if let Some((channel, reg)) = self.ata_port(pa) {
            if channel == 0 {
                if let Some(drive) = self.disk.as_mut() {
                    return drive.read(reg);
                }
            } else if channel == 1 {
                if let Some(drive) = self.cdrom.as_mut() {
                    return drive.read(reg);
                }
            }
            return Ok(0xff);
        }
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .read_at(off, self.elapsed_ns)
                .map(u8::from_be_bytes);
        }
        let pa = self.decode_pci_memory(pa)?;
        if let Some([byte]) = self.read_config(pa) {
            return Ok(byte);
        }
        if let Some(offset) = pa.checked_sub(PCI_CONFIG).filter(|&off| off < 1 << 24) {
            return self.config_byte(offset).map(|register| *register);
        }
        if let Some(off) = pa
            .checked_sub(FLASH_BASE)
            .filter(|&off| off < self.flash.image().len() as u64)
        {
            return Ok(self.flash.image()[off as usize]);
        }
        match Self::ebus_offset(pa) {
            Some(off @ (0x200000 | 0x200004 | 0x200008 | 0x20000c)) => self
                .audio
                .read_byte(off - 0x200000)
                .map_err(|e| format!("Ultra 5 CS4231 read: {e:?}")),
            Some(0..=0x0f_ffff) => self.nvram.read_byte(pa, self.elapsed_ns),
            Some(off @ 0x3023f0..=0x3023f7) => self
                .floppy_controller()
                .read(off & 7)
                .map_err(|e| format!("Ultra 5 floppy read +{}: {e:?}", off & 7)),
            Some(SIO_INDEX) => Ok(self.super_io.read_index()),
            Some(SIO_DATA) => {
                let value =
                    self.super_io.registers[self.super_io.index as usize].ok_or_else(|| {
                        format!(
                            "unmodeled Super I/O reset value for index {}",
                            self.super_io.index
                        )
                    })?;
                self.super_io.pending = None;
                Ok(value)
            }
            Some(off) if (MOUSE_UART..MOUSE_UART + 8).contains(&off) => self
                .mouse_uart
                .read(self.super_io.mouse_register(off)?, self.elapsed_ns),
            Some(off) if (KEYBOARD_UART..KEYBOARD_UART + 8).contains(&off) => self
                .keyboard
                .read(self.super_io.keyboard_register(off)?, self.elapsed_ns),
            Some(off @ 0x40_0000..=0x40_007f) => {
                self.serial.advance_to(self.elapsed_ns);
                self.serial.read((off - 0x40_0000) as u8)
            }
            _ => Err(format!("unmapped Ultra 5 physical byte read at {pa:#x}")),
        }
    }

    fn write_bypass_byte(&mut self, pa: u64, value: u8) -> Result<(), String> {
        if let Some(reg) = self.ata_bus_master(pa) {
            if reg == 0x78 && value & 1 != 0 {
                return Err("unsupported secondary CMD646 DMA".into());
            }
            self.pci_functions[3].write(reg, [value]);
            if reg == 0x70 {
                self.service_disk_dma();
            }
            return Ok(());
        }
        if let Some((channel, reg)) = self.ata_port(pa) {
            if channel == 0 {
                if let Some(drive) = self.disk.as_mut() {
                    drive.write(reg, value)?;
                    if reg == 7 {
                        self.service_disk_dma();
                    }
                    return Ok(());
                }
            } else if channel == 1 {
                if let Some(drive) = self.cdrom.as_mut() {
                    return drive.write(reg, value);
                }
            }
            return Ok(());
        }
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self.display.write_at(off, [value], self.elapsed_ns);
        }
        let pa = self.decode_pci_memory(pa)?;
        if self.write_config(pa, [value]) {
            return Ok(());
        }
        if let Some(offset) = pa.checked_sub(PCI_CONFIG).filter(|&off| off < 1 << 24) {
            *self.config_byte(offset)? = value;
            return Ok(());
        }
        match Self::ebus_offset(pa) {
            Some(0..=0x0f_ffff) => self.nvram.write_byte(pa, value, self.elapsed_ns)?,
            Some(off @ 0x3023f0..=0x3023f7) => self
                .floppy_controller()
                .write(off & 7, value)
                .map_err(|e| {
                    format!("Ultra 5 floppy write +{} value={value:#x}: {e:?}", off & 7)
                })?,
            Some(off @ (0x200000 | 0x200004 | 0x200008 | 0x20000c)) => self
                .audio
                .write_byte(off - 0x200000, value)
                .map_err(|e| format!("Ultra 5 CS4231 write: {e:?}"))?,
            Some(SIO_INDEX) => self.super_io.select(value)?,
            Some(SIO_DATA) => self.super_io.write_data(value),
            Some(off) if (MOUSE_UART..MOUSE_UART + 8).contains(&off) => {
                let reg = self.super_io.mouse_register(off)?;
                let normal_clock = self.super_io.registers[7].is_some_and(|t| t & 0x12 == 0);
                self.mouse_uart
                    .write(reg, value, self.elapsed_ns, normal_clock)?;
            }
            Some(off) if (KEYBOARD_UART..KEYBOARD_UART + 8).contains(&off) => {
                let reg = self.super_io.keyboard_register(off)?;
                // TUP[0,3] select the oscillator and UART1 prescaler.
                let normal_clock = self.super_io.registers[7].is_some_and(|tup| tup & 9 == 0);
                self.keyboard
                    .write(reg, value, self.elapsed_ns, normal_clock)?;
            }
            Some(off @ 0x40_0000..=0x40_007f) => {
                self.serial.write((off - 0x40_0000) as u8, value)?
            }
            _ => {
                return Err(format!(
                    "unmapped Ultra 5 physical byte write at {pa:#x} value={value:#04x}"
                ))
            }
        }
        Ok(())
    }

    fn write_bypass_le_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        if let Some(off) = self.onboard_mem_offset(2, pa, 0x1000000) {
            return self
                .display
                .write_at(off, value.to_le_bytes(), self.elapsed_ns);
        }
        if self.onboard_mem_offset(1, pa, 0x8000).is_some() {
            return self.write_bypass_word(pa, value.swap_bytes());
        }

        let pa = self.decode_pci_memory(pa)?;
        if self.write_config(pa, value.to_le_bytes()) {
            return Ok(());
        }
        if pa == APB_TICK {
            return self.clear_tick(u64::from(value));
        }
        if pa == AUXIO_LED {
            self.front_panel_led = value & 1 != 0;
            return Ok(());
        }
        match Self::ebus_offset(pa) {
            Some(off @ (0x71_0000 | 0x71_0004 | 0x71_0008)) => {
                self.ebus_setup[((off - 0x71_0000) / 4) as usize] = value;
                Ok(())
            }
            _ => Err(format!(
                "unmapped Ultra 5 physical LE word write at {pa:#x} value={value:#x}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ultra5::{FLASH_SIZE, RESET_NVRAM_SIZE};

    fn fixture() -> Ultra5ResetBoard {
        Ultra5ResetBoard::new(
            BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
            ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
        )
    }

    #[test]
    fn empty_ide_task_file_follows_the_enabled_simba_io_window_and_native_bars() {
        let mut board = fixture();
        let ide = PCI_CONFIG + 0x11800;
        let addr = 0x01fe_02c0_0000;
        board.write_bypass_byte(PCI_CONFIG + 0x919, 1).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 1).unwrap();
        board.write_bypass_byte(ide + 4, 1).unwrap();
        board.write_bypass_byte(ide + 9, 0x8f).unwrap();
        board.write_bypass_le_word(ide + 0x10, 0xc00001).unwrap();
        board.write_bypass_le_word(ide + 0x14, 0xc00009).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x904, 3).unwrap();
        assert!(board.read_bypass_byte(addr + 7).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x9de, 0x40).unwrap();
        assert_eq!(board.read_bypass_byte(addr + 7).unwrap(), 0xff);
        board.write_bypass_byte(addr + 7, 0xec).unwrap(); // IDENTIFY with no drive
        assert_eq!(board.read_bypass_byte(addr + 10).unwrap(), 0xff);
        assert!(board.read_bypass_byte(addr + 9).is_err());
        board.write_bypass_le_word(ide + 0x10, u32::MAX).unwrap();
        assert_eq!(
            board.read_bypass_word(ide + 0x10).unwrap().swap_bytes(),
            0xffff_fff9
        );
        assert!(board.read_bypass_byte(addr + 7).is_err());
        board.write_bypass_byte(ide + 4, 0).unwrap();
        assert!(board.read_bypass_byte(addr + 10).is_err());
    }

    #[test]
    fn empty_upa_and_absent_pci_config_have_distinct_responses() {
        let mut board = fixture();
        use crate::v9::IntegerWidth::Word;
        assert_eq!(
            board.read_integer(0x01fc_0000_0100, Word, false).unwrap(),
            0
        );
        assert!(!board.pci_read_aborted(0x01fc_0000_0100));
        assert!(board.pci_read_aborted(PCI_CONFIG + 0x08000));
        assert_ne!(board.read_bypass_byte(PCI_CONFIG + 7).unwrap() & 0x20, 0);
        board.write_bypass_byte(PCI_CONFIG + 7, 0x20).unwrap();
        assert_eq!(board.read_bypass_byte(PCI_CONFIG + 7).unwrap() & 0x20, 0);
        assert!(!board.pci_read_aborted(PCI_CONFIG + 0x8a1));
        assert!(board.read_bypass_word(PCI_CONFIG + 0x8a0).is_err());
    }

    #[test]
    fn iii_reset_control_acknowledges_only_the_written_cause() {
        let mut board = fixture();
        assert_eq!(board.read_bypass_xword(RESET_CONTROL).unwrap(), 0x8000_0000);
        for cause in [0x8000_0000, 0x1000_0000, 0x0800_0000] {
            // Only one external reset input wins, as specified by table 17-2.
            board.reset_cause = cause;
            for keep in [0, !cause & !0x6000_0000] {
                board.write_bypass_xword(RESET_CONTROL, keep).unwrap();
                assert_eq!(board.read_bypass_xword(RESET_CONTROL).unwrap(), cause);
            }
            for request in [0x4000_0000, 0x2000_0000, 0x6000_0000] {
                assert!(board
                    .write_bypass_xword(RESET_CONTROL, request | cause)
                    .is_err());
                assert_eq!(board.read_bypass_xword(RESET_CONTROL).unwrap(), cause);
            }
            board.write_bypass_xword(RESET_CONTROL, cause).unwrap();
            board.write_bypass_xword(RESET_CONTROL, cause).unwrap();
            assert_eq!(board.read_bypass_xword(RESET_CONTROL).unwrap(), 0);
        }
    }

    #[test]
    fn iii_reset_control_requires_an_aligned_uncached_xword() {
        let mut board = fixture();
        assert!(board.write_bypass_xword(RESET_CONTROL + 4, POR).is_err());
        assert!(board.write_cacheable_xword(RESET_CONTROL, POR).is_err());
        assert!(board
            .write_bypass_word(RESET_CONTROL + 4, POR as u32)
            .is_err());
        assert!(board.write_bypass_half(RESET_CONTROL, 0xffff).is_err());
        assert!(board.write_bypass_byte(RESET_CONTROL, 0xff).is_err());
        assert!(board.read_bypass_word(RESET_CONTROL).is_err());
        assert!(board.read_cacheable_xword(RESET_CONTROL).is_err());
        assert_eq!(board.read_bypass_xword(RESET_CONTROL).unwrap(), 0x8000_0000);
    }

    #[test]
    fn pci_header_byte_order_and_widths_share_one_host_function() {
        let mut board = fixture();
        assert_eq!(board.read_bypass_word(PCI_CONFIG).unwrap(), 0x8e10_00a0);
        assert_eq!(
            board.read_bypass_xword(PCI_CONFIG + 8).unwrap(),
            0x0000_0006_0000_0000
        );
        assert_eq!(board.read_bypass_half(PCI_CONFIG + 4).unwrap(), 0x0600);
        board.write_bypass_half(PCI_CONFIG + 4, 0x4601).unwrap();
        assert_eq!(board.read_bypass_byte(PCI_CONFIG + 4).unwrap(), 0x46);
        assert_eq!(board.read_bypass_byte(PCI_CONFIG + 5).unwrap(), 1);
        board.write_bypass_byte(PCI_CONFIG + 0xd, 0x2f).unwrap();
        assert_eq!(
            board.read_bypass_word(PCI_CONFIG + 0xc).unwrap(),
            0x0028_0000
        );
        board
            .write_bypass_le_word(PCI_CONFIG + 4, 0xffff_0000)
            .unwrap();
        assert_eq!(board.read_bypass_word(PCI_CONFIG + 4).unwrap(), 0x0600_a002);
        assert!(board.write_bypass_half(PCI_CONFIG + 5, 0xffff).is_err());
        assert!(board.read_bypass_half(PCI_CONFIG + (1 << 16) + 4).is_err());
        assert_eq!(board.read_bypass_half(PCI_CONFIG + 4).unwrap(), 0x0600);
    }

    #[test]
    fn pci_control_accepts_only_the_aligned_uncached_xword() {
        let mut board = fixture();
        let control = 0x01fe_0000_2000;
        board.write_bypass_xword(control, 0x14_0020_010f).unwrap();
        assert!(board.write_cacheable_xword(control, 0).is_err());
        assert!(board.write_bypass_xword(control + 4, 0).is_err());
        assert!(board.write_bypass_xword(control + 0x10000, 0).is_err());
        assert!(board.write_bypass_word(control, 0).is_err());
        assert!(board.write_bypass_byte(control, 0).is_err());
        assert!(board.read_bypass_word(control).is_err());
        assert!(board.read_cacheable_xword(control).is_err());
        assert_eq!(board.read_bypass_xword(control).unwrap(), 0x10_0020_010f);
    }

    #[test]
    fn iommu_registers_accept_only_aligned_uncached_xwords() {
        let mut board = fixture();
        let control = 0x01fe_0000_0200;
        board.write_bypass_xword(control, 2).unwrap();
        for (pa, value) in [
            (control, 2),
            (control + 8, 0x1234_6000),
            (0x01fe_0000_a5f8, 0x102),
            (0x01fe_0000_a678, 0x5000_0001),
        ] {
            board.write_bypass_xword(pa, value).unwrap();
            assert!(board.write_bypass_byte(pa, 0).is_err());
            assert!(board.write_bypass_half(pa, 0).is_err());
            assert!(board.write_bypass_word(pa, 0).is_err());
            assert!(board.write_bypass_le_word(pa, 0).is_err());
            assert!(board.write_bypass_xword(pa + 1, 0).is_err());
            assert!(board.write_cacheable_xword(pa, 0).is_err());
            assert!(board.read_bypass_byte(pa).is_err());
            assert!(board.read_bypass_half(pa).is_err());
            assert!(board.read_bypass_word(pa).is_err());
            assert!(board.read_cacheable_xword(pa).is_err());
            assert_eq!(board.read_bypass_xword(pa).unwrap(), value);
        }
    }

    #[test]
    fn cacheable_xwords_use_dram_and_leave_io_registers_alone() {
        let mut board = fixture();
        let ram = 0x1000_0000;
        board.write_cacheable_xword(ram, 0x1234).unwrap();
        assert!(board.write_bypass_xword(ram, 17).is_err());
        assert!(board.read_bypass_xword(ram).is_err());
        for invalid in [ram + 4, MEM_CONTROL0, FLASH_BASE] {
            assert!(board.write_cacheable_xword(invalid, 0xffff).is_err());
            assert!(board.read_cacheable_xword(invalid).is_err());
        }
        assert_eq!(board.read_bypass_xword(MEM_CONTROL0).unwrap(), 0xf30);
        assert_eq!(board.read_cacheable_xword(ram).unwrap(), 0x1234);
    }

    #[test]
    #[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
    fn original_obp_sizes_the_dimm_pair_restores_patterns_and_clears_its_workspace() {
        let image = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap();
        let mut nvram = vec![0; RESET_NVRAM_SIZE];
        nvram[1] = 1;
        nvram[0x10..0x18].copy_from_slice(b"soft-rst");
        let mut board = Ultra5ResetBoard::new(
            BootFlash::new(image).unwrap(),
            ResetNvram::new(nvram).unwrap(),
        );
        let mut cpu = V9Cpu::from(crate::ultra5::PowerOnReset::ultra_sparc_ii());
        let mut sentinels = Vec::new();
        for rank in [0x1000_0000, 0x3000_0000] {
            for lane in [0, 8, 16] {
                for offset in [0, 8 << 20, 16 << 20, 32 << 20, 64 << 20] {
                    let pa = rank + lane + offset;
                    let value = 0xfeed_abcd_0000_0000 | pa;
                    board.dram.write(pa, 0, value).unwrap();
                    sentinels.push((pa, 0, value));
                }
                let pa = rank + lane + 0x4000;
                board.dram.write(pa, 0x1000, !pa).unwrap();
                sentinels.push((pa, 0x1000, !pa));
            }
        }
        // Size and location follow from the two 256 MiB DIMMs on the
        // reference board. OBP puts its workspace at the end of the upper rank.
        let scratch = 0x3ff0_0000;
        let mut seen = [false; 3];
        for _ in 0..650_000 {
            match cpu.pc() & 0xf_ffff {
                0x126c => {
                    assert_eq!((cpu.read_reg(8), cpu.read_reg(9)), (0x1a0, 1));
                    for &(pa, mode, value) in &sentinels {
                        assert_eq!(board.dram.read(pa, mode).unwrap(), value, "at {pa:#x}");
                    }
                    seen[0] = true;
                }
                0x1444 => {
                    assert_eq!(
                        (cpu.read_reg(8), cpu.read_reg(9)),
                        (0x3000_0000, 0x1000_0000)
                    );
                    assert_ne!(board.mem_control0 & 0x1000, 0);
                    for addr in [scratch - 8, scratch, scratch + 0x80000, 0x3fff_fff8] {
                        board.write_cacheable_xword(addr, !addr).unwrap();
                    }
                    seen[1] = true;
                }
                0x1460 => {
                    assert_eq!(
                        board.read_cacheable_xword(scratch - 8).unwrap(),
                        !(scratch - 8)
                    );
                    for addr in [scratch, scratch + 0x80000, 0x3fff_fff8] {
                        assert_eq!(board.read_cacheable_xword(addr).unwrap(), 0);
                    }
                    seen[2] = true;
                }
                0x1f90 => break,
                _ => {}
            }
            board.step(&mut cpu).unwrap();
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(cpu.pc(), 0x01ff_f000_1f90);
        assert_eq!(cpu.read_reg(8), 0xc);
        assert_eq!(board.step(&mut cpu).unwrap().instruction, 0xd0f0_08a0);
        assert_eq!(cpu.pc(), 0x01ff_f000_1f94);
        let [ttya, ttyb] = board.take_serial_output();
        assert!(ttya.ends_with(b"Probing Memory Done\r\nMEM BASE = 0000.0000.3000.0000\r\nMEM SIZE = 0000.0000.1000.0000\r\n11-Column Mode Enabled \r\nMMUs "));
        assert!(ttyb.is_empty());
    }

    #[test]
    fn apb_tick_wraps_on_pci_clock_edges_and_reads_do_not_advance_it() {
        let mut board = fixture();
        let tick = 0x01fe_0100_08b0;
        board.elapsed_ns = 15;
        assert_eq!(board.read_bypass_xword(tick).unwrap(), 0);
        board.write_bypass_le_word(tick, 0).unwrap();
        board.elapsed_ns = 16;
        assert_eq!(board.read_bypass_xword(tick).unwrap(), 1u64.swap_bytes());
        assert_eq!(board.read_bypass_word(tick).unwrap(), 1u32.swap_bytes());
        assert!(board.write_bypass_xword(tick, 1).is_err());
        assert!(board.read_bypass_xword(tick + 0x100).is_err());
        assert!(board.write_bypass_word(tick + 4, 0).is_err());
        assert_eq!(board.tick(), 1);
        board.elapsed_ns = 65_075_262_050;
        assert_eq!(board.tick(), u32::MAX);
        board.elapsed_ns += 15;
        assert_eq!(board.tick(), 0);
        board.elapsed_ns += 500;
        assert_eq!(board.tick(), 33);
        board.write_bypass_xword(tick, 0).unwrap();
        assert_eq!(board.tick(), 0);
        board.write_bypass_byte(PCI_CONFIG + 0x40, 2).unwrap();
        assert_eq!(board.read_bypass_word(tick).unwrap(), 0);
        assert!(board.read_bypass_word(tick + 0x20000).is_err());
        let reset = crate::ultra5::PowerOnReset::ultra_sparc_ii();
        let now = board.elapsed_ns;
        let mut cpu = V9Cpu::from(reset);
        assert!(board.step(&mut cpu).is_err()); // zero flash cannot retire
        assert_eq!(board.elapsed_ns, now);
    }

    #[test]
    fn keyboard_uart_decodes_only_enabled_com1_and_preserves_divisor_when_disabled() {
        let mut board = fixture();
        let port = 0x01ff_f130_83f8;
        let index = 0x01ff_f130_0398;
        let configure = |board: &mut Ultra5ResetBoard, reg, value| {
            board.write_bypass_byte(index, reg).unwrap();
            board.write_bypass_byte(index + 1, value).unwrap();
            board.write_bypass_byte(index + 1, value).unwrap();
        };
        assert!(board.read_bypass_byte(port + 5).is_err());
        configure(&mut board, 0, 0x0f);
        configure(&mut board, 1, 0xb0);
        configure(&mut board, 2, 0x80);
        board.write_bypass_byte(port + 3, 0x83).unwrap();
        board.write_bypass_byte(port, 0x5a).unwrap();
        board.write_bypass_byte(port + 1, 0xa5).unwrap();
        assert!(board.write_bypass_word(port, 0).is_err());
        assert!(board.read_bypass_byte(port + 8).is_err());
        assert_eq!(board.read_bypass_byte(0x01ff_f130_62fd).unwrap(), 0x60); // independent transmitter
        for (register, invalid, valid) in [
            (0, 0x0d, 0x0f),
            (1, 0xb4, 0xb0),
            (2, 0x81, 0x80),
            (2, 0x90, 0x80),
        ] {
            configure(&mut board, register, invalid);
            assert!(board.write_bypass_byte(port, 0).is_err());
            assert!(board.read_bypass_byte(port + 5).is_err());
            configure(&mut board, register, valid);
            assert_eq!(board.read_bypass_byte(port).unwrap(), 0x5a);
            assert_eq!(board.read_bypass_byte(port + 1).unwrap(), 0xa5);
        }
        board.write_bypass_byte(port + 3, 3).unwrap();
        assert_eq!(board.read_bypass_byte(port + 1).unwrap(), 0); // IER, not DLM
        assert_eq!(board.read_bypass_byte(port + 5).unwrap(), 0x60);
        assert_eq!(board.read_bypass_byte(port + 2).unwrap(), 1);
        board.write_bypass_byte(port + 2, 7).unwrap();
        assert_eq!(board.read_bypass_byte(port + 2).unwrap(), 0xc1);
        board.write_bypass_byte(port + 1, 1).unwrap();
        assert_eq!(board.read_bypass_byte(port + 1).unwrap(), 1);
        board.write_bypass_byte(port + 4, 3).unwrap();
        assert_eq!(board.read_bypass_byte(port + 4).unwrap(), 3);
        assert!(board.write_bypass_byte(port, 1).is_err()); // unprogrammed clock
        configure(&mut board, 7, 0);
        board.write_bypass_byte(port, 1).unwrap(); // THR, not DLL
        assert_eq!(board.read_bypass_byte(port + 5).unwrap(), 0x20);
        board.write_bypass_byte(port + 3, 0x83).unwrap();
        assert_eq!(board.read_bypass_byte(port).unwrap(), 0x5a);
        assert_eq!(board.take_serial_output(), [vec![], vec![]]);
    }

    #[test]
    #[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
    fn original_obp_programs_the_headless_keyboard_uart_before_starting_its_timeout() {
        let flash = BootFlash::new(
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
            ))
            .unwrap(),
        )
        .unwrap();
        let uart = 0x01ff_f130_83f8;
        for diagnostics in [false, true] {
            let mut seed = vec![0; RESET_NVRAM_SIZE];
            seed[1] = u8::from(diagnostics);
            let mut board = Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(seed).unwrap());
            let reset = crate::ultra5::PowerOnReset::ultra_sparc_ii();
            let base = reset.pc - 0x20;
            let mut cpu = V9Cpu::from(reset);
            let mut writes = Vec::new();
            for _ in 0..180_000 {
                let offset = cpu.pc() - base;
                if offset == 0x630 {
                    break;
                }
                if matches!(
                    offset,
                    0x554 | 0x560 | 0x56c | 0x578 | 0x584 | 0x590 | 0x59c | 0x5a8 | 0x5b4 | 0x5c0
                ) {
                    writes.push((cpu.read_reg(11) - uart, cpu.read_reg(12)));
                }
                board.step(&mut cpu).unwrap();
            }
            assert_eq!((cpu.pc(), cpu.npc()), (base + 0x630, base + 0x634));
            assert_eq!(
                (cpu.read_reg(8), cpu.read_reg(9), cpu.read_reg(11)),
                (uart, 0x01fe_0100_08b0, 0x03f9_41f8)
            );
            assert_eq!(
                writes,
                [
                    (1, 0),
                    (4, 0),
                    (2, 0),
                    (2, 6),
                    (2, 1),
                    (3, 0x83),
                    (0, 0x60),
                    (1, 0),
                    (3, 3),
                    (4, 0)
                ]
            );
            assert_eq!(board.read_bypass_byte(uart + 3).unwrap(), 3);
            assert_eq!(board.read_bypass_byte(uart + 4).unwrap(), 0);
            assert_eq!(board.read_bypass_byte(uart + 5).unwrap(), 0x60);
            assert_eq!(board.read_bypass_byte(uart + 2).unwrap(), 0xc1);
            board.write_bypass_byte(uart + 3, 0x83).unwrap();
            assert_eq!(board.read_bypass_byte(uart).unwrap(), 96);
            assert_eq!(board.read_bypass_byte(uart + 1).unwrap(), 0);
            let [ttya, ttyb] = board.take_serial_output();
            assert!(ttyb.is_empty());
            if diagnostics {
                assert!(ttya.ends_with(b"\r\nProbing keyboard "));
            } else {
                assert!(ttya.is_empty());
            }
        }
    }

    #[test]
    fn ebus_audio_exposes_the_four_byte_codec_ports_at_word_spacing() {
        let mut b = fixture();
        let a = 0x01ff_f120_0000;
        b.write_bypass_byte(a, 12).unwrap();
        assert_eq!(b.read_bypass_byte(a + 4).unwrap(), 0x8a);
        b.write_bypass_byte(a + 4, 0x40).unwrap();
        assert_eq!(b.read_bypass_byte(a + 4).unwrap(), 0xca);
        b.write_bypass_byte(a, 6).unwrap();
        b.write_bypass_byte(a + 4, 0x3a).unwrap();
        assert_eq!(b.read_bypass_byte(a + 4).unwrap(), 0x3a);
        assert!(b.read_bypass_byte(a + 1).is_err());
        assert!(b.read_bypass_byte(a + 16).is_err());
    }

    #[test]
    fn auxio_led_obeys_pci_byte_order_and_only_decodes_its_word() {
        let mut board = fixture();
        let led = 0x01ff_f172_6000;
        assert_eq!(board.read_bypass_word(led).unwrap(), 0);
        board.write_bypass_word(led, 0x0100_0000).unwrap();
        assert!(board.front_panel_led);
        assert_eq!(board.read_bypass_word(led).unwrap(), 0x0100_0000);
        board.write_bypass_word(led, 1).unwrap();
        assert!(!board.front_panel_led);
        board.write_bypass_le_word(led, 0xfefe_fe01).unwrap();
        assert!(board.front_panel_led);
        assert!(board.write_bypass_byte(led, 0).is_err());
        assert!(board.write_bypass_xword(led, 0).is_err());
        assert!(board.write_bypass_word(led + 4, 0).is_err());
        assert!(board.write_bypass_word(led - 0x2000, 0).is_err());
        assert!(board.front_panel_led);
        let mut cpu = V9Cpu::from(crate::ultra5::PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(1, led);
        cpu.execute_fetched(0xc480_43a0, &mut board).unwrap(); // lduwa [%g1] ASI 0x1d,%g2
        assert_eq!(cpu.read_reg(2), 1);
        cpu.execute_fetched(0xc480_42a0, &mut board).unwrap(); // lduwa [%g1] ASI 0x15,%g2
        assert_eq!(cpu.read_reg(2), 0x0100_0000);
    }

    #[test]
    fn mcu_registers_retain_writable_fields_independently() {
        let mut board = fixture();
        let config = 0x01fe_0000_f010;
        let timing = config + 8;
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0xf30);
        assert_eq!(board.read_bypass_xword(timing).unwrap(), 0x4a2950);
        board.write_bypass_xword(timing, u64::MAX).unwrap();
        assert_eq!(
            board.read_bypass_xword(timing).unwrap(),
            u64::from(u32::MAX)
        );
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0xf30);
        board.write_bypass_xword(config, u64::MAX).unwrap();
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0x9e00_1fff);
        assert!(board.write_bypass_word(config, 0).is_err());
        assert!(board.write_bypass_xword(config + 4, 0).is_err());
        board.write_bypass_xword(timing, 0).unwrap();
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0x9e00_1fff);
        assert_eq!(board.read_bypass_xword(timing).unwrap(), 0);
    }

    #[test]
    #[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
    fn original_obp_sets_refresh_and_returns_to_the_ecache_probe() {
        let image = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap();
        let flash = BootFlash::new(image).unwrap();
        for marker in [[0; 8], *b"soft-rst"] {
            for diagnostics in [false, true] {
                let mut seed = vec![0; RESET_NVRAM_SIZE];
                seed[1] = u8::from(diagnostics);
                seed[0x10..0x18].copy_from_slice(&marker);
                let mut board =
                    Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(seed).unwrap());
                let reset = crate::ultra5::PowerOnReset::ultra_sparc_ii();
                let base = reset.pc - 0x20;
                let mut cpu = V9Cpu::from(reset);
                let mut polls = [0; 2];
                let mut writes = Vec::new();
                let mut retired = 0;
                while cpu.pc() != base + 0x714 && retired < 8_000 {
                    let pc = cpu.pc();
                    if pc == base + 0x14c0 || pc == base + 0x14f8 {
                        assert_eq!(cpu.read_reg(2), 0x01ff_f172_6000);
                        assert!(board.front_panel_led);
                        polls[usize::from(pc == base + 0x14f8)] += 1;
                    }
                    if pc == base + 0xfc && cpu.current_window() == 1 {
                        writes.push((cpu.read_reg(10) + cpu.read_reg(8), cpu.read_reg(9)));
                    }
                    board.step(&mut cpu).unwrap();
                    retired += 1;
                    if pc == base + 0x14fc {
                        assert_eq!(
                            (cpu.current_window(), cpu.pc(), cpu.npc()),
                            (0, base + 0x1500, base + 0x1908)
                        );
                    }
                }
                assert_eq!(
                    (cpu.pc(), cpu.npc(), cpu.read_reg(9)),
                    (base + 0x714, base + 0x718, 1 << 39)
                );
                assert_eq!(
                    writes,
                    [
                        (0x01fe_0000_f018, 0x0c4a_ab14),
                        (0x01fe_0000_f010, 0x8000_053d)
                    ]
                );
                assert_eq!(
                    (board.mem_control0, board.mem_control1),
                    (0x8000_053d, 0x0c4a_ab14)
                );
                assert_eq!(polls, [1_000, 1_000]);
                let [ttya, ttyb] = board.take_serial_output();
                let line: &[u8] = match (diagnostics, marker == *b"soft-rst") {
                    (false, _) => b"",
                    (true, true) => b"Software Power ON\r\n",
                    (true, false) => b"Hardware Power ON\r\n",
                };
                assert_eq!(ttya, line);
                assert!(ttyb.is_empty());
                println!(
                    "marker={marker:?} diagnostics={diagnostics} retired={retired} polls={polls:?}"
                );
            }
        }
    }

    #[test]
    fn boot_flash_and_ebus_accesses_follow_onboard_memory_forwarding() {
        let mut image = vec![0; FLASH_SIZE];
        image[0x20..0x24].copy_from_slice(&0x0100_0000u32.to_be_bytes()); // nop
        let mut board = Ultra5ResetBoard::new(
            BootFlash::new(image).unwrap(),
            ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
        );
        let mut cpu = V9Cpu::from(crate::ultra5::PowerOnReset::ultra_sparc_ii());
        let sab_ccr0 = PCI_MEMORY + u64::from(EBUS_BAR1) + 0x40_002c;
        board.write_bypass_le_word(AUXIO_LED, 1).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x919, 1).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 1).unwrap();
        for (offset, disabled, enabled, reason) in [
            (0x904, 0, 2, "onboard bus B"),
            (0x9df, 0x40, 0x80, "onboard bus B"),
            (0x10804, 4, 6, "PCIO memory decoding disabled"),
        ] {
            board
                .write_bypass_byte(PCI_CONFIG + offset, disabled)
                .unwrap();
            let old_cpu = cpu.clone();
            assert!(board.step(&mut cpu).unwrap_err().contains(reason));
            assert_eq!(cpu, old_cpu);
            assert!(board.read_bypass_byte(FLASH_BASE + 0x20).is_err());
            assert!(board.read_bypass_xword(FLASH_BASE + 0x20).is_err());
            assert!(board.read_bypass_word(FLASH_BASE + 0x20).is_err());
            assert!(board.read_bypass_word(AUXIO_LED).is_err());
            assert!(board.write_bypass_le_word(AUXIO_LED, 0).is_err());
            assert!(board.write_bypass_word(AUXIO_LED, 0).is_err());
            assert!(board.write_bypass_byte(sab_ccr0, 0xc3).is_err());
            assert!(board
                .write_bypass_xword(crate::ultra5::RESET_NVRAM_BASE, 1)
                .is_err());
            board
                .write_bypass_byte(PCI_CONFIG + offset, enabled)
                .unwrap();
            assert_eq!(
                board.read_bypass_word(AUXIO_LED).unwrap(),
                1u32.swap_bytes()
            );
            assert_eq!(
                board
                    .read_bypass_xword(crate::ultra5::RESET_NVRAM_BASE)
                    .unwrap(),
                0
            );
        }
        assert_eq!(board.step(&mut cpu).unwrap().instruction, 0x0100_0000);
        board.write_bypass_byte(sab_ccr0, 0xc3).unwrap();
        assert_eq!(board.serial_ccr0()[0], 0xc3);
    }

    #[test]
    fn flash_data_word_reads_include_both_headers_and_the_last_word() {
        let mut bytes = vec![0; FLASH_SIZE];
        for (offset, word) in [
            (0, *b"OBMD"),
            (0x80000, *b"POST"),
            (FLASH_SIZE - 4, [0x80, 0x21, 0x76, 0xfa]),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&word);
        }
        let mut board = Ultra5ResetBoard::new(
            BootFlash::new(bytes).unwrap(),
            ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
        );
        for (offset, word) in [
            (0, 0x4f42_4d44),
            (0x80000, 0x504f_5354),
            (0xffffc, 0x8021_76fa),
        ] {
            let pa = FLASH_BASE + offset;
            assert!(board.write_bypass_word(pa, 0).is_err());
            assert_eq!(board.read_bypass_word(pa).unwrap(), word);
        }
        for pa in [
            FLASH_BASE - 4,
            FLASH_BASE + 1,
            FLASH_BASE + 0xffffe,
            FLASH_BASE + 0x100000,
        ] {
            assert!(board.read_bypass_word(pa).is_err());
        }
    }

    #[test]
    fn bar_setup_follows_onboard_apb_bus_numbers() {
        let mut board = fixture();
        let bar = PCI_CONFIG + 0x10814;
        let write_bar = |b: &mut Ultra5ResetBoard| b.write_bypass_le_word(bar, EBUS_BAR1);
        assert!(write_bar(&mut board).is_err());
        assert!(board.read_bypass_word(bar).is_err());
        for (offset, value) in [
            (0x40, 0),
            (0x818, 0),
            (0x819, 2),
            (0x81a, 0xff),
            (0x918, 0),
            (0x919, 1),
            (0x91a, 1),
        ] {
            board.write_bypass_byte(PCI_CONFIG + offset, value).unwrap();
            assert_eq!(board.read_bypass_byte(PCI_CONFIG + offset).unwrap(), value);
        }
        // The host bus-number registers are bookkeeping.
        write_bar(&mut board).unwrap();
        // PCI byte order is 00 00 00 f1. These accessors return CPU big-endian values.
        assert_eq!(board.read_bypass_word(bar).unwrap(), 0xf1);
        assert_eq!(board.read_bypass_half(bar + 2).unwrap(), 0xf1);
        for (i, expected) in [0, 0, 0, 0xf1].into_iter().enumerate() {
            assert_eq!(board.read_bypass_byte(bar + i as u64).unwrap(), expected);
        }
        assert!(board.read_bypass_word(bar + 1).is_err());
        assert_eq!(
            board.read_bypass_xword(bar & !7).unwrap(),
            0x000000f0_000000f1
        );
        assert_eq!(board.read_bypass_word(bar + 4).unwrap(), 0);
        board.write_bypass_byte(PCI_CONFIG + 0x41, 0xff).unwrap();
        write_bar(&mut board).unwrap();
        assert!(board
            .write_bypass_le_word(PCI_CONFIG + 0x20814, EBUS_BAR1)
            .is_err());
        board.write_bypass_le_word(bar, u32::MAX).unwrap();
        assert_eq!(
            board.read_bypass_word(bar).unwrap().swap_bytes(),
            0xff80_0000
        );
        board.write_bypass_le_word(bar, 0xe100_0000).unwrap();
        assert_eq!(board.read_bypass_word(bar).unwrap(), 0xe1);
        board.write_bypass_byte(PCI_CONFIG + 0x9df, 0xc0).unwrap();
        let moved = PCI_MEMORY + 0xe100_0000 + 0x726000;
        assert!(board.read_bypass_word(AUXIO_LED).is_err());
        board.write_bypass_le_word(moved, 1).unwrap();
        assert_eq!(board.read_bypass_word(moved).unwrap(), 0x0100_0000);
        write_bar(&mut board).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x819, 3).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x919, 2).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 2).unwrap();
        assert!(board.read_bypass_word(bar).is_err());
        assert_eq!(board.read_bypass_word(bar + 0x10000).unwrap(), 0xf1);
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 0).unwrap();
        assert!(write_bar(&mut board).is_err());
        assert!(board.read_bypass_word(bar + 0x10000).is_err());
        let base = PCI_MEMORY + u64::from(EBUS_BAR1);
        for (i, value) in [0x1122_3344, 0x5566_7700, 0x0102_0304]
            .into_iter()
            .enumerate()
        {
            board
                .write_bypass_le_word(base + 0x710000 + 4 * i as u64, value)
                .unwrap();
        }
        assert_eq!(board.ebus_setup, [0x1122_3344, 0x5566_7700, 0x0102_0304]);
        assert!(board.write_bypass_le_word(base + 0x71000c, 0).is_err());
    }

    #[test]
    fn simba_and_onboard_display_report_distinct_read_only_pci_identities() {
        let mut board = fixture();
        for port in [PCI_CONFIG + 0x800, PCI_CONFIG + 0x900] {
            assert_eq!(board.read_bypass_word(port).unwrap(), 0x8e10_0050);
            board.write_bypass_word(port, 0).unwrap();
            assert_eq!(board.read_bypass_half(port + 2).unwrap(), 0x0050);
            assert_eq!(board.read_bypass_word(port + 8).unwrap(), 0x1100_0406);
            board.write_bypass_byte(port + 0xe, 0).unwrap();
            assert_eq!(board.read_bypass_byte(port + 0xe).unwrap(), 0x81);
        }
        let display = PCI_CONFIG + 0x11000;
        assert!(board.read_bypass_half(display + 2).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x919, 1).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 1).unwrap();
        board.write_bypass_word(display, 0).unwrap();
        assert_eq!(board.read_bypass_word(display).unwrap(), 0x0210_5447);
        assert_eq!(board.read_bypass_byte(display + 2).unwrap(), 0x54);
        assert_eq!(board.read_bypass_half(display + 2).unwrap(), 0x5447);
        assert_eq!(board.read_bypass_word(display + 8).unwrap(), 0x9a00_0003);
        assert_eq!(
            board.read_bypass_xword(display).unwrap(),
            0x02105447_00008002
        );
        assert!(board.read_bypass_half(display + 1).is_err());
        assert!(board.read_bypass_half(display + 0x102).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x919, 2).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 2).unwrap();
        assert!(board.read_bypass_half(display + 2).is_err());
        assert_eq!(board.read_bypass_half(display + 0x10002).unwrap(), 0x5447);
    }

    #[test]
    fn pcio_header_obeys_pci_lanes_and_is_independent_of_host_and_simba() {
        let mut board = fixture();
        let ebus = PCI_CONFIG + 0x10800;
        assert!(board.read_bypass_word(ebus).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x919, 1).unwrap();
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 1).unwrap();
        assert_eq!(board.read_bypass_word(ebus).unwrap(), 0x8e10_0010);
        assert_eq!(
            board.read_bypass_xword(ebus + 8).unwrap(),
            0x0100_8006_0000_8000
        );
        assert_eq!(board.read_bypass_word(ebus + 4).unwrap(), 0x0600_8002);
        board.write_bypass_half(ebus + 4, 0x4601).unwrap();
        assert_eq!(board.read_bypass_byte(ebus + 4).unwrap(), 0x46);
        assert_eq!(board.read_bypass_half(ebus + 4).unwrap(), 0x4601);
        assert_eq!(board.read_bypass_half(PCI_CONFIG + 4).unwrap(), 0x0600);
        assert_eq!(board.read_bypass_half(PCI_CONFIG + 0x904).unwrap(), 0x0200);
        board.write_bypass_half(ebus + 0x104, 0x0200).unwrap();
        assert_eq!(board.read_bypass_half(ebus + 0x104).unwrap(), 0x0200);
        assert_eq!(board.read_bypass_half(ebus + 4).unwrap(), 0x4601);
        assert!(board.write_bypass_half(ebus + 5, 0).is_err());
        assert!(board.write_bypass_half(ebus + 0x10004, 0).is_err());
        board.write_bypass_xword(ebus, u64::MAX).unwrap();
        assert_eq!(
            board.read_bypass_xword(ebus).unwrap(),
            0x8e10_0010_4601_8002
        );
        board.write_bypass_word(ebus + 0xc, 0x1040_ffff).unwrap();
        assert_eq!(board.read_bypass_word(ebus + 0xc).unwrap(), 0x1040_8000);
        board.write_bypass_le_word(ebus + 4, 0x0000_0100).unwrap();
        assert_eq!(board.read_bypass_word(ebus + 4).unwrap(), 0x0001_8002);
        assert_eq!(board.read_bypass_word(ebus + 0x14).unwrap(), 0xf1);
    }

    #[test]
    fn super_io_id_is_one_shot_and_paired_writes_are_interruptible() {
        let mut board = fixture();
        let index = PCI_MEMORY + u64::from(EBUS_BAR1) + SIO_INDEX;
        let data = index + 1;
        assert_eq!(board.read_bypass_byte(index).unwrap(), 0x88);
        assert_eq!(board.read_bypass_byte(index).unwrap(), 0);
        board.write_bypass_byte(index, 1).unwrap();
        for v in [0x11, 0x11] {
            board.write_bypass_byte(data, v).unwrap();
        }
        assert_eq!(board.read_bypass_byte(data).unwrap(), 0x11);
        board.write_bypass_byte(data, 0xb0).unwrap();
        assert_eq!(board.read_bypass_byte(data).unwrap(), 0x11);
        board.write_bypass_byte(index, 0x41).unwrap();
        board.write_bypass_byte(data, 0xb0).unwrap();
        board.write_bypass_byte(data, 0xb0).unwrap();
        assert_eq!(board.read_bypass_byte(index).unwrap(), 1);
        assert_eq!(board.read_bypass_byte(data).unwrap(), 0xb0);
        assert!(board.write_bypass_byte(index, 0x80).is_err());
        assert_eq!(board.read_bypass_byte(index).unwrap(), 1);
    }

    #[test]
    fn last_sram_byte_and_clock_registers_decode_separately_on_ebus() {
        let mut board = fixture();
        let base = crate::ultra5::RESET_NVRAM_BASE;
        board.write_bypass_byte(base + 0x1fef, 0x5a).unwrap();
        assert_eq!(board.read_bypass_byte(base + 0x1fef).unwrap(), 0x5a);
        assert_eq!(board.read_bypass_byte(base + 0x1ff0).unwrap(), 0);
        assert!(board.write_bypass_word(base + 0x1ff8, u32::MAX).is_err());
        assert!(board.read_bypass_byte(base + 0x2000).is_err());
        board.elapsed_ns = 1_000_000_000;
        board.write_bypass_byte(base + 0x1ff8, 0x40).unwrap();
        board.elapsed_ns += 2_000_000_000;
        assert_eq!(board.read_bypass_byte(base + 0x1ff9).unwrap(), 1);
        board.write_bypass_byte(base + 0x1ff8, 0).unwrap();
        assert_eq!(board.read_bypass_byte(base + 0x1ff9).unwrap(), 3);
        assert_eq!(board.read_bypass_byte(base + 0x1fef).unwrap(), 0x5a);
    }

    #[test]
    fn firmware_table_is_read_only_and_nvram_writes_preserve_endian_and_device_boundaries() {
        let mut board = fixture();
        let base = crate::ultra5::RESET_NVRAM_BASE;
        board
            .write_bypass_xword(base + 16, 0x0102_0304_0506_0708)
            .unwrap();
        board.write_bypass_word(base + 20, 0xaabb_ccdd).unwrap();
        assert_eq!(
            board.read_bypass_xword(base + 16).unwrap(),
            0x0102_0304_aabb_ccdd
        );
        assert_eq!(board.read_bypass_byte(base + 21).unwrap(), 0xbb);
        assert!(board.write_bypass_xword(base + 0x1ff0, 0).is_err());
        assert!(board.write_bypass_word(FLASH_BASE + 0x2a8, 0).is_err());
        assert_eq!(board.read_bypass_byte(FLASH_BASE + 0x2a8).unwrap(), 0);
        assert_eq!(board.read_bypass_xword(0x01fe_0000_f020).unwrap(), 1 << 31);
    }
}
