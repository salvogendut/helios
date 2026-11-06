//! NCR82077 initialization subset, with no media engine. See docs/devices.md.
use crate::bus::BusErrorKind;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Motion {
    remaining_ns: u64,
    cylinder: u8,
    status: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Floppy {
    dor: u8,
    rate: u8,
    specify: [u8; 2],
    config: u8,
    pretrack: u8,
    perpendicular: u8,
    locked: bool,
    command: Vec<u8>,
    result: VecDeque<u8>,
    poll_ns: Option<u64>,
    reset_status: u8,
    interrupt: bool,
    cylinders: [u8; 4],
    motions: [Option<Motion>; 4],
    seek_results: VecDeque<(u8, u8)>,
}

impl Default for Floppy {
    fn default() -> Self {
        Self {
            dor: 0, // NCR figure 6-5: held in reset after system reset
            rate: 0,
            specify: [0; 2],
            config: 0x20, // FIFO disabled, polling enabled, threshold one
            pretrack: 0,
            perpendicular: 0,
            locked: false,
            command: Vec::new(),
            result: VecDeque::new(),
            poll_ns: None,
            reset_status: 0,
            interrupt: false,
            cylinders: [0; 4],
            motions: [None; 4],
            seek_results: VecDeque::new(),
        }
    }
}

impl Floppy {
    pub fn reset(&mut self) {
        let specify = self.specify; // Intel 2.2.1: SPECIFY survives pin reset
        *self = Self::default();
        self.specify = specify;
    }

    fn reset_core(&mut self) {
        self.command.clear();
        self.result.clear();
        self.poll_ns = None;
        self.reset_status = 0;
        self.interrupt = false;
        self.cylinders = [0; 4];
        self.motions = [None; 4];
        self.seek_results.clear();
        self.perpendicular &= 0x3c;
        if !self.locked {
            self.config = (self.config & 0x50) | 0x20;
            self.pretrack = 0;
        }
    }

    fn start_poll(&mut self) {
        if self.dor & 4 != 0 && self.config & 0x10 == 0 {
            // Functional deadline based on the documented 250 us window at
            // 1 Mbps; scale with the selected data clock. No drive polling bus.
            self.poll_ns = Some(match self.rate & 3 {
                0 => 500_000,
                1 => 833_334,
                2 => 1_000_000,
                _ => 250_000,
            });
        }
    }

    pub fn advance_ns(&mut self, ns: u64) {
        for (drive, motion) in self.motions.iter_mut().enumerate() {
            if let Some(active) = motion {
                if ns >= active.remaining_ns {
                    self.cylinders[drive] = active.cylinder;
                    self.seek_results
                        .push_back((active.status, active.cylinder));
                    *motion = None;
                } else {
                    active.remaining_ns -= ns;
                }
            }
        }
        // Intel 8.2: entering the command phase pauses polling. CONFIGURE
        // need only start before the polling interrupt deadline.
        if !self.command.is_empty() || !self.result.is_empty() {
            return;
        }

        if let Some(left) = self.poll_ns {
            if ns >= left {
                self.poll_ns = None;
                self.reset_status = 4;
                self.interrupt = true;
            } else {
                self.poll_ns = Some(left - ns);
            }
        }
    }

    pub fn irq(&self) -> bool {
        (self.interrupt || !self.seek_results.is_empty()) && self.dor & 0x0c == 0x0c
    }

    fn busy_drives(&self) -> u8 {
        let mut busy = 0;
        for (drive, motion) in self.motions.iter().enumerate() {
            if motion.is_some() {
                busy |= 1 << drive;
            }
        }
        for &(status, _) in &self.seek_results {
            busy |= 1 << (status & 3);
        }
        busy
    }

    pub fn read(&mut self, offset: u64) -> Result<u8, BusErrorKind> {
        match offset {
            2 => Ok(self.dor),
            4 => Ok((if self.dor & 4 == 0 {
                0
            } else if !self.result.is_empty() {
                0xd0 // RQM, DIO, command busy
            } else {
                0x80 | if self.command.is_empty() { 0 } else { 0x10 }
            }) | self.busy_drives()),
            5 => self.result.pop_front().ok_or(BusErrorKind::WouldBlock),
            // No drive attached: inactive disk-change input. The otherwise
            // tri-stated PC/AT DIR bits are a deterministic zero convention.
            7 => Ok(0),
            _ => Err(BusErrorKind::Unsupported("floppy register or mirror")),
        }
    }

    pub fn write(&mut self, offset: u64, value: u8) -> Result<(), BusErrorKind> {
        match offset {
            2 => {
                let was_reset = self.dor & 4 == 0;
                self.dor = value & 0xdd; // NCR DOR reserves bits 5 and 1
                if self.dor & 4 == 0 {
                    self.reset_core();
                } else if was_reset {
                    self.start_poll();
                }
            }
            4 => {
                if value & 0x40 != 0 {
                    return Err(BusErrorKind::Unsupported("floppy power-down mode"));
                }
                self.rate = value & 0x1f;
                if value & 0x80 != 0 {
                    self.reset_core();
                    self.start_poll();
                }
            }
            5 => self.write_fifo(value)?,
            7 => self.rate = (self.rate & !3) | (value & 3),
            _ => return Err(BusErrorKind::Unsupported("floppy register or mirror")),
        }
        Ok(())
    }

    fn command_length(opcode: u8) -> Result<usize, BusErrorKind> {
        match opcode {
            3 | 0x0f => Ok(3),     // SPECIFY / SEEK
            4 | 7 | 0x12 => Ok(2), // SENSE DRIVE / RECALIBRATE / PERPENDICULAR
            8 | 0x0e | 0x10 | 0x14 | 0x94 => Ok(1),
            0x13 => Ok(4), // CONFIGURE
            _ => Err(BusErrorKind::Unsupported("floppy command")),
        }
    }

    fn write_fifo(&mut self, value: u8) -> Result<(), BusErrorKind> {
        if self.dor & 4 == 0 || !self.result.is_empty() {
            return Err(BusErrorKind::WouldBlock);
        }
        let opcode = self.command.first().copied().unwrap_or(value);
        let length = Self::command_length(opcode)?;
        if matches!(opcode, 7 | 0x0f)
            && self.command.len() == 1
            && self.busy_drives() & (1 << (value & 3)) != 0
        {
            return Err(BusErrorKind::WouldBlock);
        }
        self.command.push(value);
        if self.command.len() != length {
            return Ok(());
        }
        match opcode {
            3 => self.specify.copy_from_slice(&self.command[1..3]),
            4 => {
                // ST3 fixed bits 5/3, selected head/drive. No drive pins
                // assert track-zero or write-protect in the disconnected model.
                self.result.push_back(0x28 | (self.command[1] & 7));
            }
            8 => {
                self.interrupt = false;
                if let Some((status, cylinder)) = self.seek_results.pop_front() {
                    self.result.extend([status, cylinder]);
                } else if self.reset_status == 0 {
                    self.result.push_back(0x80);
                } else {
                    self.result.extend([0xc0 | (4 - self.reset_status), 0]);
                    self.reset_status -= 1;
                }
            }
            0x0e => self.result.extend([
                self.cylinders[0],
                self.cylinders[1],
                self.cylinders[2],
                self.cylinders[3],
                self.specify[0],
                self.specify[1],
                0,
                (u8::from(self.locked) << 7) | self.perpendicular,
                self.config,
                self.pretrack,
            ]),
            0x10 => self.result.push_back(0x90),
            7 | 0x0f => {
                let drive = self.command[1] & 3;
                let recalibrate = opcode == 7;
                let cylinder = if recalibrate { 0 } else { self.command[2] };
                let steps = if recalibrate {
                    // Command section 5.2.2 says 79 steps; ST0's table says
                    // 80. Follow the command description, without pulse edges.
                    79
                } else {
                    u64::from(self.cylinders[usize::from(drive)].abs_diff(cylinder)).max(1)
                };
                let rate = [500_000, 300_000, 250_000, 1_000_000][usize::from(self.rate & 3)];
                let numerator = u64::from(16 - (self.specify[0] >> 4)) * 500_000_000_000;
                let step_ns = numerator.div_ceil(rate);
                if recalibrate {
                    self.cylinders[usize::from(drive)] = 0;
                }
                self.motions[usize::from(drive)] = Some(Motion {
                    remaining_ns: steps * step_ns,
                    cylinder,
                    // Disconnected TRK0: recalibration ends with EC/SE and
                    // abnormal termination. SEEK has no position feedback.
                    status: (if recalibrate { 0x70 } else { 0x20 }) | drive,
                });
            }
            0x12 => {
                let value = self.command[1];
                self.perpendicular = (self.perpendicular & 0x3c) | (value & 3);
                if value & 0x80 != 0 {
                    self.perpendicular = value & 0x3f;
                }
            }
            0x13 => {
                self.config = self.command[2] & 0x7f;
                self.pretrack = self.command[3];
                if self.config & 0x10 != 0 {
                    self.poll_ns = None;
                }
            }
            0x14 | 0x94 => {
                self.locked = opcode & 0x80 != 0;
                self.result.push_back(u8::from(self.locked) << 4);
            }
            _ => unreachable!("validated initialization command"),
        }
        self.command.clear();
        Ok(())
    }
}
