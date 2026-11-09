//! M48T08 SRAM/clock and the SS-5 IDPROM. Clock advancement is explicit and
//! deterministic; no host wall clock is consulted. See docs/devices.md.
use crate::bus::BusErrorKind;
use std::fmt;

pub const NVRAM_SIZE: usize = 0x2000;
pub const IDPROM_OFFSET: usize = 0x1fd8;
pub const CLOCK_CONTROL: usize = 0x1ff8;
const READ: u8 = 0x40;
const WRITE: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clock {
    second: u8,
    minute: u8,
    hour: u8,
    weekday: u8,
    date: u8,
    month: u8,
    year: u8,
    stopped: bool,
}

impl Clock {
    fn days_in_month(self) -> u8 {
        match self.month {
            4 | 6 | 9 | 11 => 30,
            2 => {
                if self.year % 4 == 0 {
                    29
                } else {
                    28
                }
            }
            _ => 31,
        }
    }
    fn encode(self) -> [u8; 7] {
        fn bcd(v: u8) -> u8 {
            ((v / 10) << 4) | (v % 10)
        }
        [
            bcd(self.second) | if self.stopped { 0x80 } else { 0 },
            bcd(self.minute),
            bcd(self.hour),
            self.weekday,
            bcd(self.date),
            bcd(self.month),
            bcd(self.year),
        ]
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        fn bcd(v: u8, min: u8, max: u8) -> Option<u8> {
            let value = (v >> 4) * 10 + (v & 15);
            if v & 15 > 9 || v >> 4 > 9 || !(min..=max).contains(&value) {
                None
            } else {
                Some(value)
            }
        }
        let clock = Self {
            second: bcd(bytes[0] & 0x7f, 0, 59)?,
            minute: bcd(bytes[1], 0, 59)?,
            hour: bcd(bytes[2], 0, 23)?,
            weekday: bcd(bytes[3], 1, 7)?,
            date: bcd(bytes[4], 1, 31)?,
            month: bcd(bytes[5], 1, 12)?,
            year: bcd(bytes[6], 0, 99)?,
            stopped: bytes[0] & 0x80 != 0,
        };
        (clock.date <= clock.days_in_month()).then_some(clock)
    }
    fn advance(&mut self, seconds: u64) {
        if self.stopped {
            return;
        }
        let within_day = u64::from(self.hour) * 3600
            + u64::from(self.minute) * 60
            + u64::from(self.second)
            + seconds % 86400;
        self.second = (within_day % 60) as u8;
        self.minute = (within_day / 60 % 60) as u8;
        self.hour = (within_day / 3600 % 24) as u8;
        let days = seconds / 86400 + within_day / 86400;
        self.weekday = ((u64::from(self.weekday - 1) + days % 7) % 7 + 1) as u8;
        // The chip's two-digit year repeats after 100 years, including 25 leap
        // days. Reduce before walking months to bound work for any u64 input.
        let mut days = days % 36525;
        while days != 0 {
            let remaining = u64::from(self.days_in_month() - self.date + 1);
            if days < remaining {
                self.date += days as u8;
                break;
            }
            days -= remaining;
            self.date = 1;
            self.month += 1;
            if self.month > 12 {
                self.month = 1;
                self.year = (self.year + 1) % 100;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvramError {
    InvalidSize,
    InvalidClock,
    UnsupportedCalibration,
}
impl fmt::Display for NvramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidSize => "NVRAM image must contain exactly 8192 bytes",
            Self::InvalidClock => {
                "NVRAM image has invalid BCD clock fields or unsupported clock-test bits"
            }
            Self::UnsupportedCalibration => "RTC frequency calibration is not implemented",
        })
    }
}
impl std::error::Error for NvramError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nvram {
    bytes: Box<[u8; NVRAM_SIZE]>,
    clock: Clock,
}

impl Default for Nvram {
    fn default() -> Self {
        Self::new([0x08, 0x00, 0x20, 0xc0, 0xff, 0xee], [0xc0, 0xff, 0xee])
    }
}

impl Nvram {
    pub fn new(mac: [u8; 6], serial: [u8; 3]) -> Self {
        Self::with_machine_type(0x80, mac, serial)
    }

    /// IDPROM with an explicit machine-type byte. Byte 1 is also the high byte
    /// of the hostid: SS-5 uses 0x80; SS-20 and Axil 320 use 0x72.
    pub fn with_machine_type(machine_type: u8, mac: [u8; 6], serial: [u8; 3]) -> Self {
        let mut bytes = Box::new([0; NVRAM_SIZE]);
        let id = &mut bytes[IDPROM_OFFSET..IDPROM_OFFSET + 16];
        id[0] = 1;
        id[1] = machine_type;
        id[2..8].copy_from_slice(&mac);
        id[12..15].copy_from_slice(&serial);
        id[15] = id[..15].iter().fold(0, |sum, byte| sum ^ byte);
        // SS-5 software uses 1968 as the year origin: 32 encodes 2000-01-01.
        let clock = Clock {
            second: 0,
            minute: 0,
            hour: 0,
            weekday: 7,
            date: 1,
            month: 1,
            year: 32,
            stopped: false,
        };
        bytes[CLOCK_CONTROL + 1..].copy_from_slice(&clock.encode());
        Self { bytes, clock }
    }

    pub fn from_image(image: &[u8]) -> Result<Self, NvramError> {
        if image.len() != NVRAM_SIZE {
            return Err(NvramError::InvalidSize);
        }
        if image[CLOCK_CONTROL] & 0x3f != 0 {
            return Err(NvramError::UnsupportedCalibration);
        }
        let clock = Clock::decode(&image[CLOCK_CONTROL + 1..]).ok_or(NvramError::InvalidClock)?;
        let mut bytes = Box::new([0; NVRAM_SIZE]);
        bytes.copy_from_slice(image);
        Ok(Self { bytes, clock })
    }

    pub fn image(&self) -> Vec<u8> {
        let mut bytes = self.bytes.to_vec();
        if bytes[CLOCK_CONTROL] & (READ | WRITE) == 0 {
            bytes[CLOCK_CONTROL + 1..].copy_from_slice(&self.clock.encode());
        }
        bytes
    }

    pub fn advance_seconds(&mut self, seconds: u64) {
        self.clock.advance(seconds);
    }

    pub fn read(&self, offset: usize) -> Result<u8, BusErrorKind> {
        if offset >= NVRAM_SIZE {
            return Err(BusErrorKind::Unmapped);
        }
        if offset > CLOCK_CONTROL && self.bytes[CLOCK_CONTROL] & (READ | WRITE) == 0 {
            Ok(self.clock.encode()[offset - CLOCK_CONTROL - 1])
        } else {
            Ok(self.bytes[offset])
        }
    }

    pub fn write(&mut self, offset: usize, value: u8) -> Result<(), BusErrorKind> {
        if offset >= NVRAM_SIZE {
            return Err(BusErrorKind::Unmapped);
        }
        if offset == CLOCK_CONTROL {
            if value & 0x3f != 0 {
                return Err(BusErrorKind::Unsupported("RTC calibration"));
            }
            let old = self.bytes[offset];
            let clock = if old & WRITE != 0 && value & WRITE == 0 {
                Some(Clock::decode(&self.bytes[CLOCK_CONTROL + 1..]).ok_or(
                    BusErrorKind::Unsupported("invalid RTC BCD fields or clock-test mode"),
                )?)
            } else {
                None
            };
            if old & (READ | WRITE) == 0 && value & (READ | WRITE) != 0 {
                self.bytes[CLOCK_CONTROL + 1..].copy_from_slice(&self.clock.encode());
            }
            if let Some(clock) = clock {
                self.clock = clock;
            }
        }
        // Clock register writes affect the transfer registers; the counter is
        // loaded only on W's falling edge. R holds these registers, not time.
        self.bytes[offset] = value;
        Ok(())
    }
}
