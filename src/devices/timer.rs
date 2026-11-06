//! NCR89C105 22-bit system/processor counters, clocked every 500 ns.
//! Counter arithmetic is bounded even for very large explicit time advances.
use crate::bus::BusErrorKind;

const MODULUS: u128 = 1 << 22;
const FIELD: u32 = 0x7fff_fe00;
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
        // Match after incrementing the 22-bit counter; zero matches on wrap.
        // A non-resetting write behind the count waits for that wrap first.
        let mut distance = (u128::from(self.limit) + MODULUS - u128::from(self.count)) % MODULUS;
        if distance == 0 {
            distance = MODULUS;
        }
        if ticks < distance {
            self.count = ((u128::from(self.count) + ticks) % MODULUS) as u32;
        } else {
            self.pending = true;
            // A match reloads 1 (500 ns). Limit=1 next matches after a full
            // wrap; limits 0 and >=2 have a period one shorter than the limit.
            let period = match self.limit {
                0 => MODULUS - 1,
                1 => MODULUS,
                n => u128::from(n - 1),
            };
            self.count = ((1 + (ticks - distance) % period) % MODULUS) as u32;
        }
    }

    fn read(&mut self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => {
                self.pending = false;
                Ok(self.limit << 9)
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
pub struct Timers {
    system: Counter,
    processor: Counter,
    phase_ns: u16,
    user_run: bool,
}

impl Timers {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn system_irq(&self) -> bool {
        self.system.pending
    }
    pub fn processor_irq(&self) -> bool {
        self.processor.pending
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
        self.processor.advance(ticks);
    }

    pub fn read(&mut self, system: bool, offset: u64) -> Result<u32, BusErrorKind> {
        match (system, offset) {
            (true, 12 | 16) => Ok(0),
            (false, 12) => Ok(u32::from(self.user_run)),
            (true, _) => self.system.read(offset),
            (false, _) => self.processor.read(offset),
        }
    }

    pub fn write(&mut self, system: bool, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match (system, offset) {
            (true, 16) if value & 1 != 0 => {
                Err(BusErrorKind::Unsupported("64-bit processor user timer"))
            }
            (true, 12 | 16) => Ok(()),
            (false, 12) => {
                self.user_run = value & 1 != 0;
                Ok(())
            }
            (true, _) => self.system.write(offset, value),
            (false, _) => self.processor.write(offset, value),
        }
    }
}
