//! microSPARC-II SBus/IOMMU controls. See docs/io-control.md.
use crate::bus::{BusErrorKind, IoTlbFlush};
use crate::dma::{DmaError, DmaFault};
use std::collections::VecDeque;

pub const IO_CONTROL_SIZE: u64 = 0x8000;
const VERSION: u32 = 0x0400_0000;
const BASE_MASK: u32 = 0x07ff_fc00;
const ARBITRATION_MASK: u32 = 0x003f_0000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IoControl {
    control: u32,
    table_base: u32,
    slots: [u32; 5],
    arbitration: u32,
    flushes: VecDeque<IoTlbFlush>,
}

pub(crate) enum IoMapping {
    Bypass(u64),
    PageTable(u64),
}

impl IoControl {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn read(&self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => Ok(VERSION | self.control),
            4 => Ok(self.table_base),
            0x1010 | 0x1014 | 0x1018 | 0x101c | 0x1020 => {
                Ok(self.slots[((offset - 0x1010) / 4) as usize])
            }
            0x2000 => Ok(self.arbitration | 8), // immutable MID=8
            _ => Err(Self::unsupported_register(offset)),
        }
    }

    pub fn write(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match offset {
            0 => self.control = value & 0x1d,
            0x14 => {
                self.flushes.clear();
                self.flushes.push_back(IoTlbFlush::All);
            }
            0x18 => {
                let flush = IoTlbFlush::Page(value & !0xfff);
                if !self.flushes.contains(&IoTlbFlush::All) && !self.flushes.contains(&flush) {
                    self.flushes.push_back(flush);
                }
            }
            4 => self.table_base = value & BASE_MASK,
            0x1010 | 0x1014 | 0x1018 | 0x101c | 0x1020 => {
                let slot = ((offset - 0x1010) / 4) as usize;
                // SSCR4 serves masters 4 and 5; both must use translation.
                let mask = if slot == 4 { 2 } else { 0x0001_0003 };
                self.slots[slot] = value & mask;
            }
            // Figure 5.23 and the six-master arbiter description agree on
            // bits 21:16; the text's reserved-bit list has an off-by-one typo.
            0x2000 => self.arbitration = value & ARBITRATION_MASK,
            _ => return Err(Self::unsupported_register(offset)),
        }
        Ok(())
    }

    pub(crate) fn take_flush(&mut self) -> Option<IoTlbFlush> {
        self.flushes.pop_front()
    }

    pub(crate) fn mapping(&self, master: u8, address: u32) -> Result<IoMapping, DmaError> {
        if master >= 6 {
            return Err(DmaError::Unsupported("SBus master index must be 0..5"));
        }
        if self.arbitration & (1 << (16 + master)) == 0 {
            return Err(DmaError::NotGranted { master });
        }
        let slot = self.slots[usize::from(master.min(4))];
        if slot & 1 != 0 && address >> 30 == 0 {
            let physical = u64::from(address) | (u64::from(slot & 0x10000) << 14);
            // Section 5.8 restricts bypass to main memory (PA[30:28]=000).
            if physical >= 0x1000_0000 {
                return Err(DmaError::Fault {
                    address,
                    kind: DmaFault::InvalidBypassAddress,
                });
            }
            return Ok(IoMapping::Bypass(physical));
        }
        if self.control & 1 == 0 {
            // The manual specifies enabled translation and per-slot bypass;
            // do not invent an identity mapping for disabled, non-bypass DMA.
            return Err(DmaError::Unsupported(
                "DMA with IOMMU disabled and no slot bypass",
            ));
        }
        let range = (self.control >> 2) & 7;
        let mask = (1u32 << (24 + range)) - 1;
        if address & !mask != !mask {
            return Err(DmaError::Fault {
                address,
                kind: DmaFault::OutOfRange,
            });
        }
        // Concatenate IBA and the page index, ignoring base bits within the
        // selected table size; addition would carry a misaligned base instead.
        let table_mask = u64::from(mask >> 10);
        let base = (u64::from(self.table_base) << 4) & !table_mask;
        Ok(IoMapping::PageTable(
            base | (u64::from((address & mask) >> 12) * 4),
        ))
    }

    fn unsupported_register(offset: u64) -> BusErrorKind {
        match offset {
            0x14 | 0x18 => BusErrorKind::Unsupported("read of write-only I/O TLB flush port"),
            0x1000 | 0x1004 => BusErrorKind::Unsupported("asynchronous I/O fault register"),
            0x1050 | 0x1054 => BusErrorKind::Unsupported("memory fault capture register"),
            0x3000 | 0x3004 | 0x3008 | 0x300c | 0x3010 | 0x3014 | 0x3018 | 0x301c => {
                BusErrorKind::Unsupported("microSPARC-II performance/breakpoint register")
            }
            0x4000 | 0x6000 | 0x7000 => {
                BusErrorKind::Unsupported("microSPARC-II local graphics queue register")
            }
            _ => BusErrorKind::Unmapped,
        }
    }
}
