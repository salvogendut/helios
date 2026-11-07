//! NCR89C100 DMA2 Ethernet interface; see docs/ethernet.md.
use crate::bus::BusErrorKind;

const RESET: u32 = 0x80;
const CONTROL_MASK: u32 = 0x007f_9810;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthernetDma {
    control: u32,
    base: u32,
    diagnostic: u32,
    slave_error: bool,
    memory_error: bool,
    mac_irq: bool,
}

impl Default for EthernetDma {
    fn default() -> Self {
        Self {
            control: 0,
            base: 0xff00_0000,
            diagnostic: 0,
            slave_error: false,
            memory_error: false,
            mac_irq: false,
        }
    }
}

impl EthernetDma {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn reset_asserted(&self) -> bool {
        self.control & RESET != 0
    }
    pub fn set_mac_irq(&mut self, asserted: bool) {
        self.mac_irq = asserted && !self.reset_asserted();
    }
    pub fn irq(&self) -> bool {
        (self.mac_irq || self.memory_error) && self.control & 0x10 != 0
    }
    pub(crate) fn slave_access_error(&mut self) {
        self.slave_error = true;
    }
    pub(crate) fn memory_error(&mut self) {
        self.memory_error = true;
    }
    pub(crate) fn address(&self, address: u32) -> u32 {
        self.base | (address & 0x00ff_ffff)
    }
    pub(crate) fn record_address(&mut self, address: u32) {
        self.diagnostic = (self.diagnostic & 0xff00_0000) | (address & 0x00ff_ffff);
    }
    pub fn read(&self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => Ok(0xa000_0000
                | self.control
                | u32::from(self.mac_irq)
                | (u32::from(self.memory_error) << 1)
                | (u32::from(self.slave_error) << 6)),
            // Coherent, unbuffered transfers leave no valid/dirty cache data
            // or outstanding cache request. Retain LRU/DIRTY configuration and
            // the last DMA address (NCR89C100, pages 3-21 and 3-22).
            4 => Ok(self.diagnostic),
            8 => Ok(0),
            12 => Ok(self.base),
            _ => Err(BusErrorKind::Unsupported(
                "Ethernet DMA cache diagnostic register",
            )),
        }
    }
    pub fn write(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match offset {
            0 => {
                if value & RESET != 0 {
                    self.reset();
                    self.control = RESET;
                    return Ok(());
                }
                if value & 0x0008_0000 != 0 {
                    return Err(BusErrorKind::Unsupported(
                        "reserved Ethernet DMA burst mode",
                    ));
                }
                self.control = value & CONTROL_MASK;
                if value & 0x40 != 0 {
                    self.slave_error = false;
                }
                if value & 0x20 != 0 {
                    self.memory_error = false;
                }
                // Transfers are coherent and unbuffered. DRAIN/INVALIDATE
                // complete immediately; there is no dirty cache to drain.
            }
            4 => {
                if value & 0xc000_0000 != 0 {
                    return Err(BusErrorKind::Unsupported(
                        "Ethernet DMA diagnostic tag load/cache fill",
                    ));
                }
                // DRAIN0/1 complete immediately on the empty cache; HIT is
                // read-only. Actual diagnostic cache injection stays explicit.
                self.diagnostic = value & 0x13ff_ffff;
            }
            8 if value == 0 => {} // invalidating an already empty cache
            12 => {
                if !self.reset_asserted() {
                    self.base = value & 0xff00_0000;
                }
            }
            _ => {
                return Err(BusErrorKind::Unsupported(
                    "Ethernet DMA cache diagnostic register",
                ))
            }
        }
        Ok(())
    }
}
