//! Narrow SCSI-2 bus with read-only raw disk and CD-ROM targets, one LUN each.
//! Image bytes cross into the emulator here; no host byte order reaches DVMA.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Disk,
    Cdrom,
}

impl MediaKind {
    pub fn block_size(self) -> u32 {
        match self {
            Self::Disk => 512,
            Self::Cdrom => 2048,
        }
    }
}

#[derive(Debug)]
pub struct ScsiTarget {
    image: File,
    kind: MediaKind,
    blocks: u64,
    sense: [u8; 3],
    unit_attention: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Response {
    pub data: Vec<u8>,
    pub status: u8,
}

#[derive(Debug, Default)]
pub struct ScsiBus {
    targets: [Option<ScsiTarget>; 8],
}

impl ScsiBus {
    /// Attach one raw image at LUN 0. ID 7 is reserved for the host adapter.
    /// Images are opened read-only, including disks. No implicit replacement.
    pub fn attach(&mut self, id: u8, kind: MediaKind, path: impl AsRef<Path>) -> io::Result<()> {
        if id >= 7 || self.targets[usize::from(id)].is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SCSI ID must be unused and in 0..=6",
            ));
        }
        let image = File::open(path)?;
        let metadata = image.metadata()?;
        let size = metadata.len();
        let block_size = u64::from(kind.block_size());
        if !metadata.is_file()
            || size == 0
            || size % block_size != 0
            || size / block_size > 1_u64 << 32
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "SCSI image must be a nonempty raw file of whole blocks (512 bytes for disk, 2048 for CD-ROM), at most 2^32 blocks"));
        }
        self.targets[usize::from(id)] = Some(ScsiTarget {
            image,
            kind,
            blocks: size / block_size,
            sense: [0; 3],
            unit_attention: false,
        });
        Ok(())
    }

    pub fn target(&self, id: u8) -> Option<&ScsiTarget> {
        self.targets.get(usize::from(id)).and_then(Option::as_ref)
    }

    pub(crate) fn reset(&mut self) {
        for target in self.targets.iter_mut().flatten() {
            target.unit_attention = true;
            target.sense = [6, 0x29, 0];
        }
    }

    pub(crate) fn execute(&mut self, id: u8, lun: u8, cdb: &[u8]) -> Response {
        self.targets[usize::from(id)]
            .as_mut()
            .expect("selected SCSI target")
            .execute(lun, cdb)
    }
}

impl ScsiTarget {
    pub fn kind(&self) -> MediaKind {
        self.kind
    }
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    fn check(&mut self, key: u8, asc: u8) -> Response {
        self.sense = [key, asc, 0];
        Response {
            status: 2,
            data: Vec::new(),
        }
    }

    fn execute(&mut self, lun: u8, cdb: &[u8]) -> Response {
        if lun != 0 && !matches!(cdb[0], 0x03 | 0x12) {
            return self.check(5, 0x25); // Logical unit not supported.
        }
        if lun == 0 && self.unit_attention && !matches!(cdb[0], 0x03 | 0x12) {
            self.unit_attention = false;
            return self.check(6, 0x29);
        }
        let data = match cdb[0] {
            0x00 => Vec::new(), // TEST UNIT READY: mounted, present media.
            0x03 => {
                let sense = if lun == 0 { self.sense } else { [5, 0x25, 0] };
                let mut data = vec![0; 18];
                data[0] = 0x70;
                data[2] = sense[0];
                data[7] = 10;
                data[12..14].copy_from_slice(&sense[1..]);
                data.truncate(usize::from(cdb[4]));
                if lun == 0 {
                    self.sense = [0; 3];
                    self.unit_attention = false;
                }
                data
            }
            0x12 => {
                if cdb[1] & 3 != 0 || cdb[2] != 0 {
                    return self.check(5, 0x24);
                }
                let mut data = vec![0; 36];
                data[0] = if lun != 0 {
                    0x7f
                } else if self.kind == MediaKind::Cdrom {
                    5
                } else {
                    0
                };
                data[1] = if self.kind == MediaKind::Cdrom {
                    0x80
                } else {
                    0
                };
                data[2] = 2; // SCSI-2, response data format 2, no sync/queue claims.
                data[3] = 2;
                data[4] = 31;
                data[8..16].copy_from_slice(b"HELIOS  ");
                data[16..32].copy_from_slice(match self.kind {
                    MediaKind::Disk => b"SCSI DISK       ",
                    MediaKind::Cdrom => b"SCSI CD-ROM     ",
                });
                data[32..36].copy_from_slice(b"0001");
                data.truncate(usize::from(cdb[4]));
                data
            }
            0x25 => {
                // READ CAPACITY(10), PMI not supported.
                if cdb[8] & 1 != 0 || cdb[2..6] != [0; 4] {
                    return self.check(5, 0x24);
                }
                let mut data = Vec::with_capacity(8);
                data.extend_from_slice(&((self.blocks - 1) as u32).to_be_bytes());
                data.extend_from_slice(&self.kind.block_size().to_be_bytes());
                data
            }
            0x08 | 0x28 => {
                // READ(6)/READ(10).
                let (lba, count) = if cdb[0] == 8 {
                    (
                        u32::from_be_bytes([0, cdb[1] & 31, cdb[2], cdb[3]]),
                        if cdb[4] == 0 { 256 } else { u32::from(cdb[4]) },
                    )
                } else {
                    (
                        u32::from_be_bytes(cdb[2..6].try_into().expect("CDB LBA")),
                        u32::from(u16::from_be_bytes([cdb[7], cdb[8]])),
                    )
                };
                if u64::from(lba) + u64::from(count) > self.blocks {
                    return self.check(5, 0x21);
                }
                let block_size = self.kind.block_size();
                // READ(10) bounds this buffer to 65535 * 2048 bytes.
                let mut data = vec![0; (count * block_size) as usize];
                if self
                    .image
                    .seek(SeekFrom::Start(u64::from(lba) * u64::from(block_size)))
                    .and_then(|_| self.image.read_exact(&mut data))
                    .is_err()
                {
                    return self.check(3, 0x11); // Unrecovered read error.
                }
                data
            }
            0x0a | 0x2a => return self.check(7, 0x27), // Write protected.
            0x1a => {
                // MODE SENSE(6): header and block descriptor, no pages yet.
                if cdb[2] & 0x3f != 0 {
                    return self.check(5, 0x24);
                }
                let mut data = vec![0, 0, 0x80, 0];
                if cdb[1] & 8 == 0 {
                    data[3] = 8;
                    let blocks = self.blocks.min(0xffffff) as u32;
                    data.extend_from_slice(&blocks.to_be_bytes());
                    data.extend_from_slice(&self.kind.block_size().to_be_bytes());
                }
                data[0] = (data.len() - 1) as u8;
                data.truncate(usize::from(cdb[4]));
                data
            }
            _ => return self.check(5, 0x20), // Invalid command operation code.
        };
        Response { data, status: 0 }
    }
}
