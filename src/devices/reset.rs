//! NCR89C105 system control/status, figure 6-6. See docs/reset.md.

pub const SOFTWARE_RESET: u32 = 1;
pub const RESET_STATUS: u32 = 1 << 1;
pub const WATCHDOG_STATUS: u32 = 1 << 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResetControl {
    status: u32,
    requested: bool,
}

impl ResetControl {
    pub fn read(&self) -> u32 {
        self.status
    }

    pub fn write(&mut self, value: u32) {
        // Status bits are read/clear: zero clears, one cannot set them.
        self.status &= value;
        // Once asserted, SR cannot be canceled by another bus write.
        self.requested |= value & SOFTWARE_RESET != 0;
    }

    pub fn requested(&self) -> bool {
        self.requested
    }

    pub fn power_on_reset(&mut self) {
        *self = Self::default();
    }

    pub fn software_reset(&mut self) {
        self.status = RESET_STATUS;
        self.requested = false;
    }

    /// The controller only latches IU_ERROR; it does not reset the SBus.
    pub fn latch_watchdog(&mut self) {
        self.status |= WATCHDOG_STATUS;
    }
}
