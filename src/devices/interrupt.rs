//! Uniprocessor NCR89C105 interrupt controller. See docs/interrupts-timers.md.
use crate::bus::BusErrorKind;

pub const MASK_ALL: u32 = 1 << 31;
pub const MASK_BITS: u32 = 0xf05d_ff80;
const SOFT_BITS: u32 = 0xfffe_0000;
const MODULE_ERROR: u32 = 1 << 30;

/// Level-sensitive inputs supported by the current sun4m wiring contract.
/// Module errors use a separate latch, acknowledged by processor IC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Source {
    SBus1 = 7,
    SBus2 = 8,
    SBus3 = 9,
    SBus4 = 10,
    SBus5 = 11,
    SBus6 = 12,
    SBus7 = 13,
    Keyboard = 14,
    Serial = 15,
    Ethernet = 16,
    Scsi = 18,
    SystemTimer = 19,
    Video = 20,
    Floppy = 22,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterruptController {
    sources: u32,
    soft: u32,
    mask: u32,
    cpu_timer: bool,
    levels: u16,
}

impl Default for InterruptController {
    fn default() -> Self {
        Self {
            sources: 0,
            soft: 0,
            mask: MASK_BITS,
            cpu_timer: false,
            levels: 0,
        }
    }
}

impl InterruptController {
    pub(crate) fn latch_module_error(&mut self) {
        if self.sources & MODULE_ERROR == 0 {
            self.sources |= MODULE_ERROR;
            self.recompute();
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_source(&mut self, source: Source, asserted: bool) {
        let bit = 1u32 << source as u8;
        let sources = (self.sources & !bit) | if asserted { bit } else { 0 };
        if sources != self.sources {
            self.sources = sources;
            self.recompute();
        }
    }

    pub fn set_cpu_timer(&mut self, asserted: bool) {
        if asserted != self.cpu_timer {
            self.cpu_timer = asserted;
            self.recompute();
        }
    }

    fn recompute(&mut self) {
        self.levels = if self.mask & MASK_ALL != 0 {
            0
        } else {
            Self::hardware_levels(self.sources & !self.mask)
                | if self.cpu_timer { 1 << 14 } else { 0 }
                | (self.soft >> 16) as u16
        };
    }

    fn hardware_levels(sources: u32) -> u16 {
        // NCR89C105 table 6-24. Independent source bits can share a CPU level.
        let mut levels = 0;
        for (bit, level) in [
            (7, 2),
            (8, 3),
            (9, 5),
            (10, 7),
            (11, 9),
            (12, 11),
            (13, 13),
            (14, 12),
            (15, 12),
            (16, 6),
            (18, 4),
            (19, 10),
            (20, 8),
            (22, 11),
            (30, 15),
        ] {
            if sources & (1 << bit) != 0 {
                levels |= 1 << level;
            }
        }
        levels
    }

    /// Pending display is independent of masks. Mask All gates delivery,
    /// including software requests (NCR's documented "all interrupts" rule).
    pub fn levels(&self) -> u16 {
        self.levels
    }

    pub fn read(&self, system: bool, offset: u64) -> Result<u32, BusErrorKind> {
        if system {
            match offset {
                0 => Ok(self.sources),
                4 => Ok(self.mask),
                8 | 12 | 16 => Ok(0), // write-only aliases; no MP target on NCR89C105
                _ => Err(BusErrorKind::Unsupported("interrupt register offset")),
            }
        } else {
            match offset {
                0 => Ok(self.soft
                    | u32::from(Self::hardware_levels(self.sources))
                    | if self.cpu_timer { 1 << 14 } else { 0 }),
                4 | 8 | 12 => Ok(0),
                _ => Err(BusErrorKind::Unsupported("interrupt register offset")),
            }
        }
    }

    pub fn write(&mut self, system: bool, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        if system {
            match offset {
                8 => {
                    self.mask &= !(value & MASK_BITS);
                    self.recompute();
                }
                12 => {
                    self.mask |= value & MASK_BITS;
                    if value & MODULE_ERROR != 0 {
                        self.sources &= !MODULE_ERROR;
                    }
                    self.recompute();
                }
                0 | 4 | 16 => {} // read-only or reserved
                _ => return Err(BusErrorKind::Unsupported("interrupt register offset")),
            }
        } else {
            match offset {
                4 => {
                    self.soft &= !(value & SOFT_BITS);
                    if value & (1 << 15) != 0 {
                        self.sources &= !MODULE_ERROR;
                    }
                    self.recompute();
                }
                8 => {
                    self.soft |= value & SOFT_BITS;
                    self.recompute();
                }
                0 | 12 => {}
                _ => return Err(BusErrorKind::Unsupported("interrupt register offset")),
            }
        }
        Ok(())
    }
}
