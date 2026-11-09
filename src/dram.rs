//! Sun4m DSIMM arrays: eight fixed apertures. SS-5 uses 32 MiB banks with
//! 8/32 MiB modules; the SS-20 profile uses 64 MiB banks with 16/32/64 MiB
//! modules. Empty-bank data is unspecified by hardware; use zero without
//! parity/ECC injection.

use crate::bus::{Bus, BusError, BusErrorKind, MapError, MemoryBus};

pub const BANK_BYTES: usize = 32 * 1024 * 1024;
pub const APERTURE_BYTES: u64 = 8 * BANK_BYTES as u64;
const SMALL_BANK_BYTES: usize = 8 * 1024 * 1024;
pub const SS20_BANK_BYTES: usize = 64 * 1024 * 1024;
const SS20_SMALL_BANK_BYTES: usize = 16 * 1024 * 1024;
const SS20_MID_BANK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug)]
pub struct Ss5Dram {
    sizes: [usize; 8],
    bank_bytes: usize,
    memory: MemoryBus,
}

impl Ss5Dram {
    /// Explicit slot order, with 0 for an empty slot. Construction is atomic.
    pub fn new(sizes: [usize; 8]) -> Result<Self, MapError> {
        Self::with_bank(sizes, BANK_BYTES, &[SMALL_BANK_BYTES, BANK_BYTES])
    }

    /// SS-20/Axil DSIMMs: 16, 32 or 64 MiB modules in eight 64 MiB banks.
    pub fn ss20(sizes: [usize; 8]) -> Result<Self, MapError> {
        Self::with_bank(
            sizes,
            SS20_BANK_BYTES,
            &[SS20_SMALL_BANK_BYTES, SS20_MID_BANK_BYTES, SS20_BANK_BYTES],
        )
    }

    fn with_bank(
        sizes: [usize; 8],
        bank_bytes: usize,
        allowed: &[usize],
    ) -> Result<Self, MapError> {
        if sizes
            .iter()
            .any(|&size| size != 0 && !allowed.contains(&size))
        {
            return Err(MapError::InvalidRange);
        }
        let mut memory = MemoryBus::default();
        for (slot, &size) in sizes.iter().enumerate() {
            if size != 0 {
                memory.map_ram((slot * bank_bytes) as u64, size)?;
            }
        }
        Ok(Self {
            sizes,
            bank_bytes,
            memory,
        })
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

    /// Fill low slots with 64 MiB, then 32 MiB, then 16 MiB SS-20 DSIMMs.
    pub fn from_ss20_total(bytes: usize) -> Result<Self, MapError> {
        let large = bytes / SS20_BANK_BYTES;
        let mid = (bytes % SS20_BANK_BYTES) / SS20_MID_BANK_BYTES;
        let small = (bytes % SS20_MID_BANK_BYTES) / SS20_SMALL_BANK_BYTES;
        if bytes == 0 || bytes % SS20_SMALL_BANK_BYTES != 0 || large + mid + small > 8 {
            return Err(MapError::InvalidRange);
        }
        let mut sizes = [0; 8];
        sizes[..large].fill(SS20_BANK_BYTES);
        sizes[large..large + mid].fill(SS20_MID_BANK_BYTES);
        sizes[large + mid..large + mid + small].fill(SS20_SMALL_BANK_BYTES);
        Self::ss20(sizes)
    }

    pub fn bank_sizes(&self) -> [usize; 8] {
        self.sizes
    }

    pub fn bank_bytes(&self) -> usize {
        self.bank_bytes
    }

    pub fn aperture_bytes(&self) -> u64 {
        8 * self.bank_bytes as u64
    }

    fn decode(&self, address: u64, alignment: u64) -> Result<Option<u64>, BusError> {
        if address % alignment != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if address >= self.aperture_bytes() {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unmapped,
            });
        }
        let slot = address as usize / self.bank_bytes;
        let size = self.sizes[slot];
        Ok(if size == 0 {
            None
        } else {
            Some((slot * self.bank_bytes) as u64 | (address & (size as u64 - 1)))
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
