//! NCR89C100 NCR53C9X Fast SCSI core: asynchronous initiator subset.
//! Register semantics follow the NCR manual, not the QEMU ESP API.
use super::scsi::{Response, ScsiBus};
use crate::bus::BusErrorKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Work {
    #[default]
    Idle,
    Select,
    Timeout,
    Command,
    Execute,
    DataIn,
    DataOut,
    MessageOut,
    CompleteDataOut,
    Status,
    Message,
}

/// One byte at the ESP/DMA2 interface. `None` requests memory -> ESP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DmaRequest {
    pub write: Option<u8>,
}

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
    test_mode: bool,
    clock_factor: u8,
    high_written: bool,
    dma_nop: bool,
    needs_nop: bool,
    selection_enabled: bool,
    reset_ns: u64,
    work: Work,
    connected: bool,
    sequence: u8,
    selection_ns: u64,
    selecting: bool,
    stop_after_identify: bool,
    identify: bool,
    legacy_lun: bool,
    lun: u8,
    cdb: Vec<u8>,
    response: Response,
    data_offset: usize,
    dma: bool,
    remaining: u32,
    complete_sequence: bool,
    /// SS-20/Axil integrated-core test/read-reserved registers are accepted.
    reference_registers: bool,
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
            test_mode: false,
            clock_factor: 2,
            high_written: false,
            dma_nop: false,
            needs_nop: true,
            selection_enabled: false,
            reset_ns: 0,
            work: Work::Idle,
            connected: false,
            sequence: 0,
            selection_ns: 0,
            selecting: false,
            stop_after_identify: false,
            identify: false,
            legacy_lun: false,
            lun: 0,
            cdb: Vec::new(),
            response: Response::default(),
            data_offset: 0,
            dma: false,
            remaining: 0,
            complete_sequence: false,
            reference_registers: false,
        }
    }
}

impl Esp {
    /// Select the SS-20/Axil core's wider register readback behavior.
    pub fn set_reference_registers(&mut self, reference: bool) {
        self.reference_registers = reference;
    }

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
        self.test_mode = false;
        self.clock_factor = 2;
        self.high_written = false;
        self.dma_nop = false;
        self.needs_nop = true;
        self.selection_enabled = false;
        self.reset_ns = 0;
        self.disconnect();
        self.sequence = 0;
        self.selection_ns = 0;
        self.stop_after_identify = false;
        self.dma = false;
        self.remaining = 0;
        self.complete_sequence = false;
    }

    pub fn irq(&self) -> bool {
        self.irq
    }

    pub fn advance_ns(&mut self, ns: u64) {
        self.reset_ns = self.reset_ns.saturating_sub(ns);
        if self.work == Work::Timeout {
            self.selection_ns = self.selection_ns.saturating_sub(ns);
            if self.selection_ns == 0 {
                self.disconnect();
                self.phase(0);
                self.finish(0x20);
            }
        }
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
            6 => self.sequence,
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
            10 | 13 | 15 if self.reference_registers => 0,
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
                self.test_mode |= value & 8 != 0;
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
            10 | 13 | 15 if self.reference_registers => {}
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
        if self.test_mode && code != 0 {
            // Test-mode commands have no modeled transfer effect. The
            // SS-20/Axil PROM probes them before resetting the chip; ordinary
            // SCSI commands remain unsupported until that reset.
            if self.reference_registers && matches!(code, 0x20..=0x2f) {
                return Ok(());
            }
            return Err(BusErrorKind::Unsupported("ESP command in chip test mode"));
        }
        if !matches!(
            code,
            0 | 1 | 3 | 0x10..=0x12 | 0x41..=0x43 | 0x44 | 0x45
        ) {
            return Err(BusErrorKind::Unsupported(
                "ESP command outside asynchronous initiator subset",
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
        if self.irq && !matches!(code, 0 | 1 | 0x44) {
            return Err(BusErrorKind::Unsupported("ESP stacked interrupt"));
        }
        if self.work != Work::Idle && !matches!(code, 0 | 3) {
            return Err(BusErrorKind::Unsupported("ESP stacked command"));
        }
        // Unsupported connected phases stop before changing counters/FIFO.
        if self.connected {
            if matches!(code, 0x41..=0x43) {
                return Err(BusErrorKind::Unsupported("ESP selection while connected"));
            }
            if code == 0x10 && !matches!(self.status & 7, 0..=3 | 6 | 7) {
                return Err(BusErrorKind::Unsupported("ESP transfer phase"));
            }
            if (code == 0x11 && self.status & 7 != 3) || (code == 0x12 && self.status & 7 != 7) {
                return Err(BusErrorKind::Unsupported("ESP initiator completion phase"));
            }
        }
        if matches!(code, 0x10..=0x12) && !self.connected {
            self.command = 0;
            self.finish(0x40); // Illegal command, per the disconnected mode table.
            return Ok(());
        }
        if matches!(code, 0x10 | 0x11 | 0x41..=0x43) {
            self.dma = value & 0x80 != 0;
        }
        if value & 0x80 != 0 {
            self.counter = self.count
                & if self.config[1] & 0x40 != 0 {
                    0xffffff
                } else {
                    0xffff
                };
            self.status &= !0x10;
            self.remaining = if self.counter != 0 {
                self.counter
            } else if self.config[1] & 0x40 != 0 {
                1 << 24
            } else {
                1 << 16
            };
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
                self.disconnect();
                self.phase(0);
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
            0x41..=0x43 => {
                self.sequence = 0;
                self.cdb.clear();
                self.response = Response::default();
                self.data_offset = 0;
                self.lun = 0;
                self.identify = code != 0x41;
                self.stop_after_identify = code == 0x43;
                self.legacy_lun = code == 0x41;
                self.selecting = true;
                self.work = Work::Select;
            }
            0x10 => {
                self.complete_sequence = false;
                self.work = match self.status & 7 {
                    0 => Work::DataOut,
                    1 => Work::DataIn,
                    2 => Work::Command,
                    3 => Work::Status,
                    6 => Work::MessageOut,
                    7 => Work::Message,
                    _ => unreachable!(),
                };
            }
            0x11 => {
                self.complete_sequence = true;
                self.work = Work::Status;
            }
            0x12 => {
                self.disconnect();
                self.phase(0);
                self.finish(0x20);
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

    fn phase(&mut self, phase: u8) {
        self.status = (self.status & !7) | phase;
    }

    fn finish(&mut self, interrupt: u8) {
        self.work = Work::Idle;
        self.command = 0;
        self.interrupt = interrupt;
        self.irq = true;
    }

    fn disconnect(&mut self) {
        self.connected = false;
        self.selecting = false;
        self.stop_after_identify = false;
        self.identify = false;
        self.legacy_lun = false;
        self.work = Work::Idle;
        self.cdb.clear();
        self.response = Response::default();
        self.data_offset = 0;
    }

    /// Service local SCSI handshakes, yielding before each host-memory byte.
    /// No CPU/MMU access occurs here. DMA completion is committed separately.
    pub(crate) fn poll(&mut self, bus: &mut ScsiBus) -> Result<Option<DmaRequest>, BusErrorKind> {
        match self.work {
            Work::Idle | Work::Timeout => return Ok(None),
            Work::Select => {
                if self.destination == self.config[0] & 7 || bus.target(self.destination).is_none()
                {
                    let factor = if self.clock_factor == 0 {
                        8
                    } else {
                        self.clock_factor
                    };
                    self.selection_ns = u64::from(self.timeout) * 8192 * u64::from(factor) * 25;
                    self.work = Work::Timeout;
                    return Ok(None);
                }
                self.connected = true;
                self.phase(if self.identify { 6 } else { 2 });
                self.work = Work::Command;
            }
            Work::Execute => {
                // A CDB supplies the legacy LUN when no IDENTIFY was sent.
                self.response = bus.execute(self.destination, self.lun, &self.cdb);
                self.data_offset = 0;
                self.phase(if self.response.data_out {
                    0
                } else if self.response.data.is_empty() {
                    3
                } else {
                    1
                });
                let interrupt = if self.selecting {
                    self.sequence = 4;
                    0x18
                } else {
                    0x10
                };
                self.selecting = false;
                self.finish(interrupt);
                return Ok(None);
            }
            Work::CompleteDataOut => {
                self.response =
                    bus.complete_data_out(self.destination, &self.cdb, &self.response.data);
                self.phase(3);
                self.finish(0x10);
                return Ok(None);
            }
            _ => {}
        }
        if self.work == Work::Command {
            // A preloaded IDENTIFY/CDB prefix precedes bytes fetched by DMA.
            if self.fifo_len != 0 {
                let value = self.fifo[0];
                self.validate_command_byte(value)?;
                self.read(8)?;
                self.command_byte(value);
            } else if self.dma && self.remaining != 0 {
                return Ok(Some(DmaRequest { write: None }));
            }
            return Ok(None);
        }
        if self.work == Work::DataOut {
            if self.fifo_len != 0 {
                let value = self.read(8)?;
                self.sent_data_byte(value);
            } else if self.dma && self.remaining != 0 {
                return Ok(Some(DmaRequest { write: None }));
            }
            return Ok(None);
        }
        if self.work == Work::MessageOut {
            if self.fifo_len != 0 {
                self.read(8)?;
                if self.fifo_len == 0 {
                    self.finish_message_out();
                }
            } else if self.dma && self.remaining != 0 {
                return Ok(Some(DmaRequest { write: None }));
            } else if !self.dma {
                self.finish_message_out();
            }
            return Ok(None);
        }
        let byte = match self.work {
            Work::DataIn => self.response.data[self.data_offset],
            Work::Status => self.response.status,
            Work::Message => 0, // COMMAND COMPLETE.
            _ => unreachable!(),
        };
        if self.dma {
            return Ok(Some(DmaRequest { write: Some(byte) }));
        }
        if self.fifo_len < self.fifo.len() {
            self.write(8, byte)?;
            self.received_byte();
        }
        Ok(None)
    }

    pub(crate) fn validate_dma(&self, value: u8) -> Result<(), BusErrorKind> {
        if self.work == Work::Command {
            self.validate_command_byte(value)
        } else {
            Ok(())
        }
    }

    fn validate_command_byte(&self, value: u8) -> Result<(), BusErrorKind> {
        if self.identify && value & 0xb8 != 0x80 {
            return Err(BusErrorKind::Unsupported(
                "SCSI selection message other than IDENTIFY",
            ));
        }
        if !self.identify && self.cdb.is_empty() && !matches!(value >> 5, 0 | 1 | 2 | 5) {
            return Err(BusErrorKind::Unsupported("SCSI CDB group length"));
        }
        Ok(())
    }

    fn command_byte(&mut self, value: u8) {
        if self.identify {
            self.lun = value & 7;
            self.identify = false;
            if self.stop_after_identify {
                self.stop_after_identify = false;
                self.selecting = false;
                self.phase(6);
                self.sequence = 1;
                self.finish(0x18);
                return;
            }
            self.phase(2);
            self.sequence = 2;
        } else {
            self.cdb.push(value);
            if self.cdb.len() == 2 && self.legacy_lun {
                self.lun = value >> 5;
            }
            let length = match self.cdb[0] >> 5 {
                0 => 6,
                1 | 2 => 10,
                5 => 12,
                _ => unreachable!(),
            };
            if self.cdb.len() == length {
                self.work = Work::Execute;
            }
        }
        if self.work == Work::Command
            && ((self.dma && self.remaining == 0) || (!self.dma && self.fifo_len == 0))
        {
            // A short CDB can be continued with TRANSFER INFORMATION.
            if self.selecting {
                self.sequence = 3;
            }
            self.finish(if self.selecting { 0x18 } else { 0x10 });
            self.selecting = false;
        }
    }

    pub(crate) fn complete_dma(&mut self, value: u8) {
        self.remaining -= 1;
        self.counter = self.remaining
            & if self.config[1] & 0x40 != 0 {
                0xffffff
            } else {
                0xffff
            };
        if self.remaining == 0 {
            self.status |= 0x10;
        }
        if self.work == Work::Command {
            self.command_byte(value);
        } else if self.work == Work::DataOut {
            self.sent_data_byte(value);
        } else if self.work == Work::MessageOut {
            self.sent_message_byte();
        } else {
            self.received_byte();
        }
    }

    fn sent_message_byte(&mut self) {
        if self.remaining == 0 {
            self.finish_message_out();
        }
    }

    fn finish_message_out(&mut self) {
        self.phase(2);
        self.finish(0x10);
    }

    fn sent_data_byte(&mut self, value: u8) {
        self.response.data[self.data_offset] = value;
        self.data_offset += 1;
        if self.data_offset == self.response.data.len() {
            self.work = Work::CompleteDataOut;
        } else if (self.dma && self.remaining == 0) || (!self.dma && self.fifo_len == 0) {
            self.finish(0x10);
        }
    }

    fn received_byte(&mut self) {
        match self.work {
            Work::DataIn => {
                self.data_offset += 1;
                if self.data_offset == self.response.data.len() {
                    self.phase(3);
                    self.finish(0x10);
                } else if !self.dma || self.remaining == 0 {
                    self.finish(0x10);
                }
            }
            Work::Status => {
                self.phase(7);
                if self.complete_sequence && (!self.dma || self.remaining != 0) {
                    self.work = Work::Message;
                } else {
                    self.finish(0x10);
                }
            }
            Work::Message => self.finish(8),
            _ => unreachable!(),
        }
    }
}
