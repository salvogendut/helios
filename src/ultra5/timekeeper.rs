//! M48T59 register transfers (ST §§3.1–3.3, 3.10). The counter follows
//! elapsed board time even while the CPU has latched the transfer registers.
use crate::devices::nvram::Clock;

const READ: u8 = 0x40;
const WRITE: u8 = 0x80;
const CENTURY_ENABLE: u8 = 0x20;
const CENTURY: u8 = 0x10;
// Implemented bits in the transfer registers (ST table 5), with FT cleared.
const CALENDAR_MASK: [u8; 7] = [0xff, 0x7f, 0x3f, 0x37, 0x3f, 0x1f, 0xff];
const SECOND_NS: u64 = 1_000_000_000;

pub(super) struct Timekeeper {
    flags: u8,
    alarm: [u8; 4],
    live: Option<Clock>,
    control: u8,
    century_bits: u8,
    transfer: [u8; 7],
    last_edge_ns: u64,
}

impl Timekeeper {
    pub(super) fn from_registers(regs: &[u8]) -> Self {
        let mut transfer: [u8; 7] = regs[9..].try_into().unwrap();
        for (field, mask) in transfer.iter_mut().zip(CALENDAR_MASK) {
            *field &= mask;
        }
        // The empty firmware fixture has no retained date. Start it at the
        // same deterministic Saturday, 2000-01-01 as the sun4m Sun clock.
        if transfer == [0; 7] {
            transfer = [0, 0, 0, 7, 1, 1, 0x32];
        }
        Self {
            flags: regs[0] & 0xd0,
            alarm: regs[2..6].try_into().unwrap(),
            live: Self::decode(transfer),
            control: regs[8] & !(READ | WRITE),
            century_bits: transfer[3] & (CENTURY_ENABLE | CENTURY),
            transfer,
            last_edge_ns: 0,
        }
    }

    fn decode(mut transfer: [u8; 7]) -> Option<Clock> {
        transfer[3] &= 7;
        Clock::decode(&transfer)
    }

    fn refresh(&mut self) {
        if let Some(clock) = self.live {
            self.transfer = clock.encode();
            self.transfer[3] |= self.century_bits;
        }
    }

    fn sync(&mut self, now_ns: u64) {
        let seconds = (now_ns - self.last_edge_ns) / SECOND_NS;
        if seconds != 0 {
            self.last_edge_ns += seconds * SECOND_NS;
            if let Some(clock) = self.live.as_mut() {
                if clock.advance(seconds) && self.century_bits & CENTURY_ENABLE != 0 {
                    self.century_bits ^= CENTURY;
                }
            }
            if self.control & (READ | WRITE) == 0 {
                self.refresh();
            }
        }
    }

    pub(super) fn read(&mut self, offset: usize, now_ns: u64) -> Result<u8, String> {
        match offset {
            0 => {
                // An exact day-of-month match against day 00 cannot occur.
                // Other retained schedules need a running alarm comparator.
                if self.alarm[3] & 0xbf != 0 {
                    return Err("unsupported M48T59 active alarm comparison".into());
                }
                let flags = self.flags;
                self.flags &= !0xc0; // AF and WDF clear on read; BL is retained.
                return Ok(flags);
            }
            1 | 6 | 7 => return Ok(0), // unused, IRQ output disabled, watchdog disabled
            2..=5 => return Ok(self.alarm[offset - 2]),
            8 => return Ok(self.control),
            _ => {}
        }
        if self.control & 0x3f != 0 {
            return Err("unsupported M48T59 oscillator calibration".into());
        }
        self.sync(now_ns);
        Ok(self.transfer[offset - 9])
    }

    pub(super) fn write(&mut self, offset: usize, value: u8, now_ns: u64) -> Result<(), String> {
        if offset < 8 {
            return Err(format!(
                "unsupported M48T59 register write at {:#x}",
                0x1ff0 + offset
            ));
        }
        if offset > 8 {
            if self.control & WRITE == 0 {
                return Err("M48T59 calendar write requires the write latch".into());
            }
            if offset == 12 && value & 0x40 != 0 {
                return Err("unsupported M48T59 frequency-test output".into());
            }
            self.transfer[offset - 9] = value & CALENDAR_MASK[offset - 9];
            return Ok(());
        }
        if value & 0x3f != 0 {
            return Err("unsupported M48T59 oscillator calibration".into());
        }
        if self.control & WRITE != 0 && value & WRITE == 0 {
            let clock = Self::decode(self.transfer).ok_or("invalid M48T59 calendar")?;
            self.live = Some(clock);
            self.century_bits = self.transfer[3] & (CENTURY_ENABLE | CENTURY);
            self.last_edge_ns = now_ns;
        } else {
            self.sync(now_ns);
        }
        self.control = value;
        if value & (READ | WRITE) == 0 {
            self.refresh();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_latch_holds_calendar_while_oscillator_keeps_phase() {
        let mut clock = Timekeeper::from_registers(&[0; 16]);
        assert_eq!(clock.read(15, 0).unwrap(), 0x32);
        assert_eq!(clock.read(9, SECOND_NS - 1).unwrap(), 0);
        clock.write(8, READ, SECOND_NS + 900).unwrap();
        assert_eq!(clock.read(9, 61 * SECOND_NS).unwrap(), 1);
        assert_eq!(clock.read(10, 61 * SECOND_NS).unwrap(), 0);
        clock.write(8, 0, 61 * SECOND_NS + 10).unwrap();
        assert_eq!(clock.read(10, 61 * SECOND_NS + 10).unwrap(), 1);
        assert_eq!(clock.read(9, 62 * SECOND_NS - 1).unwrap(), 1);
        assert_eq!(clock.read(9, 62 * SECOND_NS).unwrap(), 2);
    }

    #[test]
    fn staged_calendar_requires_a_valid_date_and_restarts_at_commit() {
        let mut regs = [0; 16];
        regs[14] = 0x19; // Invalid retained date can still be inspected and repaired.
        let mut clock = Timekeeper::from_registers(&regs);
        assert_eq!(clock.read(14, SECOND_NS).unwrap(), 0x19);
        assert!(clock.write(14, 2, SECOND_NS).is_err());
        clock.write(8, READ | WRITE, SECOND_NS).unwrap();
        for (reg, val) in (9..=15).zip([0x59, 0x59, 0x23, 7, 0x30, 2, 0x32]) {
            clock.write(reg, val, SECOND_NS).unwrap();
        }
        assert!(clock.write(8, READ, 2 * SECOND_NS).is_err()); // February 30
        assert_eq!(clock.read(8, 2 * SECOND_NS).unwrap(), WRITE | READ);
        assert_eq!(clock.read(13, 2 * SECOND_NS).unwrap(), 0x30);
        clock.write(13, 0x29, 2 * SECOND_NS).unwrap();
        clock.write(8, READ, 2 * SECOND_NS + 75).unwrap();
        assert_eq!(clock.read(14, 3 * SECOND_NS + 75).unwrap(), 2);
        clock.write(8, 0, 3 * SECOND_NS + 75).unwrap();
        let result = (9..=15)
            .map(|r| clock.read(r, 3 * SECOND_NS + 75).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(result, [0, 0, 0, 1, 1, 3, 0x32]);
    }

    #[test]
    fn stopped_oscillator_resumes_only_after_committed_restart() {
        let mut clock = Timekeeper::from_registers(&[0; 16]);
        clock.write(8, WRITE, 0).unwrap();
        clock.write(9, 0x89, 0).unwrap();
        clock.write(8, 0, 0).unwrap();
        assert_eq!(clock.read(9, 100 * SECOND_NS).unwrap(), 0x89);
        clock.write(8, WRITE, 100 * SECOND_NS).unwrap();
        clock.write(9, 9, 100 * SECOND_NS).unwrap();
        clock.write(8, 0, 101 * SECOND_NS).unwrap();
        assert_eq!(clock.read(9, 102 * SECOND_NS - 1).unwrap(), 9);
        assert_eq!(clock.read(9, 102 * SECOND_NS).unwrap(), 0x10);
    }

    #[test]
    fn enabled_century_bit_toggles_once_per_year_rollover() {
        for (start, end) in [(0x27, 0x31), (0x37, 0x21), (0x17, 0x11)] {
            let mut regs = [0; 16];
            regs[9..].copy_from_slice(&[0x59, 0x59, 0x23, start, 0x31, 0x12, 0x99]);
            let mut clock = Timekeeper::from_registers(&regs);
            assert_eq!(clock.read(12, SECOND_NS).unwrap(), end);
            assert_eq!(clock.read(15, SECOND_NS).unwrap(), 0);
            assert_eq!(clock.read(13, SECOND_NS).unwrap(), 1);
            let next_century = SECOND_NS + 36525 * 86400 * SECOND_NS;
            let bit = if start & 0x20 != 0 { 0x10 } else { 0 };
            assert_eq!(
                clock.read(12, next_century).unwrap() & 0x30,
                (end ^ bit) & 0x30
            );
        }
    }

    #[test]
    fn power_on_flags_read_clear_and_unsupported_modes_are_explicit() {
        let mut regs = [0; 16];
        regs[..9].copy_from_slice(&[0xff, 0xff, 1, 2, 3, 0, 0xff, 0xff, 0xc0]);
        regs[9..].copy_from_slice(&[0, 0x80, 0xc0, 0x4f, 0xc1, 0xe1, 0x32]);
        let mut clock = Timekeeper::from_registers(&regs);
        for (reg, expected) in (9..=15).zip([0, 0, 0, 7, 1, 1, 0x32]) {
            assert_eq!(clock.read(reg, 0).unwrap(), expected);
        }
        for (offset, expected) in (0..=8).zip([0xd0, 0, 1, 2, 3, 0, 0, 0, 0]) {
            assert_eq!(clock.read(offset, 0).unwrap(), expected);
        }
        assert_eq!(clock.read(0, 0).unwrap(), 0x10);
        assert!(clock.write(7, 1, 0).is_err());
        assert!(clock.write(8, WRITE | 1, 0).is_err());
        assert_eq!(clock.read(8, 0).unwrap(), 0);
        clock.write(8, WRITE, 0).unwrap();
        assert!(clock.write(12, 0x47, 0).is_err());
        assert_eq!(clock.read(12, 0).unwrap(), 7);
        clock.write(10, 0x81, 0).unwrap();
        assert_eq!(clock.read(10, 0).unwrap(), 1);
        regs[5] = 0x80;
        assert!(Timekeeper::from_registers(&regs).read(0, 0).is_err());
    }
}
