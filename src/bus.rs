//! Word-transparent physical bus. Values are guest bit patterns, never swapped
//! host integers. Narrow accesses select big-endian byte lanes using shifts.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusErrorKind {
    Unmapped,
    /// An installed SBus controller received no slave acknowledgement.
    SBusTimeout,
    /// A selected SBus slave returned an error acknowledgement.
    SBusError,
    Misaligned,
    ReadOnly,
    /// Emulator limitation or host backpressure, never a guest bus exception.
    Unsupported(&'static str),
    WouldBlock,
}

impl BusErrorKind {
    pub fn is_host_stop(self) -> bool {
        matches!(self, Self::Unsupported(_) | Self::WouldBlock)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusError {
    pub address: u64,
    pub kind: BusErrorKind,
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} bus access at {:#x}", self.kind, self.address)
    }
}

impl std::error::Error for BusError {}

/// I/O control writes are consumed by the owner of the shared CPU/I/O TLB
/// before its next access, including diagnostic accesses. Plain RAM buses do
/// not produce these requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoTlbFlush {
    All,
    Page(u32),
}

/// Memory-controller request codes (microSPARC-II table 32). Uncached narrow
/// reads use a 64-bit DRAM read; cache-line fills are not modeled yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryRequestKind {
    Read = 1,
    WriteByte = 9,
    WriteHalf = 10,
    WriteWord = 11,
    WriteDoubleword = 12,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRequest {
    pub kind: MemoryRequestKind,
    pub cpu: bool,
    pub boot_mode: bool,
    pub cacheable: bool,
}

fn aligned(address: u64, alignment: u64) -> Result<(), BusError> {
    if address % alignment == 0 {
        Ok(())
    } else {
        Err(BusError {
            address,
            kind: BusErrorKind::Misaligned,
        })
    }
}

/// A failed access must have no side effects. Masked stores must not perform a
/// read-modify-write through a device's potentially destructive read interface.
/// MMIO devices may override narrow accesses when their register semantics differ.
/// Doubleword transfers and exchanges are indivisible bus operations. Implement
/// them explicitly: generic read-then-write fallbacks can consume MMIO data before
/// discovering a write failure. Future concurrent owners must serialize them.
pub trait Bus {
    /// CPU pin state for module fault capture, independent of alternate ASI
    /// permissions. Debugger/raw bus accesses do not issue memory requests.
    fn set_cpu_supervisor(&mut self, _supervisor: bool) {}

    fn observe_memory_request(&mut self, _address: u64, _request: MemoryRequest) {}

    /// Return true if a failed CPU store was accepted as an asynchronous module
    /// error. A plain physical bus has no module-error controller.
    fn capture_sbus_write_error(&mut self, _error: BusError, _bytes: u8) -> bool {
        false
    }

    fn take_io_tlb_flush(&mut self) -> Option<IoTlbFlush> {
        None
    }

    fn read32(&mut self, address: u64) -> Result<u32, BusError>;
    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError>;
    /// The word at the lower address comes first, with no host endian conversion.
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError>;
    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError>;
    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError>;
    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError>;

    fn write32(&mut self, address: u64, value: u32) -> Result<(), BusError> {
        self.write32_masked(address, value, u32::MAX)
    }

    fn read8(&mut self, address: u64) -> Result<u8, BusError> {
        let shift = (3 - (address & 3)) * 8;
        self.read32(address & !3)
            .map(|word| (word >> shift) as u8)
            .map_err(|error| BusError { address, ..error })
    }

    fn read16(&mut self, address: u64) -> Result<u16, BusError> {
        aligned(address, 2)?;
        let shift = (2 - (address & 2)) * 8;
        self.read32(address & !3)
            .map(|word| (word >> shift) as u16)
            .map_err(|error| BusError { address, ..error })
    }

    fn write8(&mut self, address: u64, value: u8) -> Result<(), BusError> {
        let shift = (3 - (address & 3)) * 8;
        self.write32_masked(address & !3, u32::from(value) << shift, 0xff << shift)
            .map_err(|error| BusError { address, ..error })
    }

    fn write16(&mut self, address: u64, value: u16) -> Result<(), BusError> {
        aligned(address, 2)?;
        let shift = (2 - (address & 2)) * 8;
        self.write32_masked(address & !3, u32::from(value) << shift, 0xffff << shift)
            .map_err(|error| BusError { address, ..error })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    InvalidRange,
    Overlap,
    AllocationFailed,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRange => "memory range must be nonempty, word aligned, and not overflow",
            Self::Overlap => "memory range overlaps an existing mapping",
            Self::AllocationFailed => "cannot allocate guest memory",
        })
    }
}

impl std::error::Error for MapError {}

#[derive(Debug)]
struct Region {
    base: u64,
    end: u64,
    words: Vec<u32>,
    read_only: bool,
}

#[derive(Debug, Default)]
pub struct MemoryBus {
    regions: Vec<Region>,
}

impl MemoryBus {
    pub(crate) fn overlaps(&self, start: u64, end: u64) -> bool {
        self.regions.iter().any(|r| start < r.end && end > r.base)
    }
    pub fn map_ram(&mut self, base: u64, size: usize) -> Result<(), MapError> {
        self.map(base, size, false)
    }

    /// Import a raw big-endian image at the file/memory boundary.
    pub fn map_rom(&mut self, base: u64, bytes: &[u8]) -> Result<(), MapError> {
        self.map(base, bytes.len(), true)?;
        let region = self.regions.last_mut().expect("mapping just inserted");
        for (word, bytes) in region.words.iter_mut().zip(bytes.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().expect("four-byte chunk"));
        }
        Ok(())
    }

    fn map(&mut self, base: u64, size: usize, read_only: bool) -> Result<(), MapError> {
        let end = base
            .checked_add(size as u64)
            .ok_or(MapError::InvalidRange)?;
        if size == 0 || base % 4 != 0 || size % 4 != 0 {
            return Err(MapError::InvalidRange);
        }
        if self.regions.iter().any(|r| base < r.end && r.base < end) {
            return Err(MapError::Overlap);
        }
        let mut words = Vec::new();
        words
            .try_reserve_exact(size / 4)
            .map_err(|_| MapError::AllocationFailed)?;
        words.resize(size / 4, 0);
        self.regions.push(Region {
            base,
            end,
            words,
            read_only,
        });
        Ok(())
    }

    /// Load external guest bytes into a single RAM mapping. Check the entire
    /// range before changing memory, including images with partial final words.
    pub fn load_image(&mut self, address: u64, bytes: &[u8]) -> Result<(), BusError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let error = BusError {
            address,
            kind: BusErrorKind::Unmapped,
        };
        let end = address.checked_add(bytes.len() as u64).ok_or(error)?;
        let region = self
            .regions
            .iter_mut()
            .find(|r| address >= r.base && end <= r.end)
            .ok_or(error)?;
        if region.read_only {
            return Err(BusError {
                address,
                kind: BusErrorKind::ReadOnly,
            });
        }
        let offset = (address - region.base) as usize;
        for (i, byte) in bytes.iter().enumerate() {
            let at = offset + i;
            let shift = (3 - (at & 3)) * 8;
            let word = &mut region.words[at / 4];
            *word = (*word & !(0xff << shift)) | (u32::from(*byte) << shift);
        }
        Ok(())
    }

    fn region_mut(&mut self, address: u64) -> Result<&mut Region, BusError> {
        aligned(address, 4)?;
        self.regions
            .iter_mut()
            .find(|r| address >= r.base && address < r.end)
            .ok_or(BusError {
                address,
                kind: BusErrorKind::Unmapped,
            })
    }
}

impl Bus for MemoryBus {
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        aligned(address, 8)?;
        // This concrete bus contains only RAM/ROM: reads have no side effects.
        Ok([self.read32(address)?, self.read32(address + 4)?])
    }

    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        aligned(address, 8)?;
        // Validate both words, including separate adjacent mappings, before
        // changing either. Mappings cannot change while this bus is borrowed.
        for word_address in [address, address + 4] {
            if self.region_mut(word_address)?.read_only {
                return Err(BusError {
                    address: word_address,
                    kind: BusErrorKind::ReadOnly,
                });
            }
        }
        self.write32(address, words[0])?;
        self.write32(address + 4, words[1])
    }

    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        aligned(address, 4)?;
        let region = self.region_mut(address)?;
        if region.read_only {
            return Err(BusError {
                address,
                kind: BusErrorKind::ReadOnly,
            });
        }
        let word = &mut region.words[((address - region.base) / 4) as usize];
        Ok(std::mem::replace(word, value))
    }

    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        let region = self
            .region_mut(address & !3)
            .map_err(|error| BusError { address, ..error })?;
        if region.read_only {
            return Err(BusError {
                address,
                kind: BusErrorKind::ReadOnly,
            });
        }
        let word = &mut region.words[((address - region.base) / 4) as usize];
        let shift = (3 - (address & 3)) * 8;
        let old = (*word >> shift) as u8;
        *word |= 0xff << shift;
        Ok(old)
    }

    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        let region = self.region_mut(address)?;
        Ok(region.words[((address - region.base) / 4) as usize])
    }

    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        let region = self.region_mut(address)?;
        if region.read_only {
            return Err(BusError {
                address,
                kind: BusErrorKind::ReadOnly,
            });
        }
        let word = &mut region.words[((address - region.base) / 4) as usize];
        *word = (*word & !mask) | (value & mask);
        Ok(())
    }
}
