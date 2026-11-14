//! NCR89C105 counters and 54-bit processor user timers, clocked every 500 ns.
//! Counter arithmetic is bounded even for very large explicit time advances.
use crate::bus::BusErrorKind;
use std::collections::BTreeMap;

const MODULUS: u128 = 1 << 22;
const FIELD: u32 = 0x7fff_fe00;
const USER_MODULUS: u128 = 1 << 54;
pub const TICK_NS: u64 = 500;
// Sun-4M has four processor slots; board profiles constrain populated slots.
const MAX_HARDWARE_PROCESSOR: u8 = 3;

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
struct ProcessorTimer {
    processor: Counter,
    user: UserTimer,
    user_run: bool,
    user_mode: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timers {
    system: Counter,
    processors: BTreeMap<u8, ProcessorTimer>,
    phase_ns: u16,
}

impl Default for Timers {
    fn default() -> Self {
        let mut timers = Self {
            system: Counter::default(),
            processors: BTreeMap::new(),
            phase_ns: 0,
        };
        timers.install_processor(0);
        timers
    }
}

impl Timers {
    /// Install the CPU-local counter/user-timer bank for a hardware processor
    /// number. The number is the hardware slot, not a compact vector index.
    pub fn install_processor(&mut self, processor: u8) -> bool {
        if processor > MAX_HARDWARE_PROCESSOR || self.processors.contains_key(&processor) {
            return false;
        }
        self.processors.insert(processor, ProcessorTimer::default());
        true
    }

    pub fn processors(&self) -> impl Iterator<Item = u8> + '_ {
        self.processors.keys().copied()
    }

    pub fn contains_processor(&self, processor: u8) -> bool {
        self.processors.contains_key(&processor)
    }

    pub fn reset(&mut self) {
        self.system = Counter::default();
        for bank in self.processors.values_mut() {
            *bank = ProcessorTimer::default();
        }
        self.phase_ns = 0;
    }

    pub fn system_irq(&self) -> bool {
        self.system.pending
    }

    pub fn processor_irq(&self) -> bool {
        self.processor_irq_for(0)
    }

    pub fn processor_irq_for(&self, processor: u8) -> bool {
        self.processors
            .get(&processor)
            .is_some_and(|bank| !bank.user_mode && bank.processor.pending)
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
        for bank in self.processors.values_mut() {
            if bank.user_mode {
                if bank.user_run {
                    bank.user.advance(ticks);
                }
            } else {
                bank.processor.advance(ticks);
            }
        }
    }

    pub fn read(&mut self, system: bool, offset: u64) -> Result<u32, BusErrorKind> {
        if system {
            self.read_system(offset)
        } else {
            self.read_cpu(0, offset)
        }
    }

    pub fn read_system(&mut self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            12 => Ok(0),
            16 => Ok(self.mode_bits()),
            _ => self.system.read(offset),
        }
    }

    pub fn read_cpu(&mut self, processor: u8, offset: u64) -> Result<u32, BusErrorKind> {
        let Some(bank) = self.processors.get_mut(&processor) else {
            return match offset {
                0 | 4 | 8 | 12 => Ok(0),
                _ => Err(BusErrorKind::Unsupported("timer register offset")),
            };
        };
        match offset {
            0 if bank.user_mode => Ok((bank.user.value() >> 32) as u32),
            4 if bank.user_mode => Ok(bank.user.value() as u32),
            12 => Ok(u32::from(bank.user_run)),
            _ => bank.processor.read(offset),
        }
    }

    pub fn write(&mut self, system: bool, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        if system {
            self.write_system(offset, value)
        } else {
            self.write_cpu(0, offset, value)
        }
    }

    pub fn write_system(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match offset {
            12 => Ok(()),
            16 => {
                for (&processor, bank) in &mut self.processors {
                    let user_mode = value & (1 << processor) != 0;
                    if user_mode != bank.user_mode {
                        // Mode-switch transfer state is not specified by NCR.
                        // Initialize the selected count/status; retain Run and the
                        // counter-mode limit latch. See docs/interrupts-timers.md.
                        bank.user_mode = user_mode;
                        bank.user = UserTimer::default();
                        bank.processor.count = 0;
                        bank.processor.pending = false;
                    }
                }
                Ok(())
            }
            _ => self.system.write(offset, value),
        }
    }

    pub fn write_cpu(
        &mut self,
        processor: u8,
        offset: u64,
        value: u32,
    ) -> Result<(), BusErrorKind> {
        let Some(bank) = self.processors.get_mut(&processor) else {
            return match offset {
                0 | 4 | 8 | 12 => Ok(()),
                _ => Err(BusErrorKind::Unsupported("timer register offset")),
            };
        };
        match offset {
            0 if bank.user_mode => {
                bank.user
                    .write((u64::from(value) << 32) | u64::from(bank.user.value() as u32));
                Ok(())
            }
            4 if bank.user_mode => {
                bank.user
                    .write((bank.user.value() & 0xffff_ffff_0000_0000) | u64::from(value));
                Ok(())
            }
            12 => {
                bank.user_run = value & 1 != 0;
                Ok(())
            }
            _ => bank.processor.write(offset, value),
        }
    }

    fn mode_bits(&self) -> u32 {
        self.processors
            .iter()
            .filter(|(_, bank)| bank.user_mode)
            .fold(0, |bits, (&processor, _)| bits | (1 << processor))
    }

    /// Only the processor count in user mode accepts two-word bursts. Sample
    /// both halves together; do not use the destructive counter-limit reader.
    pub fn read_user_doubleword(&self) -> Result<[u32; 2], BusErrorKind> {
        self.read_user_doubleword_for(0)
    }

    pub fn read_user_doubleword_for(&self, processor: u8) -> Result<[u32; 2], BusErrorKind> {
        let bank = self.require_user_mode(processor)?;
        let value = bank.user.value();
        Ok([(value >> 32) as u32, value as u32])
    }

    pub fn write_user_doubleword(&mut self, words: [u32; 2]) -> Result<(), BusErrorKind> {
        self.write_user_doubleword_for(0, words)
    }

    pub fn write_user_doubleword_for(
        &mut self,
        processor: u8,
        words: [u32; 2],
    ) -> Result<(), BusErrorKind> {
        let bank = self.require_user_mode_mut(processor)?;
        bank.user
            .write((u64::from(words[0]) << 32) | u64::from(words[1]));
        Ok(())
    }

    fn require_user_mode(&self, processor: u8) -> Result<&ProcessorTimer, BusErrorKind> {
        self.processors
            .get(&processor)
            .filter(|bank| bank.user_mode)
            .ok_or(BusErrorKind::Unsupported(
                "timer doubleword requires processor user mode",
            ))
    }

    fn require_user_mode_mut(
        &mut self,
        processor: u8,
    ) -> Result<&mut ProcessorTimer, BusErrorKind> {
        self.processors
            .get_mut(&processor)
            .filter(|bank| bank.user_mode)
            .ok_or(BusErrorKind::Unsupported(
                "timer doubleword requires processor user mode",
            ))
    }
}
