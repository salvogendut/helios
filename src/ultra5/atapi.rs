//! Polled 16-bit ATAPI packet transport on the CMD646 secondary channel.
use crate::devices::scsi::{MediaKind, Response, ScsiTarget};
use std::{io, path::Path};

const DRDY: u8 = 0x40;
const DRQ: u8 = 8;
const ERR: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Identify,
    Packet,
    DataIn,
    DataOut,
}

pub(super) struct Cdrom {
    target: ScsiTarget,
    phase: Phase,
    device: u8,
    control: u8,
    features: u8,
    status: u8,
    irq: bool,
    error: u8,
    count: u8,
    sector: u8,
    byte_count: u16,
    limit: usize,
    packet: [u8; 12],
    buffer: Vec<u8>,
    next_lba: u32,
    remaining_blocks: u32,
    pos: usize,
    phase_end: usize,
}

impl Cdrom {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let mut cd = Self {
            target: ScsiTarget::open(path, MediaKind::Cdrom, false)?,
            phase: Phase::Idle,
            device: 0xa0,
            control: 0,
            features: 0,
            irq: false,
            status: 0,
            error: 0,
            count: 0,
            sector: 0,
            byte_count: 0,
            limit: 0,
            packet: [0; 12],
            buffer: Vec::new(),
            next_lba: 0,
            remaining_blocks: 0,
            pos: 0,
            phase_end: 0,
        };
        cd.reset();
        Ok(cd)
    }

    fn reset(&mut self) {
        self.irq = false;
        self.phase = Phase::Idle;
        self.remaining_blocks = 0;
        self.device = 0xa0;
        self.features = 0;
        self.status = 0;
        self.error = 1;
        self.count = 1;
        self.sector = 1;
        self.byte_count = 0xeb14;
        self.buffer.clear();
        self.pos = 0;
        self.phase_end = 0;
    }

    pub(super) fn interrupt(&self) -> bool {
        self.irq && self.control & 6 == 0
    }
    pub(super) fn activity(&self) -> (u64, u64) {
        self.target.activity()
    }
    pub(super) fn read(&mut self, reg: u8) -> Result<u8, String> {
        if self.device & 0x10 != 0 {
            return Ok(if matches!(reg, 1 | 7 | 8) { 0 } else { 0xff });
        }
        if reg == 7 {
            self.irq = false;
        }
        Ok(match reg {
            1 => self.error,
            2 => self.count,
            3 => self.sector,
            4 => self.byte_count as u8,
            5 => (self.byte_count >> 8) as u8,
            6 => self.device,
            7 | 8 if self.control & 4 != 0 => 0x80,
            7 | 8 => self.status,
            _ => return Err("ATAPI data register requires a 16-bit transfer".into()),
        })
    }

    pub(super) fn write(&mut self, reg: u8, value: u8) -> Result<(), String> {
        if reg == 8 {
            if self.control & 4 != 0 && value & 4 == 0 {
                self.reset();
            }
            self.control = value;
            return Ok(());
        }
        if self.control & 4 != 0 {
            return Ok(());
        }
        if reg == 6 {
            self.device = value;
            return Ok(());
        }
        if self.device & 0x10 != 0 {
            return Ok(());
        }
        match reg {
            1 => self.features = value,
            2 => self.count = value,
            3 => self.sector = value,
            4 => self.byte_count = (self.byte_count & 0xff00) | u16::from(value),
            5 => self.byte_count = (self.byte_count & 0xff) | u16::from(value) << 8,
            7 => self.command(value),
            _ => return Err("ATAPI data register requires a 16-bit transfer".into()),
        }
        Ok(())
    }

    fn command(&mut self, op: u8) {
        self.irq = op != 0xa0; // IDENTIFY word 0 advertises polled packet request
        self.remaining_blocks = 0;
        self.pos = 0;
        self.error = 0;
        self.buffer.clear();
        match op {
            0x08 | 0x90 => self.reset(),
            0xa0 if self.features & 3 == 0 => {
                self.limit = match self.byte_count {
                    0 => 65536,
                    n => usize::from(n),
                };
                self.phase = Phase::Packet;
                self.status = DRDY | DRQ;
                self.count = 1;
            }
            0xa1 => {
                self.buffer = identify();
                self.phase_end = self.buffer.len();
                self.phase = Phase::Identify;
                self.status = DRDY | DRQ;
            }
            0xef if self.features == 3 && matches!(self.count,0..=4 | 8..=12) => {
                self.phase = Phase::Idle;
                self.status = DRDY;
            }
            0xec => {
                self.reset(); // return the ATAPI signature on ATA IDENTIFY
                self.abort(4);
            }
            _ => self.abort(4),
        }
    }

    fn abort(&mut self, error: u8) {
        self.irq = true;
        self.phase = Phase::Idle;
        self.remaining_blocks = 0;
        self.status = DRDY | ERR;
        self.count = 3;
        self.error = error;
    }

    fn execute_packet(&mut self) {
        if self.packet[0] != 0xa8 {
            let response = self.target.execute(0, &self.packet);
            self.response(response);
            return;
        }
        // READ(12) has a 32-bit transfer length. Check the complete range
        // before exposing the first byte, then stage at most one optical block.
        let lba = u32::from_be_bytes(self.packet[2..6].try_into().unwrap());
        let blocks = u32::from_be_bytes(self.packet[6..10].try_into().unwrap());
        let failure = if self.packet[1] & !0x18 != 0 || self.packet[10..12] != [0; 2] {
            Some(self.target.check(5, 0x24))
        } else if blocks != 0
            && u64::from(lba) + u64::from(blocks) > self.target.blocks().min(1u64 << 32)
        {
            Some(self.target.check(5, 0x21))
        } else {
            None
        };
        if let Some(error) = failure {
            self.response(error);
        } else if blocks == 0 {
            let response = self.target.execute(0, &[0; 6]); // check medium readiness
            self.response(response);
        } else {
            self.next_lba = lba;
            self.remaining_blocks = blocks;
            self.read_next_block();
        }
    }

    fn read_next_block(&mut self) {
        let mut read = [0u8; 10];
        read[0] = 0x28;
        read[1] = self.packet[1];
        read[2..6].copy_from_slice(&self.next_lba.to_be_bytes());
        read[8] = 1;
        self.remaining_blocks -= 1;
        if self.remaining_blocks != 0 {
            self.next_lba += 1;
        }
        let response = self.target.execute(0, &read);
        self.response(response);
    }

    fn response(&mut self, response: Response) {
        if response.status != 0 {
            self.abort(self.target.sense_key() << 4);
        } else if response.data.is_empty() {
            self.finish();
        } else {
            self.phase = if response.data_out {
                Phase::DataOut
            } else {
                Phase::DataIn
            };
            self.buffer = response.data;
            self.pos = 0;
            self.next_data_phase();
        }
    }

    fn next_data_phase(&mut self) {
        self.irq = true;
        let remaining = self.buffer.len() - self.pos;
        // Intermediate phases always transfer complete words. A final odd byte
        // is padded on the wire only.
        if remaining > self.limit && self.limit < 2 {
            self.abort(4);
            return;
        }
        let len = if remaining > self.limit {
            self.limit & !1
        } else {
            remaining
        };
        self.phase_end = self.pos + len;
        self.byte_count = len as u16;
        self.count = if self.phase == Phase::DataIn { 2 } else { 0 };
        self.status = DRDY | DRQ;
    }

    fn finish(&mut self) {
        self.irq = self.phase != Phase::Identify;
        self.phase = Phase::Idle;
        self.remaining_blocks = 0;
        self.status = DRDY;
        self.count = 3;
    }

    pub(super) fn read_data(&mut self) -> Result<[u8; 2], String> {
        if self.device & 0x10 != 0 {
            return Ok([0xff; 2]);
        }
        if self.control & 4 != 0 || !matches!(self.phase, Phase::DataIn | Phase::Identify) {
            return Err("unexpected ATAPI PIO data read".into());
        }
        let n = (self.phase_end - self.pos).min(2);
        let mut word = [0; 2];
        word[..n].copy_from_slice(&self.buffer[self.pos..self.pos + n]);
        self.pos += n;
        if self.pos == self.buffer.len() && self.remaining_blocks != 0 {
            self.read_next_block();
        } else if self.pos == self.buffer.len() {
            self.finish();
        } else if self.pos == self.phase_end {
            self.next_data_phase();
        }
        Ok(word)
    }

    pub(super) fn write_data(&mut self, word: [u8; 2]) -> Result<(), String> {
        if self.device & 0x10 != 0 || self.control & 4 != 0 {
            return Ok(());
        }
        match self.phase {
            Phase::Packet => {
                self.packet[self.pos..self.pos + 2].copy_from_slice(&word);
                self.pos += 2;
                if self.pos == self.packet.len() {
                    self.execute_packet();
                }
            }
            Phase::DataOut => {
                let n = (self.phase_end - self.pos).min(2);
                self.buffer[self.pos..self.pos + n].copy_from_slice(&word[..n]);
                self.pos += n;
                if self.pos == self.buffer.len() {
                    let response = self.target.complete_data_out(&self.packet, &self.buffer);
                    self.response(response);
                } else if self.pos == self.phase_end {
                    self.next_data_phase();
                }
            }
            _ => return Err("unexpected ATAPI PIO data write".into()),
        }
        Ok(())
    }
}

fn identify() -> Vec<u8> {
    let mut data = vec![0; 512];
    for (word, value) in [(0, 0x85c0u16), (49, 1 << 9)] {
        data[2 * word..2 * word + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (word, len, text) in [
        (10, 20, "HELIOS0001"),
        (23, 8, "1.0"),
        (27, 40, "HELIOS ATAPI CD-ROM"),
    ] {
        for i in 0..len {
            data[2 * word + (i ^ 1)] = text.as_bytes().get(i).copied().unwrap_or(b' ');
        }
    }
    data
}
