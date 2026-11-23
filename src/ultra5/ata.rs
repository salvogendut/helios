//! ATA primary master on the CMD646. The data port presents 16-bit PCI
//! little-endian transfers. Sector payload bytes retain their on-disk order.
use crate::media::{Media, MediaKind};
use std::{io, path::Path};

const READY: u8 = 0x50;
const DRQ: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transfer {
    Idle,
    Identify,
    Read,
    Write,
    DmaRead,
    DmaWrite,
}

pub(super) struct Disk {
    media: Media,
    sectors: u32,
    head: u8,
    count: u8,
    sector: u8,
    cylinder: u16,
    heads: u8,
    track: u8,
    control: u8,
    features: u8,
    dma_mode: Option<u8>,
    status: u8,
    irq: bool,
    error: u8,
    transfer: Transfer,
    lba: u32,
    pos: usize,
    data: [u8; 512],
}

impl Disk {
    pub(super) fn open(path: &Path, writable: bool) -> io::Result<Self> {
        let media = Media::open(path, MediaKind::Disk, writable)?;
        let size = media.info().logical_bytes;
        if size % 512 != 0 || !(1..=0x0fff_ffff).contains(&(size / 512)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ATA disk requires 1..=0x0fffffff whole 512-byte sectors",
            ));
        }
        let mut disk = Self {
            media,
            sectors: (size / 512) as u32,
            head: 0xa0,
            count: 1,
            sector: 1,
            cylinder: 0,
            heads: 16,
            track: 63,
            control: 0,
            features: 0,
            dma_mode: None,
            irq: false,
            status: READY,
            error: 1,
            transfer: Transfer::Idle,
            lba: 0,
            pos: 0,
            data: [0; 512],
        };
        disk.reset();
        Ok(disk)
    }

    fn reset(&mut self) {
        self.irq = false;
        self.head = 0xa0;
        self.count = 1;
        self.sector = 1;
        self.cylinder = 0;
        self.status = READY;
        self.error = 1;
        self.features = 0;
        self.dma_mode = None;
        self.transfer = Transfer::Idle;
        self.pos = 0;
    }

    pub(super) fn interrupt(&self) -> bool {
        self.irq && self.control & 6 == 0
    }
    pub(super) fn read(&mut self, reg: u8) -> Result<u8, String> {
        if self.head & 0x10 != 0 {
            return Ok(if matches!(reg, 1 | 7 | 8) { 0 } else { 0xff });
        }
        if reg == 7 {
            self.irq = false;
        }
        Ok(match reg {
            1 => self.error,
            2 => self.count,
            3 => self.sector,
            4 => self.cylinder as u8,
            5 => (self.cylinder >> 8) as u8,
            6 => self.head,
            7 | 8 if self.control & 4 != 0 => 0x80,
            7 | 8 => self.status,
            _ => return Err("ATA data register requires a word transfer".into()),
        })
    }

    pub(super) fn write(&mut self, reg: u8, value: u8) -> Result<(), String> {
        if reg == 8 {
            if self.control & 4 != 0 && value & 4 == 0 {
                self.reset();
            }
            self.control = value;
        } else if self.control & 4 == 0 {
            if reg == 6 {
                self.head = value;
            } else if self.head & 0x10 == 0 {
                match reg {
                    1 => self.features = value,
                    2 => self.count = value,
                    3 => self.sector = value,
                    4 => self.cylinder = (self.cylinder & 0xff00) | u16::from(value),
                    5 => self.cylinder = (self.cylinder & 0xff) | (u16::from(value) << 8),
                    7 => self.command(value),
                    _ => return Err("ATA data register requires a word transfer".into()),
                }
            }
        }
        Ok(())
    }

    fn fail(&mut self, error: u8) {
        self.irq = true;
        self.error = error;
        self.status = READY | 1;
        self.transfer = Transfer::Idle;
    }

    fn command(&mut self, op: u8) {
        // PIO data-out requests its first block by DRQ without an interrupt.
        self.irq = !matches!(op, 0x30 | 0x31 | 0xc8..=0xcb);
        self.transfer = Transfer::Idle;
        self.error = 0;
        self.status = READY;
        self.pos = 0;
        match op {
            0xec => {
                self.identify();
                self.transfer = Transfer::Identify;
                self.status |= DRQ;
            }
            0x20 | 0x21 | 0x30 | 0x31 | 0xc8..=0xcb => {
                self.lba = if self.head & 0x40 != 0 {
                    (u32::from(self.head & 15) << 24)
                        | (u32::from(self.cylinder) << 8)
                        | u32::from(self.sector)
                } else if self.sector == 0
                    || self.sector > self.track
                    || self.head & 15 >= self.heads
                {
                    self.fail(0x10);
                    return;
                } else {
                    (u32::from(self.cylinder) * u32::from(self.heads) + u32::from(self.head & 15))
                        * u32::from(self.track)
                        + u32::from(self.sector - 1)
                };
                let count = if self.count == 0 {
                    256
                } else {
                    u32::from(self.count)
                };
                if self.lba + count > self.sectors {
                    self.fail(0x10);
                } else if op >= 0xc8 {
                    if op & 2 != 0 && !self.media.writable() {
                        self.fail(4);
                    } else {
                        self.transfer = if op & 2 == 0 {
                            Transfer::DmaRead
                        } else {
                            Transfer::DmaWrite
                        };
                        self.status |= DRQ;
                    }
                } else if op & 0x10 == 0 {
                    self.transfer = Transfer::Read;
                    self.read_sector();
                } else if self.media.writable() {
                    self.transfer = Transfer::Write;
                    self.status |= DRQ;
                } else {
                    self.fail(4);
                }
            }
            0x10..=0x1f => self.cylinder = 0, // recalibrate
            0x90 => self.reset(),
            0x91 if self.count != 0 => {
                self.heads = (self.head & 15) + 1;
                self.track = self.count;
            }
            0xef if self.features == 3 && matches!(self.count, 0..=4 | 8..=12) => {
                self.dma_mode = None;
            }
            0xef if self.features == 3 && (0x20..=0x22).contains(&self.count) => {
                self.dma_mode = Some(self.count - 0x20);
            }
            0xe7 => {
                if self.media.sync().is_err() {
                    self.fail(0x40);
                }
            }
            _ => self.fail(4), // ABRT
        }
    }

    pub(super) fn dma_request(&self) -> Option<(bool, usize)> {
        let read = match self.transfer {
            Transfer::DmaRead => true,
            Transfer::DmaWrite => false,
            _ => return None,
        };
        Some((
            read,
            512 * if self.count == 0 {
                256
            } else {
                usize::from(self.count)
            },
        ))
    }
    pub(super) fn dma_transfer(&mut self, bytes: &mut [u8]) -> bool {
        let Some((read, size)) = self.dma_request() else {
            return false;
        };
        if bytes.len() != size {
            self.fail(4);
            return false;
        }
        let at = u64::from(self.lba) * 512;
        let result = if read {
            self.media.read_exact_at(at, bytes)
        } else {
            self.media.write_all_at(at, bytes)
        };
        if result.is_err() {
            self.fail(0x40);
            return false;
        }
        for _ in 0..size / 512 {
            self.advance();
        }
        self.irq = true;
        true
    }
    pub(super) fn dma_abort(&mut self) {
        self.fail(4);
    }

    fn read_sector(&mut self) {
        if self
            .media
            .read_exact_at(u64::from(self.lba) * 512, &mut self.data)
            .is_ok()
        {
            self.status |= DRQ;
        } else {
            self.fail(0x40);
        }
    }

    fn advance(&mut self) {
        self.irq = self.transfer == Transfer::Write || self.count != 1;
        self.pos = 0;
        self.lba += 1;
        self.count = self.count.wrapping_sub(1);
        if self.head & 0x40 != 0 {
            self.head = (self.head & 0xf0) | (self.lba >> 24) as u8;
            self.cylinder = (self.lba >> 8) as u16;
            self.sector = self.lba as u8;
        } else {
            let t = self.lba / u32::from(self.track);
            self.sector = (self.lba % u32::from(self.track)) as u8 + 1;
            self.head = (self.head & 0xf0) | (t % u32::from(self.heads)) as u8;
            self.cylinder = (t / u32::from(self.heads)) as u16;
        }
        if self.count == 0 {
            self.status = READY;
            self.transfer = Transfer::Idle;
        } else if self.transfer == Transfer::Read {
            self.read_sector();
        }
    }

    pub(super) fn read_data(&mut self) -> Result<[u8; 2], String> {
        if self.head & 0x10 != 0 {
            return Ok([0xff; 2]);
        }
        if self.control & 4 != 0 || !matches!(self.transfer, Transfer::Read | Transfer::Identify) {
            return Err("ATA data read without a PIO data-in phase".into());
        }
        let value = [self.data[self.pos], self.data[self.pos + 1]];
        self.pos += 2;
        if self.pos == 512 {
            if self.transfer == Transfer::Identify {
                self.transfer = Transfer::Idle;
                self.status = READY;
            } else {
                self.advance();
            }
        }
        Ok(value)
    }

    pub(super) fn write_data(&mut self, bytes: [u8; 2]) -> Result<(), String> {
        if self.head & 0x10 != 0 {
            return Ok(());
        }
        if self.control & 4 != 0 || self.transfer != Transfer::Write {
            return Err("ATA data write without a PIO data-out phase".into());
        }
        self.data[self.pos..self.pos + 2].copy_from_slice(&bytes);
        self.pos += 2;
        if self.pos == 512 {
            if self
                .media
                .write_all_at(u64::from(self.lba) * 512, &self.data)
                .is_err()
            {
                self.fail(0x40);
            } else {
                self.advance();
            }
        }
        Ok(())
    }

    fn identify(&mut self) {
        self.data.fill(0);
        let cylinders = (self.sectors / (16 * 63)).min(16383) as u16;
        let current = (self.sectors / (u32::from(self.heads) * u32::from(self.track))).min(65535);
        let chs_capacity = current * u32::from(self.heads) * u32::from(self.track);
        for (word, value) in [
            (0, 0x0040),
            (1, cylinders),
            (3, 16),
            (6, 63),
            (49, 0x0300),
            (53, 3),
            (54, current as u16),
            (55, u16::from(self.heads)),
            (56, u16::from(self.track)),
            (57, chs_capacity as u16),
            (58, (chs_capacity >> 16) as u16),
            (60, self.sectors as u16),
            (61, (self.sectors >> 16) as u16),
            (63, 7 | self.dma_mode.map_or(0, |mode| 0x100 << mode)),
            (64, 3),
            (65, 120),
            (66, 120),
        ] {
            self.data[word * 2..word * 2 + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (word, len, text) in [
            (10, 20, "HELIOS0002"),
            (23, 8, "1.0"),
            (27, 40, "HELIOS ATA DISK"),
        ] {
            for i in 0..len {
                self.data[word * 2 + (i ^ 1)] = text.as_bytes().get(i).copied().unwrap_or(b' ');
            }
        }
    }
}
