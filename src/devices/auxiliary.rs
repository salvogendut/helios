//! NCR89C105 auxiliary ports, figures 6-21 and 6-22.
//! Pin levels are functional state; a terminal-count event has no bus-cycle duration.
use crate::bus::BusErrorKind;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Auxiliary {
    outputs: u8,
    high_density: bool,
    terminal_count_pulses: u64,
    power_scratch: u8,
}

impl Auxiliary {
    pub fn reset(&mut self) {
        self.outputs = 0;
        self.terminal_count_pulses = 0;
        self.power_scratch = 0;
        // Density is an external input, not a resettable output latch.
    }

    pub fn set_floppy_high_density(&mut self, high_density: bool) {
        self.high_density = high_density;
    }

    pub fn read(&self) -> u8 {
        self.outputs | (u8::from(self.high_density) << 5)
    }

    pub fn write(&mut self, value: u8) {
        // E=link test, M=monitor/mouse mux, L=front-panel LED.
        self.outputs = value & 0x0d;
        if value & 2 != 0 {
            self.terminal_count_pulses = self.terminal_count_pulses.saturating_add(1);
        }
    }

    /// Consume generated TC pulses. The floppy controller connection is pending.
    /// TC always reads zero; each write of one generates a separate pulse.
    pub fn take_terminal_count_pulses(&mut self) -> u64 {
        std::mem::take(&mut self.terminal_count_pulses)
    }

    /// AuxIO2: stable power supply, with no external power-failure input yet.
    pub fn read_power(&self) -> u8 {
        self.power_scratch
    }

    pub fn write_power(&mut self, value: u8) -> Result<(), BusErrorKind> {
        if value & 1 != 0 {
            return Err(BusErrorKind::Unsupported("system power-off request"));
        }
        // Bits 3:2 are writable but have no effect. Bit 1 acknowledges power
        // failure; with no asserted input it has nothing to clear. Other
        // reserved bits read zero and cannot fabricate a power failure.
        self.power_scratch = value & 0x0c;
        Ok(())
    }
}
