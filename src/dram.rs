//! SS-5 DSIMMs: eight fixed 32 MiB apertures, each empty, 8 MiB or 32 MiB.
//! Empty-bank data is unspecified by hardware; use zero without parity injection.

use crate::bus::{Bus, BusError, BusErrorKind, MapError, MemoryBus};

pub const BANK_BYTES: usize = 32 * 1024 * 1024;
pub const APERTURE_BYTES: u64 = 8 * BANK_BYTES as u64;
const SMALL_BANK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct Ss5Dram {
    sizes: [usize; 8],
    memory: MemoryBus,
}

impl Ss5Dram {
    /// Explicit slot order, with 0 for an empty slot. Construction is atomic.
    pub fn new(sizes: [usize; 8]) -> Result<Self, MapError> {
        if sizes
            .iter()
            .any(|&size| !matches!(size, 0 | SMALL_BANK_BYTES | BANK_BYTES))
        {
            return Err(MapError::InvalidRange);
        }
        let mut memory = MemoryBus::default();
        for (slot, &size) in sizes.iter().enumerate() {
            if size != 0 {
                memory.map_ram((slot * BANK_BYTES) as u64, size)?;
            }
        }
        Ok(Self { sizes, memory })
    }

    /// Fill low slots first with 32 MiB DSIMMs, then 8 MiB DSIMMs.
    pub fn from_total(bytes: usize) -> Result<Self, MapError> {
        let large = bytes / BANK_BYTES;
        let small = (bytes % BANK_BYTES) / SMALL_BANK_BYTES;
        if bytes == 0 || bytes % SMALL_BANK_BYTES != 0 || large + small > 8 {
            return Err(MapError::InvalidRange);
        }
        let mut sizes = [0; 8];
        sizes[..large].fill(BANK_BYTES);
        sizes[large..large + small].fill(SMALL_BANK_BYTES);
        Self::new(sizes)
    }

    pub fn bank_sizes(&self) -> [usize; 8] {
        self.sizes
    }

    fn decode(&self, address: u64, alignment: u64) -> Result<Option<u64>, BusError> {
        if address % alignment != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if address >= APERTURE_BYTES {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unmapped,
            });
        }
        let slot = address as usize / BANK_BYTES;
        let size = self.sizes[slot];
        Ok(if size == 0 {
            None
        } else {
            Some((slot * BANK_BYTES) as u64 | (address & (size as u64 - 1)))
        })
    }

    /// Image imports target real storage within one bank, never mirrors or holes.
    pub fn load_image(&mut self, address: u64, bytes: &[u8]) -> Result<(), BusError> {
        self.memory.load_image(address, bytes)
    }
}

impl Bus for Ss5Dram {
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        match self.decode(address, 4)? {
            Some(physical) => self.memory.read32(physical),
            None => Ok(0),
        }
    }

    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        match self.decode(address, 4)? {
            Some(physical) => self.memory.write32_masked(physical, value, mask),
            None => Ok(()),
        }
    }

    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        match self.decode(address, 8)? {
            Some(physical) => self.memory.read_doubleword(physical),
            None => Ok([0; 2]),
        }
    }

    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        // Aligned doublewords cannot cross a bank or a mirror boundary.
        match self.decode(address, 8)? {
            Some(physical) => self.memory.write_doubleword(physical, words),
            None => Ok(()),
        }
    }

    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        match self.decode(address, 4)? {
            Some(physical) => self.memory.swap32(physical, value),
            None => Ok(0),
        }
    }

    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        match self.decode(address, 1)? {
            Some(physical) => self.memory.ldstub8(physical),
            None => Ok(0),
        }
    }
}
