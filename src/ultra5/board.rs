use super::dram::Dram;
use super::keyboard_uart::KeyboardUart;
use super::pbm::{Pbm, PCI_CONFIG};
use super::serial::Serial;
use super::simba::Simba;
use super::{BootFlash, FlashFetch, ResetNvram, FLASH_BASE};
use crate::v9::{CpuFault, Translation, V9Cpu, V9DataBus};

const APB_TICK: u64 = PCI_CONFIG + 0x08b0; // primary bus, device 1 function 0
const PCI_MEMORY: u64 = 0x01ff_0000_0000;
const EBUS_BAR1: u32 = 0xf100_0000;
const SIO_INDEX: u64 = 0x30_0398;
const SIO_DATA: u64 = SIO_INDEX + 1;
const KEYBOARD_UART: u64 = 0x30_83f8;
const AUXIO_LED: u64 = PCI_MEMORY + EBUS_BAR1 as u64 + 0x72_6000;
const MEM_CONTROL0: u64 = 0x01fe_0000_f010;
const MEM_CONTROL1: u64 = 0x01fe_0000_f018;

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
    ebus_setup: [u32; 3],
    front_panel_led: bool,
    mem_control0: u32,
    mem_control1: u32,
    super_io: SuperIoConfig,
    keyboard: KeyboardUart,
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
            ebus_setup: [0; 3],
            front_panel_led: false,
            // IIi tables 18-3 and 18-7: DIMM pairs, refresh interval and DRAM timings.
            mem_control0: 0x0f30,
            mem_control1: 0x004a_2950,
            super_io: SuperIoConfig::default(),
            keyboard: KeyboardUart::default(),
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
        let fetch = self.step_at(cpu, mapping)?;
        Ok(FlashFetch {
            pc,
            npc,
            physical_address: mapping.pa,
            flash_offset: (mapping.pa - FLASH_BASE) as usize,
            instruction: fetch.instruction,
        })
    }

    pub fn step_mapped(&mut self, cpu: &mut V9Cpu) -> Result<BoardInstruction, String> {
        let mapping = cpu.instruction_address()?;
        Ok(self.step_at(cpu, mapping)?)
    }

    /// Run one retirement, fast MMU miss or eligible software interrupt.
    /// The retirement-only step_mapped API stops before trap or interrupt entry.
    pub fn step_outcome(&mut self, cpu: &mut V9Cpu) -> Result<BoardStep, String> {
        let fault_pc = cpu.pc();
        let mapping = cpu.instruction_address();
        let fetch = mapping.as_ref().ok().copied();
        let result = match mapping {
            Ok(fetch) => self.step_at(cpu, fetch),
            Err(fault) => Err(fault),
        };
        match result {
            Ok(step) => Ok(BoardStep::Retired(step)),
            Err(CpuFault::TlbMiss(miss)) => {
                let tt = cpu.enter_mmu_miss(miss, fetch)?;
                self.elapsed_ns = self.elapsed_ns.saturating_add(crate::machine::STEP_NS);
                Ok(BoardStep::TrapTaken {
                    fault_pc,
                    tt,
                    vector: cpu.pc(),
                })
            }
            Err(CpuFault::Interrupt(level)) => {
                cpu.enter_interrupt(level)?;
                self.elapsed_ns = self.elapsed_ns.saturating_add(crate::machine::STEP_NS);
                Ok(BoardStep::InterruptTaken {
                    pc: fault_pc,
                    level,
                    vector: cpu.pc(),
                })
            }
            Err(CpuFault::Stop(message)) => Err(message),
        }
    }

    fn step_at(
        &mut self,
        cpu: &mut V9Cpu,
        mapping: Translation,
    ) -> Result<BoardInstruction, CpuFault> {
        let instruction = if mapping.cacheable {
            let word = self.dram.read(mapping.pa & !7, self.mem_control0)?;
            if mapping.pa & 4 == 0 {
                (word >> 32) as u32
            } else {
                word as u32
            }
        } else {
            self.check_pci_memory(mapping.pa)?;
            self.flash.read_word(mapping.pa)?
        };
        cpu.retire_fetch(mapping, instruction, self)?;
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

    /// Drain bytes actually transmitted by the guest, channel A followed by B.
    pub fn take_serial_output(&mut self) -> [Vec<u8>; 2] {
        self.serial.take_output()
    }

    /// Whether the host bridge is selected by an incoming PCI single-address
    /// memory cycle. Actual DMA still requires a bus master and IOMMU.
    pub fn selects_pci_memory(&self, address: u32) -> bool {
        self.pbm.selects_pci_memory(address)
    }

    fn ebus_offset(pa: u64) -> Option<u64> {
        pa.checked_sub(PCI_MEMORY + u64::from(EBUS_BAR1))
            .filter(|&off| off < 0x80_0000)
    }

    fn check_pci_memory(&self, pa: u64) -> Result<(), String> {
        if let Some(address) = pa
            .checked_sub(PCI_MEMORY)
            .and_then(|p| u32::try_from(p).ok())
        {
            if !self.apb.forwards_onboard_memory(address) {
                return Err(format!(
                    "Ultra 5 Simba does not route PCI memory {address:#010x} to onboard bus B"
                ));
            }
        }
        Ok(())
    }

    fn read_config<const N: usize>(&self, pa: u64) -> Option<[u8; N]> {
        self.pbm.header.read(pa).or_else(|| self.apb.read(pa))
    }

    fn write_config<const N: usize>(&mut self, pa: u64, bytes: [u8; N]) -> bool {
        self.pbm.header.write(pa, bytes) || self.apb.write(pa, bytes)
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

    fn pcio_bar1_at(&self, offset: u64) -> bool {
        self.apb
            .onboard_config_bus()
            .is_some_and(|bus| offset == (u64::from(bus) << 16) + 0x814)
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
    fn read_bypass_half(&mut self, pa: u64) -> Result<u16, String> {
        if let Some(bytes) = self.read_config(pa) {
            return Ok(u16::from_be_bytes(bytes));
        }
        Err(format!(
            "unmapped Ultra 5 physical halfword read at {pa:#x}"
        ))
    }
    fn write_bypass_half(&mut self, pa: u64, value: u16) -> Result<(), String> {
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
        self.check_pci_memory(pa)?;
        if self.write_config(pa, value.to_be_bytes()) {
            return Ok(());
        }
        if pa == APB_TICK {
            return self.clear_tick(value.swap_bytes());
        }
        match pa {
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
        self.check_pci_memory(pa)?;
        if let Some(bytes) = self.read_config(pa) {
            return Ok(u32::from_be_bytes(bytes));
        }
        if pa == APB_TICK {
            return Ok(self.tick().swap_bytes());
        }
        if pa == AUXIO_LED {
            // PCIO bit 0 is in the first byte on the little-endian EBus interface.
            return Ok(u32::from(self.front_panel_led).swap_bytes());
        }
        let i = self.nvram.sram_index(pa, 4)?;
        Ok(u32::from_be_bytes(
            self.nvram.image[i..i + 4].try_into().unwrap(),
        ))
    }
    fn write_bypass_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        self.check_pci_memory(pa)?;
        if self.write_config(pa, value.to_be_bytes()) {
            return Ok(());
        }
        if pa == APB_TICK {
            return self.clear_tick(u64::from(value.swap_bytes()));
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
        self.check_pci_memory(pa)?;
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
            MEM_CONTROL0 => return Ok(u64::from(self.mem_control0)),
            MEM_CONTROL1 => return Ok(u64::from(self.mem_control1)),
            // A freshly constructed board starts from SYS_RESET_L (IIi table 17-2).
            0x01fe_0000_f020 => return Ok(1 << 31),
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
        self.check_pci_memory(pa)?;
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
            Some(0..=0x0f_ffff) => Ok(self.nvram.image[self.nvram.sram_index(pa, 1)?]),
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
            Some(off) if (KEYBOARD_UART..KEYBOARD_UART + 8).contains(&off) => self
                .keyboard
                .read(self.super_io.keyboard_register(off)?, self.elapsed_ns),
            Some(off @ 0x40_0000..=0x40_007f) => self.serial.read((off - 0x40_0000) as u8),
            _ => Err(format!("unmapped Ultra 5 physical byte read at {pa:#x}")),
        }
    }

    fn write_bypass_byte(&mut self, pa: u64, value: u8) -> Result<(), String> {
        self.check_pci_memory(pa)?;
        if self.write_config(pa, [value]) {
            return Ok(());
        }
        if let Some(offset) = pa.checked_sub(PCI_CONFIG).filter(|&off| off < 1 << 24) {
            *self.config_byte(offset)? = value;
            return Ok(());
        }
        match Self::ebus_offset(pa) {
            Some(SIO_INDEX) => self.super_io.select(value)?,
            Some(SIO_DATA) => self.super_io.write_data(value),
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
        self.check_pci_memory(pa)?;
        if self.write_config(pa, value.to_le_bytes()) {
            return Ok(());
        }
        if pa == APB_TICK {
            return self.clear_tick(u64::from(value));
        }
        if pa == AUXIO_LED {
            return self.write_bypass_word(pa, value.swap_bytes());
        }
        if pa
            .checked_sub(PCI_CONFIG)
            .is_some_and(|offset| self.pcio_bar1_at(offset))
        {
            // Scope the fixture to the board's reset aperture. BAR sizing and
            // relocation require additional bridge decoding and are explicit stops.
            if value != EBUS_BAR1 {
                return Err(format!("unsupported Ultra 5 PCIO BAR1 value {value:#x}"));
            }
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
        assert!(board.read_bypass_byte(0x01ff_f130_62fd).is_err()); // distinct mouse UART
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
        assert!(board.write_bypass_byte(port + 1, 1).is_err()); // IRQ delivery unsupported
        assert!(board.write_bypass_byte(port + 4, 0x10).is_err()); // loopback
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
        for (offset, disabled, enabled) in [(0x904, 0, 2), (0x9df, 0x40, 0x80)] {
            board
                .write_bypass_byte(PCI_CONFIG + offset, disabled)
                .unwrap();
            let old_cpu = cpu.clone();
            assert!(board.step(&mut cpu).unwrap_err().contains("onboard bus B"));
            assert_eq!(cpu, old_cpu);
            assert!(board.read_bypass_byte(FLASH_BASE + 0x20).is_err());
            assert!(board.read_bypass_xword(FLASH_BASE + 0x20).is_err());
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
    fn bar_setup_follows_onboard_apb_bus_numbers() {
        let mut board = fixture();
        let bar = PCI_CONFIG + 0x10814;
        let write_bar = |b: &mut Ultra5ResetBoard| b.write_bypass_le_word(bar, EBUS_BAR1);
        assert!(write_bar(&mut board).is_err());
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
        write_bar(&mut board).unwrap(); // host bus-number registers are bookkeeping
        board.write_bypass_byte(PCI_CONFIG + 0x41, 0xff).unwrap();
        write_bar(&mut board).unwrap();
        assert!(board
            .write_bypass_le_word(PCI_CONFIG + 0x20814, EBUS_BAR1)
            .is_err());
        assert!(board.write_bypass_le_word(bar, 0xe100_0000).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 0).unwrap();
        assert!(write_bar(&mut board).is_err());
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
