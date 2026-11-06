//! NCR89C100 DMA2 SCSI register interface. See docs/devices.md.
//! No SCSI DMA request or FIFO transfer engine is implemented yet.
use crate::bus::BusErrorKind;

const ID: u32 = 0xa000_0000;
const RESET: u32 = 1 << 7;
const ENABLE: u32 = 1 << 9;
const COUNT: u32 = 1 << 13;
const NEXT: u32 = 1 << 24;
const CONTROL_MASK: u32 = 0x01ff_2390;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScsiDma {
    control: u32,
    address: u32,
    count: u32,
    next_address: u32,
    next_count: u32,
    loaded: bool,
    next_loaded: bool,
    slave_error: bool,
    scsi_irq: bool,
}

impl ScsiDma {
    pub fn reset(&mut self) {
        // Address/count reset values are indeterminate in the manual. Choose 0.
        *self = Self::default();
    }

    pub fn reset_asserted(&self) -> bool {
        self.control & RESET != 0
    }

    pub fn set_scsi_irq(&mut self, asserted: bool) {
        self.scsi_irq = asserted && !self.reset_asserted();
    }

    pub fn irq(&self) -> bool {
        self.scsi_irq && self.control & 0x10 != 0
    }

    pub(crate) fn slave_access_error(&mut self) {
        self.slave_error = true;
    }

    pub fn read(&self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => Ok(ID
                | self.control
                | u32::from(self.scsi_irq)
                | (u32::from(self.slave_error) << 6)
                | (u32::from(self.loaded && self.control & ENABLE != 0) << 25)
                | (u32::from(self.loaded) << 26)
                | (u32::from(self.next_loaded) << 27)),
            4 => Ok(self.address),
            8 => Ok(self.count),
            _ => Err(BusErrorKind::Unsupported("SCSI DMA diagnostic register")),
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
                if value & NEXT != 0 && value & COUNT == 0 {
                    return Err(BusErrorKind::Unsupported("SCSI DMA NEXT without count"));
                }
                if value & 0x000c_0000 == 0x000c_0000 {
                    return Err(BusErrorKind::Unsupported("reserved SCSI DMA burst mode"));
                }
                if value & 0x0060_0000 == 0x0060_0000 {
                    return Err(BusErrorKind::Unsupported(
                        "conflicting SCSI DMA cycle modes",
                    ));
                }
                self.control = value & CONTROL_MASK;
                if value & 0x40 != 0 {
                    self.slave_error = false;
                }
                if value & NEXT == 0 {
                    self.next_loaded = false;
                }
                // D_INVALIDATE is a pulse, not stored. With an empty FIFO it
                // completes immediately; in chaining mode it invalidates both
                // address latches. It does not acknowledge the SCSI IRQ input.
                if value & (NEXT | 0x20) == NEXT | 0x20 {
                    self.loaded = false;
                    self.next_loaded = false;
                }
            }
            4 => {
                if self.reset_asserted() {
                    return Ok(());
                }
                if self.control & NEXT != 0 {
                    self.next_address = value;
                    self.next_loaded = true;
                    if !self.loaded {
                        self.address = self.next_address;
                        self.count = self.next_count;
                        self.loaded = true;
                        self.next_loaded = false;
                    }
                } else {
                    self.address = value;
                    self.loaded = true;
                }
            }
            8 => {
                if self.reset_asserted() {
                    return Ok(());
                }
                if self.control & NEXT != 0 {
                    self.next_count = value & 0x00ff_ffff;
                } else {
                    self.count = value & 0x00ff_ffff;
                }
            }
            _ => return Err(BusErrorKind::Unsupported("SCSI DMA diagnostic register")),
        }
        Ok(())
    }
}
