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

/// A validated DSIMM population, independent of backing storage allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DramLayout {
    sizes: [usize; 8],
    bank_bytes: usize,
}

impl DramLayout {
    /// Validate an explicit SS-5 eight-slot population (8/32 MiB modules).
    pub fn ss5(sizes: [usize; 8]) -> Result<Self, MapError> {
        Self::validate(sizes, BANK_BYTES, &[SMALL_BANK_BYTES, BANK_BYTES])
    }

    /// Validate an explicit SS-20-style eight-slot population (16/32/64 MiB).
    pub fn ss20(sizes: [usize; 8]) -> Result<Self, MapError> {
        Self::validate(
            sizes,
            SS20_BANK_BYTES,
            &[SS20_SMALL_BANK_BYTES, SS20_MID_BANK_BYTES, SS20_BANK_BYTES],
        )
    }

    fn validate(sizes: [usize; 8], bank_bytes: usize, allowed: &[usize]) -> Result<Self, MapError> {
        if sizes
            .iter()
            .any(|&size| size != 0 && !allowed.contains(&size))
        {
            return Err(MapError::InvalidRange);
        }
        Ok(Self { sizes, bank_bytes })
    }

    fn from_total(bytes: usize, bank_bytes: usize, sockets: usize) -> Result<Self, MapError> {
        let modules = match bank_bytes {
            BANK_BYTES => {
                let large = bytes / BANK_BYTES;
                let small = (bytes % BANK_BYTES) / SMALL_BANK_BYTES;
                if bytes == 0 || bytes % SMALL_BANK_BYTES != 0 || large + small > sockets {
                    return Err(MapError::InvalidRange);
                }
                let mut sizes = [0; 8];
                sizes[..large].fill(BANK_BYTES);
                sizes[large..large + small].fill(SMALL_BANK_BYTES);
                sizes
            }
            SS20_BANK_BYTES => {
                let large = bytes / SS20_BANK_BYTES;
                let mid = (bytes % SS20_BANK_BYTES) / SS20_MID_BANK_BYTES;
                let small = (bytes % SS20_MID_BANK_BYTES) / SS20_SMALL_BANK_BYTES;
                if bytes == 0 || bytes % SS20_SMALL_BANK_BYTES != 0 || large + mid + small > sockets
                {
                    return Err(MapError::InvalidRange);
                }
                let mut sizes = [0; 8];
                sizes[..large].fill(SS20_BANK_BYTES);
                sizes[large..large + mid].fill(SS20_MID_BANK_BYTES);
                sizes[large + mid..large + mid + small].fill(SS20_SMALL_BANK_BYTES);
                sizes
            }
            _ => return Err(MapError::InvalidRange),
        };
        Self::validate(
            modules,
            bank_bytes,
            if bank_bytes == BANK_BYTES {
                &[SMALL_BANK_BYTES, BANK_BYTES]
            } else {
                &[SS20_SMALL_BANK_BYTES, SS20_MID_BANK_BYTES, SS20_BANK_BYTES]
            },
        )
    }

    /// Build the largest-module-first SS-5 layout for a total.
    pub fn ss5_total(bytes: usize) -> Result<Self, MapError> {
        Self::from_total(bytes, BANK_BYTES, 8)
    }

    /// Build an SS-20 layout using up to `sockets` DRAM banks.
    /// `sockets == 7` reserves bank 7 for the first VSIMM.
    pub fn ss20_total(bytes: usize, sockets: usize) -> Result<Self, MapError> {
        if sockets > 8 {
            return Err(MapError::InvalidRange);
        }
        Self::from_total(bytes, SS20_BANK_BYTES, sockets)
    }

    /// Build the Axil 320 OBP 2.19 profile using eight 16/64 MiB banks.
    /// That ROM distinguishes 16 MiB from 64 MiB by testing the +16 MiB
    /// mirror; it does not test +32 MiB. The former contiguous 32 MiB
    /// simulation was misreported as 64 MiB. This is a firmware/profile limit,
    /// not a claim about every Axil board or firmware revision.
    pub fn axil_total(bytes: usize) -> Result<Self, MapError> {
        let large = bytes / SS20_BANK_BYTES;
        let small = (bytes % SS20_BANK_BYTES) / SS20_SMALL_BANK_BYTES;
        if bytes == 0 || bytes % SS20_SMALL_BANK_BYTES != 0 || large + small > 8 {
            return Err(MapError::InvalidRange);
        }
        let mut sizes = [0; 8];
        sizes[..large].fill(SS20_BANK_BYTES);
        sizes[large..large + small].fill(SS20_SMALL_BANK_BYTES);
        Self::ss20(sizes)
    }

    pub fn bank_sizes(self) -> [usize; 8] {
        self.sizes
    }

    pub fn bank_bytes(self) -> usize {
        self.bank_bytes
    }

    pub fn total_bytes(self) -> usize {
        self.sizes
            .iter()
            .fold(0, |total, &size| total.saturating_add(size))
    }

    pub fn aperture_bytes(self) -> u64 {
        8 * self.bank_bytes as u64
    }

    /// Totals constructible from the SS-5 module sizes and eight sockets.
    pub fn valid_ss5_totals() -> Vec<usize> {
        let mut totals = Vec::new();
        for large in 0..=8 {
            for small in 0..=(8 - large) {
                let total = large * BANK_BYTES + small * SMALL_BANK_BYTES;
                if total != 0 {
                    totals.push(total);
                }
            }
        }
        totals.sort_unstable();
        totals.dedup();
        totals
    }

    /// Totals constructible from SS-20 module sizes and `sockets` DRAM sockets.
    pub fn valid_ss20_totals(sockets: usize) -> Vec<usize> {
        let mut totals = Vec::new();
        if sockets > 8 {
            return totals;
        }
        for large in 0..=sockets {
            for mid in 0..=(sockets - large) {
                for small in 0..=(sockets - large - mid) {
                    let total = large * SS20_BANK_BYTES
                        + mid * SS20_MID_BANK_BYTES
                        + small * SS20_SMALL_BANK_BYTES;
                    if total != 0 {
                        totals.push(total);
                    }
                }
            }
        }
        totals.sort_unstable();
        totals.dedup();
        totals
    }

    /// Totals usable with the Axil 320 OBP 2.19 profile's 16/64 MiB banks.
    pub fn valid_axil_totals() -> Vec<usize> {
        let mut totals = Vec::new();
        for large in 0..=8 {
            for small in 0..=(8 - large) {
                let total = large * SS20_BANK_BYTES + small * SS20_SMALL_BANK_BYTES;
                if total != 0 {
                    totals.push(total);
                }
            }
        }
        totals.sort_unstable();
        totals.dedup();
        totals
    }

    fn allocate(self) -> Result<Ss5Dram, MapError> {
        let mut memory = MemoryBus::default();
        let mut windows = vec![None; self.aperture_bytes() as usize / WINDOW_BYTES];
        for (slot, &size) in self.sizes.iter().enumerate() {
            if size == 0 {
                continue;
            }
            // On a 32 MiB SS-20 module, PA25 selects the rank and PA24
            // mirrors it. All other modules mirror a single rank.
            let ranks = if self.bank_bytes == SS20_BANK_BYTES && size == SS20_MID_BANK_BYTES {
                2
            } else {
                1
            };
            let rank_size = size / ranks;
            let rank_span = self.bank_bytes / ranks;
            for rank in 0..ranks {
                let base = slot * self.bank_bytes + rank * rank_span;
                let region = memory.map_indexed_ram(base as u64, rank_size)?;
                for offset in (0..rank_span).step_by(WINDOW_BYTES) {
                    windows[(base + offset) / WINDOW_BYTES] = Some(DramWindow {
                        region,
                        first_word: (offset % rank_size) / 4,
                    });
                }
            }
        }
        Ok(Ss5Dram {
            sizes: self.sizes,
            bank_bytes: self.bank_bytes,
            memory,
            windows,
        })
    }
}

// The smallest supported module determines the physical lookup granularity.
const WINDOW_BYTES: usize = SMALL_BANK_BYTES;

#[derive(Clone, Copy, Debug)]
struct DramWindow {
    region: usize,
    first_word: usize,
}

#[derive(Debug)]
pub struct Ss5Dram {
    sizes: [usize; 8],
    bank_bytes: usize,
    memory: MemoryBus,
    windows: Vec<Option<DramWindow>>,
}

impl Ss5Dram {
    /// Explicit slot order, with 0 for an empty slot. Construction is atomic.
    pub fn new(sizes: [usize; 8]) -> Result<Self, MapError> {
        DramLayout::ss5(sizes)?.allocate()
    }

    /// SS-20/Axil DSIMMs: 16, 32 or 64 MiB modules in eight 64 MiB banks.
    pub fn ss20(sizes: [usize; 8]) -> Result<Self, MapError> {
        DramLayout::ss20(sizes)?.allocate()
    }

    /// Allocate the backing storage for an already validated layout.
    pub fn from_layout(layout: DramLayout) -> Result<Self, MapError> {
        layout.allocate()
    }

    /// Fill low slots first with 32 MiB DSIMMs, then 8 MiB DSIMMs.
    pub fn from_total(bytes: usize) -> Result<Self, MapError> {
        DramLayout::ss5_total(bytes)?.allocate()
    }

    /// Fill low slots with 64 MiB, then 32 MiB, then 16 MiB SS-20 DSIMMs.
    pub fn from_ss20_total(bytes: usize) -> Result<Self, MapError> {
        DramLayout::ss20_total(bytes, 8)?.allocate()
    }

    pub(crate) fn diagnostic_digest(&self) -> u64 {
        let mut digest = crate::diagnostic::Digest::new();
        for &size in &self.sizes {
            digest.u64(size as u64);
        }
        digest.u64(self.bank_bytes as u64);
        digest.u64(self.memory.diagnostic_digest());
        digest.finish()
    }

    pub fn layout(&self) -> DramLayout {
        DramLayout {
            sizes: self.sizes,
            bank_bytes: self.bank_bytes,
        }
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

    /// Whether a complete access span lies inside the eight-bank aperture,
    /// including empty banks and module mirrors.
    pub fn contains_aperture(&self, address: u64, size: u64) -> bool {
        size != 0
            && address
                .checked_add(size)
                .is_some_and(|end| end <= self.aperture_bytes())
    }

    /// Whether every bank touched by the span contains installed DSIMM storage.
    /// A mirror within a populated small module counts; an empty bank does not.
    pub fn contains_storage(&self, address: u64, size: u64) -> bool {
        if !self.contains_aperture(address, size) {
            return false;
        }
        let first = (address / self.bank_bytes as u64) as usize;
        let last = ((address + size - 1) / self.bank_bytes as u64) as usize;
        self.sizes[first..=last].iter().all(|&bank| bank != 0)
    }

    /// Resolve the containing word. Aligned doublewords always stay in the
    /// same window. Check the full physical address before narrowing to usize.
    #[inline]
    fn word_location(
        &self,
        address: u64,
        alignment: u64,
    ) -> Result<Option<(usize, usize)>, BusError> {
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
        let byte = address as usize;
        Ok(self.windows[byte / WINDOW_BYTES]
            .map(|window| (window.region, window.first_word + (byte % WINDOW_BYTES) / 4)))
    }

    /// Image imports target real storage within one bank, never mirrors or holes.
    pub fn load_image(&mut self, address: u64, bytes: &[u8]) -> Result<(), BusError> {
        self.memory.load_image(address, bytes)
    }

    /// Read-only word peek for parking-token proof validation.
    pub(crate) fn peek32(&self, address: u64) -> Result<u32, BusError> {
        Ok(match self.word_location(address, 4)? {
            Some((region, word)) => self.memory.ram_words(region)[word],
            None => 0,
        })
    }
}

impl Bus for Ss5Dram {
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        self.peek32(address)
    }

    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        if let Some((region, word)) = self.word_location(address, 4)? {
            let dest = &mut self.memory.ram_words_mut(region)[word];
            *dest = (*dest & !mask) | (value & mask);
        }
        Ok(())
    }

    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        Ok(match self.word_location(address, 8)? {
            Some((region, word)) => {
                let words = self.memory.ram_words(region);
                [words[word], words[word + 1]]
            }
            None => [0; 2],
        })
    }

    fn write_doubleword(&mut self, address: u64, value: [u32; 2]) -> Result<(), BusError> {
        if let Some((region, word)) = self.word_location(address, 8)? {
            self.memory.ram_words_mut(region)[word..word + 2].copy_from_slice(&value);
        }
        Ok(())
    }

    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        Ok(match self.word_location(address, 4)? {
            Some((region, word)) => {
                std::mem::replace(&mut self.memory.ram_words_mut(region)[word], value)
            }
            None => 0,
        })
    }

    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        Ok(match self.word_location(address, 1)? {
            Some((region, word)) => {
                let dest = &mut self.memory.ram_words_mut(region)[word];
                let shift = (3 - (address & 3)) * 8;
                let old = (*dest >> shift) as u8;
                *dest |= 0xff << shift;
                old
            }
            None => 0,
        })
    }
}
