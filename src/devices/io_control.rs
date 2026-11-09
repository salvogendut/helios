//! microSPARC-II SBus/IOMMU controls. See docs/io-control.md.
use crate::bus::{BusError, BusErrorKind, IoTlbFlush, MemoryRequest};
use crate::dma::{DmaError, DmaFault};
use std::collections::VecDeque;

pub const IO_CONTROL_SIZE: u64 = 0x8000;
const VERSION: u32 = 0x0400_0000;
const BASE_MASK: u32 = 0x07ff_fc00;
const ARBITRATION_MASK: u32 = 0x003f_0000;
// microSPARC-II manual table 38: revision 3.0. This is a model choice,
// independent of the IOCR version, not QEMU's turboSPARC identity.
const MASK_ID: u32 = 0x3000_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoControl {
    version: u32,
    control: u32,
    table_base: u32,
    slots: [u32; 5],
    arbitration: u32,
    flushes: VecDeque<IoTlbFlush>,
    async_status: u32,
    async_address: u32,
    async_locked: bool,
    memory_status: u32,
    memory_address: u32,
    memory_status_locked: bool,
    memory_address_locked: bool,
    virtual_address_mask: u32,
    virtual_address_compare: u32,
    arbiter_enable: u32,
}

impl Default for IoControl {
    fn default() -> Self {
        Self::new(VERSION)
    }
}

pub(crate) enum IoMapping {
    Bypass(u64),
    PageTable(u64),
}

impl IoControl {
    /// `version` is the implementation/version field of the control register:
    /// microSPARC-II reports 0x0400_0000; the sun4m SS-20/Axil IOMMU 0x1300_0000.
    pub const fn new(version: u32) -> Self {
        Self {
            version,
            control: 0,
            table_base: 0,
            slots: [0; 5],
            arbitration: 0,
            flushes: VecDeque::new(),
            async_status: 0,
            async_address: 0,
            async_locked: false,
            memory_status: 0,
            memory_address: 0,
            memory_status_locked: false,
            memory_address_locked: false,
            virtual_address_mask: 0,
            virtual_address_compare: 0,
            arbiter_enable: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.version);
    }

    pub fn read(&mut self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => Ok(self.version | self.control),
            4 => Ok(self.table_base),
            0x1000 => {
                self.async_locked = false;
                Ok(self.async_status | 0x0080_0000)
            }
            0x1004 => Ok(self.async_address),
            0x1008 => Ok(self.arbiter_enable | 1), // MBus master 8 always enabled

            0x1050 => {
                self.memory_status_locked = false;
                Ok(self.memory_status)
            }
            0x1054 => {
                self.memory_address_locked = false;
                Ok(self.memory_address)
            }
            0x1010 | 0x1014 | 0x1018 | 0x101c | 0x1020 => {
                Ok(self.slots[((offset - 0x1010) / 4) as usize])
            }
            0x2000 => Ok(self.arbitration | 8), // immutable MID=8
            0x3018 => Ok(MASK_ID | self.virtual_address_mask),
            0x301c => Ok(self.virtual_address_compare),
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
            0x1000 => {
                self.async_status = value & 0xff0e_0000;
                self.async_locked = value & (1 << 31) != 0;
            }
            0x1004 => self.async_address = value & 0x7fff_ffff,
            0x1008 => self.arbiter_enable = value & 0x801f_000f,
            0x1050 => {
                self.memory_status = value & 0x8188_f8f0;
                self.memory_status_locked = value & (1 << 31) != 0;
                self.memory_address_locked = self.memory_status_locked;
            }
            0x1054 => self.memory_address = value & 0x7fff_ffff,
            0x1010 | 0x1014 | 0x1018 | 0x101c | 0x1020 => {
                let slot = ((offset - 0x1010) / 4) as usize;
                // SSCR4 serves masters 4 and 5; both must use translation.
                let mask = if slot == 4 { 2 } else { 0x0001_0003 };
                self.slots[slot] = value & mask;
            }
            // Figure 5.23 and the six-master arbiter description agree on
            // bits 21:16; the text's reserved-bit list has an off-by-one typo.
            0x2000 => self.arbitration = value & ARBITRATION_MASK,
            // Figure 5.30: ID[31:24] read-only, [23:11] reserved,
            // comparison enables/inversions [10:0] writable.
            0x3018 => self.virtual_address_mask = value & 0x7ff,
            0x301c => self.virtual_address_compare = value,
            _ => return Err(Self::unsupported_register(offset)),
        }
        Ok(())
    }

    pub(crate) fn take_flush(&mut self) -> Option<IoTlbFlush> {
        self.flushes.pop_front()
    }

    pub(crate) fn memory_request(
        &mut self,
        address: u64,
        request: MemoryRequest,
        supervisor: bool,
    ) {
        if !self.memory_status_locked {
            self.memory_status = (u32::from(supervisor) << 24)
                | (u32::from(request.cpu) << 23)
                | (u32::from(request.boot_mode) << 12)
                | (u32::from(request.cacheable) << 11)
                | ((request.kind as u32) << 4);
        }
        if !self.memory_address_locked {
            self.memory_address = address as u32 & 0x7fff_ffff;
        }
    }

    pub(crate) fn sbus_write_error(&mut self, error: BusError, bytes: u8, supervisor: bool) {
        if self.async_locked {
            self.async_status |= 1 << 19; // retain first address/cause, report multiple errors
            return;
        }
        let size = match bytes {
            1 => 1,
            2 => 2,
            4 => 0,
            _ => unreachable!("plain SBus store size"),
        };
        let cause = match error.kind {
            BusErrorKind::SBusTimeout => 1 << 29,
            BusErrorKind::SBusError => 1 << 28,
            _ => unreachable!("SBus hardware error"),
        };
        self.async_status =
            (1 << 31) | cause | (size << 25) | (u32::from(supervisor) << 24) | (1 << 17);
        self.async_address = error.address as u32 & 0x7fff_ffff;
        self.async_locked = true;
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
            0x3000 | 0x3004 | 0x3008 | 0x300c | 0x3010 | 0x3014 => {
                BusErrorKind::Unsupported("microSPARC-II performance/breakpoint register")
            }
            0x4000 | 0x6000 | 0x7000 => {
                BusErrorKind::Unsupported("microSPARC-II local graphics queue register")
            }
            _ => BusErrorKind::Unmapped,
        }
    }
}
