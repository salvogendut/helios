//! microSPARC-II SBus/IOMMU configuration subset. See docs/io-control.md.
//! DVMA translation, fault capture and diagnostic/performance registers are
//! separate facilities; unsupported operations stop before changing state.
use crate::bus::BusErrorKind;

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
            0 => {
                if value & 1 != 0 {
                    return Err(BusErrorKind::Unsupported("IOMMU DVMA translation enable"));
                }
                self.control = value & 0x1c;
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

    fn unsupported_register(offset: u64) -> BusErrorKind {
        match offset {
            0x14 | 0x18 => BusErrorKind::Unsupported("I/O TLB flush register"),
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
