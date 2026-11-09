//! Machine-owned physical decoding. Device bytes never pass through RAM's
//! word-based narrow-access defaults or a speculative read-modify-write.
use crate::bus::{Bus, BusError, BusErrorKind, IoTlbFlush, MapError, MemoryBus, MemoryRequest};
use crate::devices::{
    auxiliary::Auxiliary,
    cgsix::{Cgsix, VRAM_BYTES, VRAM_OFFSET},
    cs4231::Cs4231,
    esp::Esp,
    ethernet::Ethernet,
    ethernet_dma::EthernetDma,
    floppy::Floppy,
    interrupt::{InterruptController, Source},
    io_control::{IoControl, IO_CONTROL_SIZE},
    keyboard::Keyboard,
    mouse::Mouse,
    nvram::{Nvram, NVRAM_SIZE},
    reset::ResetControl,
    scsi::ScsiBus,
    scsi_dma::ScsiDma,
    serial::Serial,
    timer::Timers,
};
use crate::dram::{Ss5Dram, APERTURE_BYTES};

pub const IO_CONTROL_BASE: u64 = 0x1000_0000;
pub const CGSIX_BASE: u64 = 0x3000_0000; // SS5 external SBus slot 1
const CGSIX_SIZE: u64 = 0x0200_0000;
/// SS5 onboard Crystal CS4231 audio register aperture.
pub const CS4231_BASE: u64 = 0x6c00_0000;
/// SS-5 board-level AFX presence byte. The selected SBus CGsix profile has
/// no AFX card. Only the firmware-observed byte read is implemented.
pub const AFX_PRESENCE_BASE: u64 = 0x6e00_0000;
pub const KEYBOARD_SERIAL_BASE: u64 = 0x7100_0000;
pub const SERIAL_BASE: u64 = 0x7110_0000;
pub const NVRAM_BASE: u64 = 0x7120_0000;
pub const FLOPPY_BASE: u64 = 0x7140_0000;
pub const AUXILIARY_BASE: u64 = 0x7190_0000;
pub const AUXILIARY_POWER_BASE: u64 = 0x7191_0000;
pub const CPU_TIMER_BASE: u64 = 0x71d0_0000;
pub const SYSTEM_TIMER_BASE: u64 = 0x71d1_0000;
pub const CPU_INTERRUPT_BASE: u64 = 0x71e0_0000;
pub const SYSTEM_INTERRUPT_BASE: u64 = 0x71e1_0000;
pub const RESET_BASE: u64 = 0x71f0_0000;
pub const MACIO_ID_BASE: u64 = 0x7800_0000;
pub const SCSI_DMA_BASE: u64 = 0x7840_0000;
pub const ETHERNET_DMA_BASE: u64 = 0x7840_0010;
pub const ESP_BASE: u64 = 0x7880_0000;
pub const ETHERNET_BASE: u64 = 0x78c0_0000;
const MACIO_DMA_SIZE: u64 = 0x40;
const DEVICE_RANGES: [(u64, u64); 18] = [
    (IO_CONTROL_BASE, IO_CONTROL_BASE + IO_CONTROL_SIZE),
    (AFX_PRESENCE_BASE, AFX_PRESENCE_BASE + 1),
    (CS4231_BASE, CS4231_BASE + 0x40),
    (KEYBOARD_SERIAL_BASE, KEYBOARD_SERIAL_BASE + 8),
    (SERIAL_BASE, SERIAL_BASE + 8),
    (NVRAM_BASE, NVRAM_BASE + NVRAM_SIZE as u64),
    (FLOPPY_BASE, FLOPPY_BASE + 0x10_0000),
    (AUXILIARY_BASE, AUXILIARY_BASE + 1),
    (AUXILIARY_POWER_BASE, AUXILIARY_POWER_BASE + 1),
    (CPU_TIMER_BASE, CPU_TIMER_BASE + 0x10),
    (SYSTEM_TIMER_BASE, SYSTEM_TIMER_BASE + 0x14),
    (CPU_INTERRUPT_BASE, CPU_INTERRUPT_BASE + 0x10),
    (SYSTEM_INTERRUPT_BASE, SYSTEM_INTERRUPT_BASE + 0x14),
    (RESET_BASE, RESET_BASE + 4),
    (MACIO_ID_BASE, MACIO_ID_BASE + 4),
    (SCSI_DMA_BASE, SCSI_DMA_BASE + MACIO_DMA_SIZE),
    (ESP_BASE, ESP_BASE + 0x40),
    (ETHERNET_BASE, ETHERNET_BASE + 4),
];

#[derive(Clone, Copy)]
enum WordDevice {
    IoControl,
    Cs4231,
    Timer(bool),
    Interrupt(bool),
    Reset,
    MacioId,
    ScsiDma,
    EthernetDma,
}

#[derive(Debug)]
pub struct Sun4mDevices {
    pub auxiliary: Auxiliary,
    pub cs4231: Cs4231,
    pub floppy: Floppy,
    pub io_control: IoControl,
    pub nvram: Nvram,
    pub serial: Serial,
    /// SCC channels A (keyboard) and B (Sun Mouse Systems).
    pub keyboard_serial: Serial,
    pub keyboard: Option<Keyboard>,
    pub mouse: Mouse,
    pub interrupts: InterruptController,
    pub timers: Timers,
    pub reset: ResetControl,
    pub scsi_dma: ScsiDma,
    pub esp: Esp,
    pub scsi: ScsiBus,
    pub ethernet_dma: EthernetDma,
    pub ethernet: Ethernet,
    rtc_phase_ns: u32,
    timer_ns: u64,
}

impl Sun4mDevices {
    pub(crate) fn sync_interrupts(&mut self) {
        self.ethernet_dma.set_mac_irq(self.ethernet.irq());
        self.scsi_dma.set_scsi_irq(self.esp.irq());
        self.interrupts
            .set_source(Source::Scsi, self.scsi_dma.irq());
        self.interrupts
            .set_source(Source::Ethernet, self.ethernet_dma.irq());
        self.interrupts
            .set_source(Source::Floppy, self.floppy.irq());
        self.interrupts
            .set_source(Source::Serial, self.serial.irq());
        self.interrupts
            .set_source(Source::Keyboard, self.keyboard_serial.irq());
        self.interrupts
            .set_source(Source::SystemTimer, self.timers.system_irq());
        self.interrupts.set_cpu_timer(self.timers.processor_irq());
    }

    fn advance_ns(&mut self, ns: u64) {
        if let Some(keyboard) = &mut self.keyboard {
            keyboard.advance_ns(ns, &mut self.keyboard_serial);
        }
        if !self.mouse.idle() {
            self.mouse.advance_ns(ns, &mut self.keyboard_serial);
        }
        self.ethernet.advance_ns(ns);
        self.esp.advance_ns(ns);
        self.floppy.advance_ns(ns);
        self.timer_ns += ns;
        if self.timer_ns >= 500 {
            self.timers.advance_ns(self.timer_ns);
            self.timer_ns = 0;
        }
        let remainder = ns % 1_000_000_000 + u64::from(self.rtc_phase_ns);
        self.nvram
            .advance_seconds(ns / 1_000_000_000 + remainder / 1_000_000_000);
        self.rtc_phase_ns = (remainder % 1_000_000_000) as u32;
        self.sync_interrupts();
    }
}

#[derive(Debug, Default)]
pub struct SystemBus {
    memory: MemoryBus,
    devices: Option<Sun4mDevices>,
    dram: Option<Ss5Dram>,
    cpu_user: bool,
    prom_cycles: Option<u64>,
    prom_phase: u64,
    cgsix: Option<Cgsix>,
    device_ns: u64,
}

/// SS5/85 SBus clock (CPU / 4), SPARCengine 5 table 2-1.
pub const SBUS_CLOCK_HZ: u64 = 21_250_000;
/// NCR89C105 table 6-10: six SBus clocks for each byte-wide PROM read.
pub const PROM_BYTE_CYCLES: u64 = 6;

impl SystemBus {
    pub fn install_cgsix(&mut self, card: Cgsix) -> Result<(), MapError> {
        if self.cgsix.is_some() || self.memory.overlaps(CGSIX_BASE, CGSIX_BASE + CGSIX_SIZE) {
            return Err(MapError::Overlap);
        }
        self.cgsix = Some(card);
        Ok(())
    }
    pub fn cgsix(&self) -> Option<&Cgsix> {
        self.cgsix.as_ref()
    }
    fn sync_cgsix_irq(&mut self) {
        if let (Some(devices), Some(card)) = (&mut self.devices, &self.cgsix) {
            devices.interrupts.set_source(Source::SBus5, card.irq());
        }
    }
    fn cgsix_at(&mut self, address: u64) -> Option<&mut Cgsix> {
        if (CGSIX_BASE..CGSIX_BASE + CGSIX_SIZE).contains(&address) {
            self.cgsix.as_mut()
        } else {
            None
        }
    }
    pub(crate) fn begin_cpu_step(&mut self) {
        self.prom_cycles = Some(0);
    }

    pub(crate) fn finish_cpu_step(&mut self, completed: bool) -> u64 {
        let cycles = self.prom_cycles.take().unwrap_or(0);
        if !completed {
            return 0;
        }
        let numerator = cycles * 1_000_000_000 + self.prom_phase;
        self.prom_phase = numerator % SBUS_CLOCK_HZ;
        numerator / SBUS_CLOCK_HZ
    }

    fn account_prom_read(&mut self, address: u64, bytes: u64) {
        if self.devices.is_some() && (0x7000_0000..0x7010_0000).contains(&address) {
            if let Some(cycles) = &mut self.prom_cycles {
                *cycles += bytes * PROM_BYTE_CYCLES;
            }
        }
    }

    fn check_macio_access(&mut self, address: u64, bytes: u64) -> Result<(), BusError> {
        let dma = (SCSI_DMA_BASE..SCSI_DMA_BASE + MACIO_DMA_SIZE).contains(&address);
        let scsi = dma && address & 0x10 == 0;
        let ethernet = dma && address & 0x10 != 0;
        let mac = (ETHERNET_BASE..ETHERNET_BASE + 4).contains(&address);
        let id = (MACIO_ID_BASE..MACIO_ID_BASE + 4).contains(&address);
        let esp = (ESP_BASE..ESP_BASE + 0x40).contains(&address);
        if let Some(devices) = &mut self.devices {
            if esp && bytes != 1 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("wide ESP access"),
                });
            }
            if scsi || ethernet || mac || id {
                if address % bytes != 0 {
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::Misaligned,
                    });
                }
                if ((scsi || ethernet) && bytes != 4) || (mac && bytes != 2) || bytes > 4 {
                    if scsi {
                        devices.scsi_dma.slave_access_error();
                    }
                    if ethernet || mac {
                        devices.ethernet_dma.slave_access_error();
                    }
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::SBusError,
                    });
                }
            }
        }
        Ok(())
    }
    fn check_floppy_access(&self, address: u64, bytes: u64) -> Result<(), BusError> {
        if self.devices.is_some() && (FLOPPY_BASE..FLOPPY_BASE + 0x10_0000).contains(&address) {
            if address % bytes != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            if bytes != 1 {
                // NCR89C105 rejects wider transfers throughout this aperture.
                return Err(BusError {
                    address,
                    kind: BusErrorKind::SBusError,
                });
            }
        }
        Ok(())
    }
    fn physical_error(&self, error: BusError) -> BusError {
        // microSPARC-II table 15: PA[30:28]=3..7 select SBus slots.
        if self.devices.is_some()
            && (0x3000_0000..0x8000_0000).contains(&error.address)
            && error.kind == BusErrorKind::Unmapped
        {
            // Reserved NCR89C105 windows are decoded by an existing slave,
            // unlike an empty SBus slot. Keep missing register mirrors and
            // devices outside these documented holes separate.
            let reserved_ncr = (0x7150_0000..0x7180_0000).contains(&error.address)
                || (0x71c0_0000..0x71d0_0000).contains(&error.address)
                || (0x71e0_0010..0x71e1_0000).contains(&error.address)
                || (0x71e1_0014..0x71f0_0000).contains(&error.address)
                || (0x71f0_0004..0x7200_0000).contains(&error.address);
            BusError {
                kind: if reserved_ncr {
                    BusErrorKind::SBusError
                } else {
                    BusErrorKind::SBusTimeout
                },
                ..error
            }
        } else {
            error
        }
    }
    pub fn install_ss5_dram(&mut self, dram: Ss5Dram) -> Result<(), MapError> {
        if self.dram.is_some() || self.memory.overlaps(0, APERTURE_BYTES) {
            return Err(MapError::Overlap);
        }
        self.dram = Some(dram);
        Ok(())
    }

    pub fn dram(&self) -> Option<&Ss5Dram> {
        self.dram.as_ref()
    }

    fn dram_at(&mut self, address: u64) -> Option<&mut Ss5Dram> {
        if address < APERTURE_BYTES {
            self.dram.as_mut()
        } else {
            None
        }
    }

    pub fn install_sun4m(&mut self, nvram: Nvram) -> Result<(), MapError> {
        if self.devices.is_some()
            || DEVICE_RANGES
                .iter()
                .any(|&(start, end)| self.memory.overlaps(start, end))
        {
            return Err(MapError::Overlap);
        }
        self.devices = Some(Sun4mDevices {
            auxiliary: Auxiliary::default(),
            cs4231: Cs4231::default(),
            floppy: Floppy::default(),
            io_control: IoControl::default(),
            nvram,
            serial: Serial::default(),
            keyboard_serial: Serial::default(),
            keyboard: None,
            mouse: Mouse::default(),
            interrupts: InterruptController::default(),
            timers: Timers::default(),
            reset: ResetControl::default(),
            scsi_dma: ScsiDma::default(),
            esp: Esp::default(),
            scsi: ScsiBus::default(),
            ethernet_dma: EthernetDma::default(),
            ethernet: Ethernet::default(),
            rtc_phase_ns: 0,
            timer_ns: 0,
        });
        Ok(())
    }
    pub fn devices(&self) -> Option<&Sun4mDevices> {
        self.devices.as_ref()
    }
    pub fn devices_mut(&mut self) -> Option<&mut Sun4mDevices> {
        self.devices.as_mut()
    }
    pub fn reset_devices(&mut self) {
        self.reset_io();
        if let Some(devices) = &mut self.devices {
            devices.reset.power_on_reset();
        }
    }
    pub(crate) fn software_reset_devices(&mut self) {
        self.reset_io();
        if let Some(devices) = &mut self.devices {
            devices.reset.software_reset();
        }
    }
    fn reset_io(&mut self) {
        if let Some(card) = &mut self.cgsix {
            card.reset();
        }
        self.cpu_user = false;
        self.prom_cycles = None;
        self.prom_phase = 0;
        self.device_ns = 0;
        if let Some(devices) = &mut self.devices {
            devices.io_control.reset();
            devices.auxiliary.reset();
            devices.cs4231.reset();
            devices.floppy.reset();
            devices.scsi_dma.reset();
            devices.esp.reset();
            devices.ethernet_dma.reset();
            devices.ethernet.reset();
            devices.serial.reset();
            devices.keyboard_serial.reset();
            if let Some(keyboard) = &mut devices.keyboard {
                keyboard.reset();
            }
            devices.mouse.reset();
            devices.interrupts.reset();
            devices.timers.reset();
            devices.timer_ns = 0;
        }
    }
    pub fn advance_ns(&mut self, ns: u64) {
        self.advance_devices_ns(ns);
    }

    /// CPU-boundary path: coalesce device work to the 500 ns timer tick.
    pub(crate) fn advance_ns_fast(&mut self, ns: u64) {
        self.device_ns = self.device_ns.saturating_add(ns);
        if self.device_ns < 500 {
            return;
        }
        let elapsed = std::mem::take(&mut self.device_ns);
        self.advance_devices_ns(elapsed);
    }

    fn advance_devices_ns(&mut self, ns: u64) {
        if let Some(card) = &mut self.cgsix {
            card.advance_ns(ns);
        }
        if let Some(devices) = &mut self.devices {
            devices.advance_ns(ns);
        }
        self.sync_cgsix_irq();
    }
    pub fn advance_seconds(&mut self, seconds: u64) {
        if self.device_ns != 0 {
            let pending = std::mem::take(&mut self.device_ns);
            self.advance_devices_ns(pending);
        }
        if let Some(card) = &mut self.cgsix {
            card.advance_ns(seconds.saturating_mul(1_000_000_000));
        }
        if let Some(devices) = &mut self.devices {
            if let Some(keyboard) = &mut devices.keyboard {
                keyboard.advance_ns(
                    seconds.saturating_mul(1_000_000_000),
                    &mut devices.keyboard_serial,
                );
            }
            devices
                .ethernet
                .advance_ns(seconds.saturating_mul(1_000_000_000));
            devices
                .esp
                .advance_ns(seconds.saturating_mul(1_000_000_000));
            devices
                .floppy
                .advance_ns(seconds.saturating_mul(1_000_000_000));
            devices.timers.advance_seconds(seconds);
            devices.nvram.advance_seconds(seconds);
            devices.sync_interrupts();
        }
        self.sync_cgsix_irq();
    }
    pub fn interrupt_levels(&mut self) -> u16 {
        self.sync_cgsix_irq();
        self.devices
            .as_ref()
            .map_or(0, |devices| devices.interrupts.levels())
    }
    fn word_device(&self, address: u64) -> Option<(WordDevice, u64)> {
        self.devices.as_ref()?;
        // NCR89C100 table 2-7 footnote 2 and DMA2 table 3-1: bit 5 is
        // not decoded. The second 32 bytes alias the same register bank.
        if (SCSI_DMA_BASE..SCSI_DMA_BASE + MACIO_DMA_SIZE).contains(&address) {
            let device = if address & 0x10 == 0 {
                WordDevice::ScsiDma
            } else {
                WordDevice::EthernetDma
            };
            return Some((device, address & 0xf));
        }
        for (base, size, device) in [
            (IO_CONTROL_BASE, IO_CONTROL_SIZE, WordDevice::IoControl),
            (CS4231_BASE, 0x40, WordDevice::Cs4231),
            (CPU_TIMER_BASE, 0x10, WordDevice::Timer(false)),
            (SYSTEM_TIMER_BASE, 0x14, WordDevice::Timer(true)),
            (CPU_INTERRUPT_BASE, 0x10, WordDevice::Interrupt(false)),
            (SYSTEM_INTERRUPT_BASE, 0x14, WordDevice::Interrupt(true)),
            (RESET_BASE, 4, WordDevice::Reset),
            (MACIO_ID_BASE, 4, WordDevice::MacioId),
        ] {
            if (base..base + size).contains(&address) {
                return Some((device, address - base));
            }
        }
        None
    }
    fn reject_narrow_word_device(&self, address: u64) -> Result<(), BusError> {
        if self.word_device(address).is_some() {
            Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("device requires plain aligned word load/store"),
            })
        } else {
            Ok(())
        }
    }
    fn overlaps_device(&self, address: u64, size: u64) -> bool {
        (self.cgsix.is_some()
            && address < CGSIX_BASE + CGSIX_SIZE
            && address.saturating_add(size) > CGSIX_BASE)
            || self.devices.is_some()
                && DEVICE_RANGES
                    .iter()
                    .any(|&(start, end)| address < end && address.saturating_add(size) > start)
    }
    fn map_check(&self, address: u64, size: usize) -> Result<(), MapError> {
        if address.checked_add(size as u64).is_none() {
            return Err(MapError::InvalidRange);
        }
        if self.overlaps_device(address, size as u64)
            || (self.dram.is_some() && size != 0 && address < APERTURE_BYTES)
        {
            Err(MapError::Overlap)
        } else {
            Ok(())
        }
    }
    pub fn map_ram(&mut self, address: u64, size: usize) -> Result<(), MapError> {
        self.map_check(address, size)?;
        self.memory.map_ram(address, size)
    }
    pub fn map_rom(&mut self, address: u64, bytes: &[u8]) -> Result<(), MapError> {
        self.map_check(address, bytes.len())?;
        self.memory.map_rom(address, bytes)
    }
    pub fn load_image(&mut self, address: u64, bytes: &[u8]) -> Result<(), BusError> {
        if let Some(dram) = self.dram_at(address) {
            return dram.load_image(address, bytes);
        }
        if self.overlaps_device(address, bytes.len() as u64) {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("raw image loading into devices"),
            });
        }
        self.memory.load_image(address, bytes)
    }
    fn memory_only(&self, address: u64, size: u64) -> Result<(), BusError> {
        if address % size != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if self.overlaps_device(address, size) {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported(
                    "unsupported device access width or atomic operation",
                ),
            });
        }
        Ok(())
    }
}

impl Bus for SystemBus {
    fn set_cpu_supervisor(&mut self, supervisor: bool) {
        self.cpu_user = !supervisor;
    }

    fn observe_memory_request(&mut self, address: u64, request: MemoryRequest) {
        if address < APERTURE_BYTES {
            if let Some(devices) = &mut self.devices {
                devices
                    .io_control
                    .memory_request(address, request, !self.cpu_user);
            }
        }
    }

    fn capture_sbus_write_error(&mut self, error: BusError, bytes: u8) -> bool {
        if !matches!(
            error.kind,
            BusErrorKind::SBusTimeout | BusErrorKind::SBusError
        ) || !matches!(bytes, 1 | 2 | 4)
        {
            return false;
        }
        if let Some(devices) = &mut self.devices {
            devices
                .io_control
                .sbus_write_error(error, bytes, !self.cpu_user);
            devices.interrupts.latch_module_error();
            true
        } else {
            false
        }
    }
    fn take_io_tlb_flush(&mut self) -> Option<IoTlbFlush> {
        self.devices.as_mut()?.io_control.take_flush()
    }

    fn read8(&mut self, address: u64) -> Result<u8, BusError> {
        if let Some(card) = self.cgsix_at(address) {
            return card
                .read(address - CGSIX_BASE, 1)
                .map(|v| v as u8)
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 1)?;
        if self.devices.is_some() && (MACIO_ID_BASE..MACIO_ID_BASE + 4).contains(&address) {
            return Ok((0xfe81_0103u32 >> ((3 - (address - MACIO_ID_BASE)) * 8)) as u8);
        }
        self.check_floppy_access(address, 1)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read8(address);
        }
        if let Some(devices) = self.devices.as_ref() {
            if (CS4231_BASE..CS4231_BASE + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .read_byte(address - CS4231_BASE)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.reject_narrow_word_device(address)?;
        if let Some(devices) = &mut self.devices {
            if (ESP_BASE..ESP_BASE + 0x40).contains(&address) {
                let result = if devices.scsi_dma.reset_asserted() {
                    Ok(0)
                } else {
                    devices.esp.read(address - ESP_BASE)
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == AFX_PRESENCE_BASE {
                // Original OBP afx-slot-empty? tests bit 0. The motherboard
                // presence port exists even when the AFX slot is empty.
                return Ok(0);
            }
            if (FLOPPY_BASE..FLOPPY_BASE + 0x10_0000).contains(&address) {
                let result = devices.floppy.read(address - FLOPPY_BASE);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == AUXILIARY_BASE {
                return Ok(devices.auxiliary.read());
            }
            if address == AUXILIARY_POWER_BASE {
                return Ok(devices.auxiliary.read_power());
            }
            if (KEYBOARD_SERIAL_BASE..KEYBOARD_SERIAL_BASE + 8).contains(&address) {
                let result = devices.keyboard_serial.read(address - KEYBOARD_SERIAL_BASE);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (SERIAL_BASE..SERIAL_BASE + 8).contains(&address) {
                let result = devices.serial.read(address - SERIAL_BASE);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (NVRAM_BASE..NVRAM_BASE + NVRAM_SIZE as u64).contains(&address) {
                return devices
                    .nvram
                    .read((address - NVRAM_BASE) as usize)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        let result = self.memory.read8(address);
        if result.is_ok() {
            self.account_prom_read(address, 1);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write8(&mut self, address: u64, value: u8) -> Result<(), BusError> {
        if let Some(card) = self.cgsix_at(address) {
            let shift = (3 - address % 4) * 8;
            return card
                .write(
                    (address - CGSIX_BASE) & !3,
                    u32::from(value) << shift,
                    255 << shift,
                )
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 1)?;
        self.check_floppy_access(address, 1)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write8(address, value);
        }
        if let Some(devices) = self.devices.as_mut() {
            if (CS4231_BASE..CS4231_BASE + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .write_byte(address - CS4231_BASE, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.reject_narrow_word_device(address)?;
        if let Some(devices) = &mut self.devices {
            if (ESP_BASE..ESP_BASE + 0x40).contains(&address) {
                let result = if devices.scsi_dma.reset_asserted() {
                    Ok(())
                } else {
                    let result = devices.esp.write(address - ESP_BASE, value);
                    if result.is_ok() && address == ESP_BASE + 12 && value & 0x7f == 3 {
                        devices.scsi.reset();
                    }
                    result
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == AFX_PRESENCE_BASE {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("write to AFX presence port"),
                });
            }
            if (FLOPPY_BASE..FLOPPY_BASE + 0x10_0000).contains(&address) {
                let result = devices.floppy.write(address - FLOPPY_BASE, value);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == AUXILIARY_BASE {
                devices.auxiliary.write(value);
                return Ok(());
            }
            if address == AUXILIARY_POWER_BASE {
                return devices
                    .auxiliary
                    .write_power(value)
                    .map_err(|kind| BusError { address, kind });
            }
            if (KEYBOARD_SERIAL_BASE..KEYBOARD_SERIAL_BASE + 8).contains(&address) {
                let result = devices
                    .keyboard_serial
                    .write(address - KEYBOARD_SERIAL_BASE, value);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (SERIAL_BASE..SERIAL_BASE + 8).contains(&address) {
                let result = devices.serial.write(address - SERIAL_BASE, value);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (NVRAM_BASE..NVRAM_BASE + NVRAM_SIZE as u64).contains(&address) {
                return devices
                    .nvram
                    .write((address - NVRAM_BASE) as usize, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory
            .write8(address, value)
            .map_err(|error| self.physical_error(error))
    }
    fn read16(&mut self, address: u64) -> Result<u16, BusError> {
        if let Some(card) = self.cgsix_at(address) {
            return card
                .read(address - CGSIX_BASE, 2)
                .map(|v| v as u16)
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 2)?;
        if (ETHERNET_BASE..ETHERNET_BASE + 4).contains(&address) {
            if let Some(devices) = &mut self.devices {
                return if devices.ethernet_dma.reset_asserted() {
                    Ok(0)
                } else {
                    devices
                        .ethernet
                        .read(address - ETHERNET_BASE)
                        .map_err(|kind| BusError { address, kind })
                };
            }
        }
        if self.devices.is_some() && (MACIO_ID_BASE..MACIO_ID_BASE + 4).contains(&address) {
            return Ok((0xfe81_0103u32 >> ((2 - (address - MACIO_ID_BASE)) * 8)) as u16);
        }
        self.check_floppy_access(address, 2)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read16(address);
        }
        if let Some(devices) = self.devices.as_ref() {
            if (CS4231_BASE..CS4231_BASE + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .read_half(address - CS4231_BASE)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory_only(address, 2)?;
        let result = self.memory.read16(address);
        if result.is_ok() {
            self.account_prom_read(address, 2);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write16(&mut self, address: u64, value: u16) -> Result<(), BusError> {
        if let Some(card) = self.cgsix_at(address) {
            if address & 1 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let shift = (2 - address % 4) * 8;
            return card
                .write(
                    (address - CGSIX_BASE) & !3,
                    u32::from(value) << shift,
                    65535 << shift,
                )
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 2)?;
        if (ETHERNET_BASE..ETHERNET_BASE + 4).contains(&address) {
            if let Some(devices) = &mut self.devices {
                let result = if devices.ethernet_dma.reset_asserted() {
                    Ok(())
                } else {
                    devices.ethernet.write(address - ETHERNET_BASE, value)
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
        }
        self.check_floppy_access(address, 2)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write16(address, value);
        }
        if let Some(devices) = self.devices.as_mut() {
            if (CS4231_BASE..CS4231_BASE + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .write_half(address - CS4231_BASE, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory_only(address, 2)?;
        self.memory
            .write16(address, value)
            .map_err(|error| self.physical_error(error))
    }
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        if let Some(card) = self.cgsix_at(address) {
            return card
                .read(address - CGSIX_BASE, 4)
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 4)?;
        self.check_floppy_access(address, 4)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read32(address);
        }
        if address & 3 != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if let Some((device, offset)) = self.word_device(address) {
            let devices = self.devices.as_mut().expect("decoded installed device");
            devices.sync_interrupts();
            let result = match device {
                WordDevice::IoControl => devices.io_control.read(offset),
                WordDevice::Cs4231 => devices.cs4231.read(offset),
                WordDevice::Timer(system) => devices.timers.read(system, offset),
                WordDevice::Interrupt(system) => devices.interrupts.read(system, offset),
                WordDevice::Reset => Ok(devices.reset.read()),
                WordDevice::MacioId => Ok(0xfe81_0103),
                WordDevice::ScsiDma => devices.scsi_dma.read(offset),
                WordDevice::EthernetDma => devices.ethernet_dma.read(offset),
            };
            devices.sync_interrupts();
            return result.map_err(|kind| BusError { address, kind });
        }
        self.memory_only(address, 4)?;
        let result = self.memory.read32(address);
        if result.is_ok() {
            self.account_prom_read(address, 4);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        if let Some(card) = self.cgsix_at(address) {
            let result = card
                .write(address - CGSIX_BASE, value, mask)
                .map_err(|kind| BusError { address, kind });
            self.sync_cgsix_irq();
            return result;
        }
        if mask != u32::MAX
            && self.devices.is_some()
            && (ETHERNET_BASE..ETHERNET_BASE + 4).contains(&address)
        {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("masked Ethernet register store"),
            });
        }
        self.check_macio_access(address, 4)?;
        if mask != u32::MAX
            && self.devices.is_some()
            && (FLOPPY_BASE..FLOPPY_BASE + 0x10_0000).contains(&address)
        {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("masked floppy access"),
            });
        }
        self.check_floppy_access(address, 4)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write32_masked(address, value, mask);
        }
        if address & 3 != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if let Some((device, offset)) = self.word_device(address) {
            if mask != u32::MAX {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("masked device word store"),
                });
            }
            let devices = self.devices.as_mut().expect("decoded installed device");
            let result = match device {
                WordDevice::IoControl => devices.io_control.write(offset, value),
                WordDevice::Cs4231 => devices.cs4231.write(offset, value),
                WordDevice::MacioId => Err(BusErrorKind::Unsupported("write to MACIO ID")),
                WordDevice::ScsiDma => {
                    let result = devices.scsi_dma.write(offset, value);
                    if result.is_ok() && devices.scsi_dma.reset_asserted() {
                        devices.esp.reset();
                    }
                    result
                }
                WordDevice::Timer(system) => devices.timers.write(system, offset, value),
                WordDevice::EthernetDma => {
                    let result = devices.ethernet_dma.write(offset, value);
                    if result.is_ok() && devices.ethernet_dma.reset_asserted() {
                        devices.ethernet.reset();
                    }
                    result
                }
                WordDevice::Interrupt(system) => devices.interrupts.write(system, offset, value),
                WordDevice::Reset => {
                    devices.reset.write(value);
                    Ok(())
                }
            };
            devices.sync_interrupts();
            return result.map_err(|kind| BusError { address, kind });
        }
        self.memory_only(address, 4)?;
        self.memory
            .write32_masked(address, value, mask)
            .map_err(|error| self.physical_error(error))
    }
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        if let Some(card) = self.cgsix_at(address) {
            let offset = address - CGSIX_BASE;
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            if offset >= 32768 && !(VRAM_OFFSET..VRAM_OFFSET + VRAM_BYTES as u64).contains(&offset)
            {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("CGsix register doubleword"),
                });
            }
            return Ok([
                card.read(offset, 4)
                    .map_err(|kind| BusError { address, kind })?,
                card.read(offset + 4, 4).map_err(|kind| BusError {
                    address: address + 4,
                    kind,
                })?,
            ]);
        }
        self.check_macio_access(address, 8)?;
        self.check_floppy_access(address, 8)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read_doubleword(address);
        }
        if address == CPU_TIMER_BASE {
            if let Some(devices) = &self.devices {
                return devices
                    .timers
                    .read_user_doubleword()
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory_only(address, 8)?;
        let result = self.memory.read_doubleword(address);
        if result.is_ok() {
            self.account_prom_read(address, 8);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        if let Some(card) = self.cgsix_at(address) {
            let offset = address - CGSIX_BASE;
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            if !(VRAM_OFFSET..VRAM_OFFSET + VRAM_BYTES as u64).contains(&offset) {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("CGsix register doubleword"),
                });
            }
            card.write(offset, words[0], u32::MAX)
                .map_err(|kind| BusError { address, kind })?;
            return card
                .write(offset + 4, words[1], u32::MAX)
                .map_err(|kind| BusError {
                    address: address + 4,
                    kind,
                });
        }
        self.check_macio_access(address, 8)?;
        self.check_floppy_access(address, 8)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write_doubleword(address, words);
        }
        if address == CPU_TIMER_BASE {
            if let Some(devices) = &mut self.devices {
                return devices
                    .timers
                    .write_user_doubleword(words)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory_only(address, 8)?;
        self.memory
            .write_doubleword(address, words)
            .map_err(|error| self.physical_error(error))
    }
    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        if let Some(dram) = self.dram_at(address) {
            return dram.swap32(address, value);
        }
        self.memory_only(address, 4)?;
        self.memory
            .swap32(address, value)
            .map_err(|error| self.physical_error(error))
    }
    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        if let Some(dram) = self.dram_at(address) {
            return dram.ldstub8(address);
        }
        self.memory_only(address, 1)?;
        self.memory
            .ldstub8(address)
            .map_err(|error| self.physical_error(error))
    }
}
