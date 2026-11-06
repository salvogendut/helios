//! NCR89C100 NCR53C9X Fast SCSI core: disconnected initialization subset.
//! Register semantics follow the NCR manual, not the QEMU ESP API.
use crate::bus::BusErrorKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Esp {
    count: u32,
    counter: u32,
    fifo: [u8; 16],
    fifo_len: usize,
    command: u8,
    status: u8,
    interrupt: u8,
    irq: bool,
    destination: u8,
    timeout: u8,
    sync_period: u8,
    sync_offset: u8,
    config: [u8; 3],
    clock_factor: u8,
    high_written: bool,
    dma_nop: bool,
    needs_nop: bool,
    selection_enabled: bool,
    reset_ns: u64,
}

impl Default for Esp {
    fn default() -> Self {
        Self {
            count: 0,
            counter: 0,
            fifo: [0; 16],
            fifo_len: 0,
            command: 0,
            status: 0,
            interrupt: 0,
            irq: false,
            destination: 0,
            timeout: 0,
            sync_period: 5,
            sync_offset: 0,
            config: [0; 3],
            clock_factor: 2,
            high_written: false,
            dma_nop: false,
            needs_nop: true,
            selection_enabled: false,
            reset_ns: 0,
        }
    }
}

impl Esp {
    pub fn reset(&mut self) {
        // Count/counter, destination and timeout survive all resets. Only the
        // bottom FIFO element is cleared; the other storage is unspecified.
        self.fifo[0] = 0;
        self.fifo_len = 0;
        self.command = 0;
        self.status = 0;
        self.interrupt = 0;
        self.irq = false;
        self.sync_period = 5;
        self.sync_offset = 0;
        self.config = [0; 3];
        self.clock_factor = 2;
        self.high_written = false;
        self.dma_nop = false;
        self.needs_nop = true;
        self.selection_enabled = false;
        self.reset_ns = 0;
    }

    pub fn irq(&self) -> bool {
        self.irq
    }

    pub fn advance_ns(&mut self, ns: u64) {
        self.reset_ns = self.reset_ns.saturating_sub(ns);
    }

    fn index(offset: u64) -> Result<usize, BusErrorKind> {
        if offset < 0x40 && offset & 3 == 0 {
            Ok((offset / 4) as usize)
        } else {
            Err(BusErrorKind::Unsupported("undocumented ESP register lane"))
        }
    }

    pub fn read(&mut self, offset: u64) -> Result<u8, BusErrorKind> {
        let result = match Self::index(offset)? {
            0 => self.counter as u8,
            1 => (self.counter >> 8) as u8,
            2 => {
                let value = self.fifo[0];
                if self.fifo_len != 0 {
                    self.fifo.copy_within(1..self.fifo_len, 0);
                    self.fifo_len -= 1;
                }
                value
            }
            3 => self.command,
            4 => self.status | if self.irq { 0x80 } else { 0 },
            5 => {
                let value = self.interrupt;
                if self.irq {
                    self.irq = false;
                    self.interrupt = 0;
                    self.status &= 0x17; // TC and live bus phase survive.
                }
                value
            }
            6 => 0, // Disconnected sequencer: no selection or transfer yet.
            7 => self.fifo_len as u8,
            8 => self.config[0],
            11 => self.config[1],
            12 => self.config[2],
            14 if self.config[1] & 0x40 != 0 => {
                if !self.high_written && self.dma_nop {
                    2 // NCR53C9X family 0, revision 2, per the integrated core manual.
                } else {
                    (self.counter >> 16) as u8
                }
            }
            14 => 0, // High counter disabled in 16-bit compatibility mode.
            _ => return Err(BusErrorKind::Unsupported("read of reserved ESP register")),
        };
        Ok(result)
    }

    pub fn write(&mut self, offset: u64, value: u8) -> Result<(), BusErrorKind> {
        match Self::index(offset)? {
            0 => self.count = (self.count & !0xff) | u32::from(value),
            1 => self.count = (self.count & !0xff00) | (u32::from(value) << 8),
            2 => {
                if self.fifo_len == 16 {
                    self.fifo[15] = value;
                    self.status |= 0x40; // FIFO overwrite: gross error, no IRQ.
                } else {
                    self.fifo[self.fifo_len] = value;
                    self.fifo_len += 1;
                }
            }
            3 => return self.command(value),
            4 => self.destination = value & 7,
            5 => self.timeout = value,
            6 => self.sync_period = value & 31,
            7 => self.sync_offset = value & 15,
            8 => {
                if value & 8 != 0 {
                    return Err(BusErrorKind::Unsupported("ESP chip test mode"));
                }
                self.config[0] = value;
            }
            9 => {
                if value & 7 == 1 {
                    return Err(BusErrorKind::Unsupported("reserved ESP clock factor"));
                }
                self.clock_factor = value & 7;
            }
            11 => self.config[1] = value,
            12 => self.config[2] = value,
            14 => {
                if self.config[1] & 0x40 != 0 {
                    self.count = (self.count & 0xffff) | (u32::from(value) << 16);
                    self.high_written = true;
                }
            }
            _ => {
                return Err(BusErrorKind::Unsupported(
                    "write of reserved/test ESP register",
                ))
            }
        }
        Ok(())
    }

    fn command(&mut self, value: u8) -> Result<(), BusErrorKind> {
        let code = value & 0x7f;
        if code == 2 {
            self.reset();
            return Ok(());
        }
        if !matches!(code, 0 | 1 | 3 | 0x44 | 0x45) {
            return Err(BusErrorKind::Unsupported(
                "ESP selection or transfer command",
            ));
        }
        if self.needs_nop && code != 0 {
            return Err(BusErrorKind::Unsupported(
                "ESP command before required reset NOP",
            ));
        }
        if self.reset_ns != 0 && code != 0 {
            return Err(BusErrorKind::Unsupported(
                "ESP command during SCSI reset pulse",
            ));
        }
        if self.irq && matches!(code, 3 | 0x45) {
            return Err(BusErrorKind::Unsupported("ESP stacked interrupt"));
        }
        if value & 0x80 != 0 {
            self.counter = self.count
                & if self.config[1] & 0x40 != 0 {
                    0xffffff
                } else {
                    0xffff
                };
            self.status &= !0x10;
        }
        self.command = value;
        match code {
            0 => {
                self.needs_nop = false;
                if value & 0x80 != 0 {
                    self.dma_nop = true;
                }
            }
            1 => {
                self.fifo_len = 0;
                self.fifo[0] = 0;
            }
            3 => {
                // 40 MHz board clock, 130*CCF clocks gives the documented
                // approximately 25 us pulse (26 us with the normal CCF=8).
                let factor = if self.clock_factor == 0 {
                    8
                } else {
                    self.clock_factor
                };
                self.reset_ns = 130 * u64::from(factor) * 25;
                self.command = 0;
                self.selection_enabled = false;
                self.interrupt = 0x80;
                self.irq = self.config[0] & 0x40 == 0;
            }
            0x44 => self.selection_enabled = true,
            0x45 => {
                self.selection_enabled = false;
                self.interrupt = 8;
                self.irq = true;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
