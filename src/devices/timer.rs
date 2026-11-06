//! NCR89C105 counters and 54-bit processor user timer, clocked every 500 ns.
//! Counter arithmetic is bounded even for very large explicit time advances.
use crate::bus::BusErrorKind;

const MODULUS: u128 = 1 << 22;
const FIELD: u32 = 0x7fff_fe00;
const USER_MODULUS: u128 = 1 << 54;
pub const TICK_NS: u64 = 500;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Counter {
    count: u32,
    limit: u32,
    pending: bool,
}

impl Counter {
    fn advance(&mut self, ticks: u128) {
        if ticks == 0 {
            return;
        }
        // Sun-4M section 5.3.2: overflow reloads 1, skipping zero.
        // Therefore a zero limit free-runs without matching or raising IRQ.
        let span = MODULUS - 1;
        if self.limit == 0 {
            self.count = (1 + (u128::from(self.count) + ticks - 1) % span) as u32;
            return;
        }
        let distance = if self.limit > self.count {
            u128::from(self.limit - self.count)
        } else {
            span - u128::from(self.count) + u128::from(self.limit)
        };
        if ticks < distance {
            self.count = (1 + (u128::from(self.count) + ticks - 1) % span) as u32;
        } else {
            self.pending = true;
            let period = if self.limit == 1 {
                span
            } else {
                u128::from(self.limit - 1)
            };
            self.count = (1 + (ticks - distance) % period) as u32;
        }
    }

    fn read(&mut self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => {
                let value = (self.limit << 9) | if self.pending { 1 << 31 } else { 0 };
                self.pending = false;
                Ok(value)
            }
            4 => Ok((self.count << 9) | if self.pending { 1 << 31 } else { 0 }),
            8 => Ok(0), // write-only non-resetting limit port
            _ => Err(BusErrorKind::Unsupported("timer register offset")),
        }
    }

    fn write(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match offset {
            0 => {
                self.limit = (value & FIELD) >> 9;
                self.count = 1;
                self.pending = false;
            }
            4 => {} // counter is read-only in counter mode
            8 => self.limit = (value & FIELD) >> 9,
            _ => return Err(BusErrorKind::Unsupported("timer register offset")),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UserTimer {
    count: u64,
    overflow: bool,
}

impl UserTimer {
    fn advance(&mut self, ticks: u128) {
        let total = u128::from(self.count) + ticks;
        self.overflow |= total >= USER_MODULUS;
        self.count = (total % USER_MODULUS) as u64;
    }

    fn value(&self) -> u64 {
        (self.count << 9) | if self.overflow { 1 << 63 } else { 0 }
    }

    fn write(&mut self, value: u64) {
        self.count = (value & 0x7fff_ffff_ffff_fe00) >> 9;
        self.overflow = false;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timers {
    system: Counter,
    processor: Counter,
    phase_ns: u16,
    user_run: bool,
    user_mode: bool,
    user: UserTimer,
}

impl Timers {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn system_irq(&self) -> bool {
        self.system.pending
    }
    pub fn processor_irq(&self) -> bool {
        !self.user_mode && self.processor.pending
    }

    pub fn advance_ns(&mut self, ns: u64) {
        let remainder = ns % TICK_NS + u64::from(self.phase_ns);
        let ticks = ns / TICK_NS + remainder / TICK_NS;
        self.phase_ns = (remainder % TICK_NS) as u16;
        self.advance_ticks(u128::from(ticks));
    }

    pub fn advance_seconds(&mut self, seconds: u64) {
        self.advance_ticks(u128::from(seconds) * 2_000_000);
    }

    fn advance_ticks(&mut self, ticks: u128) {
        self.system.advance(ticks);
        if self.user_mode {
            if self.user_run {
                self.user.advance(ticks);
            }
        } else {
            self.processor.advance(ticks);
        }
    }

    pub fn read(&mut self, system: bool, offset: u64) -> Result<u32, BusErrorKind> {
        match (system, offset) {
            (true, 12) => Ok(0),
            (true, 16) => Ok(u32::from(self.user_mode)),
            (false, 0) if self.user_mode => Ok((self.user.value() >> 32) as u32),
            (false, 4) if self.user_mode => Ok(self.user.value() as u32),
            (false, 12) => Ok(u32::from(self.user_run)),
            (true, _) => self.system.read(offset),
            (false, _) => self.processor.read(offset),
        }
    }

    pub fn write(&mut self, system: bool, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match (system, offset) {
            (true, 16) => {
                let user_mode = value & 1 != 0;
                if user_mode != self.user_mode {
                    // Mode-switch transfer state is not specified by NCR.
                    // Initialize the selected count/status; retain Run and the
                    // counter-mode limit latch. See docs/interrupts-timers.md.
                    self.user_mode = user_mode;
                    self.user = UserTimer::default();
                    self.processor.count = 0;
                    self.processor.pending = false;
                }
                Ok(())
            }
            (true, 12) => Ok(()),
            (false, 0) if self.user_mode => {
                self.user
                    .write((u64::from(value) << 32) | u64::from(self.user.value() as u32));
                Ok(())
            }
            (false, 4) if self.user_mode => {
                self.user
                    .write((self.user.value() & 0xffff_ffff_0000_0000) | u64::from(value));
                Ok(())
            }
            (false, 12) => {
                self.user_run = value & 1 != 0;
                Ok(())
            }
            (true, _) => self.system.write(offset, value),
            (false, _) => self.processor.write(offset, value),
        }
    }

    /// Only the processor count in user mode accepts two-word bursts. Sample
    /// both halves together; do not use the destructive counter-limit reader.
    pub fn read_user_doubleword(&self) -> Result<[u32; 2], BusErrorKind> {
        self.require_user_mode()?;
        let value = self.user.value();
        Ok([(value >> 32) as u32, value as u32])
    }

    pub fn write_user_doubleword(&mut self, words: [u32; 2]) -> Result<(), BusErrorKind> {
        self.require_user_mode()?;
        self.user
            .write((u64::from(words[0]) << 32) | u64::from(words[1]));
        Ok(())
    }

    fn require_user_mode(&self) -> Result<(), BusErrorKind> {
        if self.user_mode {
            Ok(())
        } else {
            Err(BusErrorKind::Unsupported(
                "timer doubleword requires processor user mode",
            ))
        }
    }
}
