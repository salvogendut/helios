//! Machine-owned physical decoding. Device bytes never pass through RAM's
//! word-based narrow-access defaults or a speculative read-modify-write.
use crate::bus::{Bus, BusError, BusErrorKind, MapError, MemoryBus};
use crate::devices::{
    interrupt::{InterruptController, Source},
    nvram::{Nvram, NVRAM_SIZE},
    serial::Serial,
    timer::Timers,
};

pub const SERIAL_BASE: u64 = 0x7110_0000;
pub const NVRAM_BASE: u64 = 0x7120_0000;
pub const CPU_TIMER_BASE: u64 = 0x71d0_0000;
pub const SYSTEM_TIMER_BASE: u64 = 0x71d1_0000;
pub const CPU_INTERRUPT_BASE: u64 = 0x71e0_0000;
pub const SYSTEM_INTERRUPT_BASE: u64 = 0x71e1_0000;
const DEVICE_RANGES: [(u64, u64); 6] = [
    (SERIAL_BASE, SERIAL_BASE + 8),
    (NVRAM_BASE, NVRAM_BASE + NVRAM_SIZE as u64),
    (CPU_TIMER_BASE, CPU_TIMER_BASE + 0x10),
    (SYSTEM_TIMER_BASE, SYSTEM_TIMER_BASE + 0x14),
    (CPU_INTERRUPT_BASE, CPU_INTERRUPT_BASE + 0x10),
    (SYSTEM_INTERRUPT_BASE, SYSTEM_INTERRUPT_BASE + 0x14),
];

#[derive(Clone, Copy)]
enum WordDevice {
    Timer(bool),
    Interrupt(bool),
}

#[derive(Debug)]
pub struct Sun4mDevices {
    pub nvram: Nvram,
    pub serial: Serial,
    pub interrupts: InterruptController,
    pub timers: Timers,
    rtc_phase_ns: u32,
}

impl Sun4mDevices {
    fn sync_interrupts(&mut self) {
        self.interrupts
            .set_source(Source::SystemTimer, self.timers.system_irq());
        self.interrupts.set_cpu_timer(self.timers.processor_irq());
    }

    fn advance_ns(&mut self, ns: u64) {
        self.timers.advance_ns(ns);
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
}

impl SystemBus {
    pub fn install_sun4m(&mut self, nvram: Nvram) -> Result<(), MapError> {
        if self.devices.is_some()
            || DEVICE_RANGES
                .iter()
                .any(|&(start, end)| self.memory.overlaps(start, end))
        {
            return Err(MapError::Overlap);
        }
        self.devices = Some(Sun4mDevices {
            nvram,
            serial: Serial::default(),
            interrupts: InterruptController::default(),
            timers: Timers::default(),
            rtc_phase_ns: 0,
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
        if let Some(devices) = &mut self.devices {
            devices.serial.reset();
            devices.interrupts.reset();
            devices.timers.reset();
        }
    }
    pub fn advance_ns(&mut self, ns: u64) {
        if let Some(devices) = &mut self.devices {
            devices.advance_ns(ns);
        }
    }
    pub fn advance_seconds(&mut self, seconds: u64) {
        if let Some(devices) = &mut self.devices {
            devices.timers.advance_seconds(seconds);
            devices.nvram.advance_seconds(seconds);
            devices.sync_interrupts();
        }
    }
    pub fn interrupt_levels(&mut self) -> u16 {
        if let Some(devices) = &mut self.devices {
            devices.sync_interrupts();
            devices.interrupts.levels()
        } else {
            0
        }
    }
    fn word_device(&self, address: u64) -> Option<(WordDevice, u64)> {
        self.devices.as_ref()?;
        for (base, size, device) in [
            (CPU_TIMER_BASE, 0x10, WordDevice::Timer(false)),
            (SYSTEM_TIMER_BASE, 0x14, WordDevice::Timer(true)),
            (CPU_INTERRUPT_BASE, 0x10, WordDevice::Interrupt(false)),
            (SYSTEM_INTERRUPT_BASE, 0x14, WordDevice::Interrupt(true)),
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
        self.devices.is_some()
            && DEVICE_RANGES
                .iter()
                .any(|&(start, end)| address < end && address.saturating_add(size) > start)
    }
    fn map_check(&self, address: u64, size: usize) -> Result<(), MapError> {
        if address.checked_add(size as u64).is_none() {
            return Err(MapError::InvalidRange);
        }
        if self.overlaps_device(address, size as u64) {
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
    fn read8(&mut self, address: u64) -> Result<u8, BusError> {
        self.reject_narrow_word_device(address)?;
        if let Some(devices) = &mut self.devices {
            if (SERIAL_BASE..SERIAL_BASE + 8).contains(&address) {
                return devices
                    .serial
                    .read(address - SERIAL_BASE)
                    .map_err(|kind| BusError { address, kind });
            }
            if (NVRAM_BASE..NVRAM_BASE + NVRAM_SIZE as u64).contains(&address) {
                return devices
                    .nvram
                    .read((address - NVRAM_BASE) as usize)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory.read8(address)
    }
    fn write8(&mut self, address: u64, value: u8) -> Result<(), BusError> {
        self.reject_narrow_word_device(address)?;
        if let Some(devices) = &mut self.devices {
            if (SERIAL_BASE..SERIAL_BASE + 8).contains(&address) {
                return devices
                    .serial
                    .write(address - SERIAL_BASE, value)
                    .map_err(|kind| BusError { address, kind });
            }
            if (NVRAM_BASE..NVRAM_BASE + NVRAM_SIZE as u64).contains(&address) {
                return devices
                    .nvram
                    .write((address - NVRAM_BASE) as usize, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory.write8(address, value)
    }
    fn read16(&mut self, address: u64) -> Result<u16, BusError> {
        self.memory_only(address, 2)?;
        self.memory.read16(address)
    }
    fn write16(&mut self, address: u64, value: u16) -> Result<(), BusError> {
        self.memory_only(address, 2)?;
        self.memory.write16(address, value)
    }
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
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
                WordDevice::Timer(system) => devices.timers.read(system, offset),
                WordDevice::Interrupt(system) => devices.interrupts.read(system, offset),
            };
            devices.sync_interrupts();
            return result.map_err(|kind| BusError { address, kind });
        }
        self.memory_only(address, 4)?;
        self.memory.read32(address)
    }
    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
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
                WordDevice::Timer(system) => devices.timers.write(system, offset, value),
                WordDevice::Interrupt(system) => devices.interrupts.write(system, offset, value),
            };
            devices.sync_interrupts();
            return result.map_err(|kind| BusError { address, kind });
        }
        self.memory_only(address, 4)?;
        self.memory.write32_masked(address, value, mask)
    }
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        self.memory_only(address, 8)?;
        self.memory.read_doubleword(address)
    }
    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        self.memory_only(address, 8)?;
        self.memory.write_doubleword(address, words)
    }
    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        self.memory_only(address, 4)?;
        self.memory.swap32(address, value)
    }
    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        self.memory_only(address, 1)?;
        self.memory.ldstub8(address)
    }
}
