//! Narrow SCSI-2 bus with raw disk and read-only CD-ROM targets, one LUN each.
//! Image bytes cross into the emulator here; no host byte order reaches DVMA.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
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
    writable: bool,
    image_bytes: u64,
    block_size: u32,
    sense: [u8; 3],
    unit_attention: bool,
    started: bool,
    removal_prevented: bool,
    reads: u64,
    writes: u64,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Response {
    pub data: Vec<u8>,
    pub status: u8,
    /// `data` is a parameter or disk-write buffer filled during DATA OUT.
    pub data_out: bool,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("data_len", &self.data.len())
            .field("status", &self.status)
            .field("data_out", &self.data_out)
            .finish()
    }
}

#[derive(Debug, Default)]
pub struct ScsiBus {
    targets: [Option<ScsiTarget>; 8],
}

impl ScsiBus {
    /// Attach one raw image at LUN 0. ID 7 is reserved for the host adapter.
    /// Images are opened read-only, including disks. No implicit replacement.
    pub fn attach(&mut self, id: u8, kind: MediaKind, path: impl AsRef<Path>) -> io::Result<()> {
        self.attach_image(id, kind, path.as_ref(), false)
    }

    /// Explicitly attach an existing raw disk for guest writes. This never
    /// creates, truncates or replaces a host file; CD-ROMs remain read-only.
    pub fn attach_disk_rw(&mut self, id: u8, path: impl AsRef<Path>) -> io::Result<()> {
        self.attach_image(id, MediaKind::Disk, path.as_ref(), true)
    }

    fn attach_image(
        &mut self,
        id: u8,
        kind: MediaKind,
        path: &Path,
        writable: bool,
    ) -> io::Result<()> {
        if id >= 7 || self.targets[usize::from(id)].is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SCSI ID must be unused and in 0..=6",
            ));
        }
        let image = File::options().read(true).write(writable).open(path)?;
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
            writable,
            image_bytes: size,
            block_size: kind.block_size(),
            sense: [0; 3],
            unit_attention: false,
            started: true,
            removal_prevented: false,
            reads: 0,
            writes: 0,
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
            target.block_size = target.kind.block_size();
            target.removal_prevented = false;
        }
    }

    pub(crate) fn execute(&mut self, id: u8, lun: u8, cdb: &[u8]) -> Response {
        self.targets[usize::from(id)]
            .as_mut()
            .expect("selected SCSI target")
            .execute(lun, cdb)
    }

    pub(crate) fn complete_data_out(&mut self, id: u8, cdb: &[u8], data: &[u8]) -> Response {
        let target = self.targets[usize::from(id)]
            .as_mut()
            .expect("selected SCSI target");
        match cdb[0] {
            0x15 | 0x55 => target.mode_select(cdb, data),
            0x0a | 0x2a => target.write_blocks(cdb, data),
            _ => target.check(5, 0x20),
        }
    }
}

impl ScsiTarget {
    pub fn kind(&self) -> MediaKind {
        self.kind
    }
    pub fn writable(&self) -> bool {
        self.writable
    }
    pub fn blocks(&self) -> u64 {
        self.image_bytes / u64::from(self.block_size)
    }
    pub fn block_size(&self) -> u32 {
        self.block_size
    }
    pub fn removal_prevented(&self) -> bool {
        self.removal_prevented
    }
    /// Monotonic completed read/write transfer counts for frontend activity
    /// lamps. Counters do not affect guest-visible behavior or reset state.
    pub fn activity(&self) -> (u64, u64) {
        (self.reads, self.writes)
    }

    fn check(&mut self, key: u8, asc: u8) -> Response {
        self.sense = [key, asc, 0];
        Response {
            status: 2,
            ..Response::default()
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
        if !self.started && matches!(cdb[0], 0x00 | 0x08 | 0x0a | 0x25 | 0x28 | 0x2a | 0x35) {
            let response = self.check(2, 4);
            self.sense[2] = 2; // Logical unit not ready, initializing command required.
            return response;
        }
        let data = match cdb[0] {
            0x00 => Vec::new(), // TEST UNIT READY: mounted, present media.
            0x1b => {
                // START STOP UNIT, including IMMED. Functional spin-up is
                // immediate; eject/load is outside the fixed-media backend.
                if cdb[1] & 0x1e != 0 || cdb[4] & !3 != 0 {
                    return self.check(5, 0x24);
                }
                if cdb[4] & 2 != 0 {
                    if self.removal_prevented && cdb[4] & 1 == 0 {
                        let response = self.check(5, 0x53);
                        self.sense[2] = 2; // Medium removal prevented.
                        return response;
                    }
                    return self.check(5, 0x24); // Eject/load backend pending.
                }
                self.started = cdb[4] & 1 != 0;
                Vec::new()
            }
            0x1e => {
                if self.kind != MediaKind::Cdrom {
                    return self.check(5, 0x20);
                }
                if cdb[1] & 0x1f != 0 || cdb[2] != 0 || cdb[3] != 0 || cdb[4] & !1 != 0 {
                    return self.check(5, 0x24);
                }
                self.removal_prevented = cdb[4] & 1 != 0;
                Vec::new()
            }
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
                data.extend_from_slice(&((self.blocks() - 1) as u32).to_be_bytes());
                data.extend_from_slice(&self.block_size.to_be_bytes());
                data
            }
            0x08 | 0x28 | 0x0a | 0x2a => {
                let write = matches!(cdb[0], 0x0a | 0x2a);
                if write && !self.writable {
                    return self.check(7, 0x27);
                }
                let (offset, length) = match self.block_transfer(cdb) {
                    Ok(range) => range,
                    Err(response) => return response,
                };
                let mut data = vec![0; length];
                if write {
                    if cdb[cdb.len() - 1] != 0 {
                        return self.check(5, 0x24); // Linked/vendor control unsupported.
                    }
                    return Response {
                        data,
                        status: 0,
                        data_out: length != 0,
                    };
                }
                if self
                    .image
                    .seek(SeekFrom::Start(offset))
                    .and_then(|_| self.image.read_exact(&mut data))
                    .is_err()
                {
                    return self.check(3, 0x11); // Unrecovered read error.
                }
                self.reads += 1;
                data
            }
            0x35 => {
                if self.kind != MediaKind::Disk {
                    return self.check(5, 0x20);
                }
                // No asynchronous flush or relative addressing. Flushing the
                // entire file satisfies every valid requested block range.
                if cdb[1] & 0x1f != 0 || cdb[6] != 0 || cdb[9] != 0 {
                    return self.check(5, 0x24);
                }
                let lba = u64::from(u32::from_be_bytes(cdb[2..6].try_into().expect("CDB LBA")));
                let count = u64::from(u16::from_be_bytes([cdb[7], cdb[8]]));
                if lba >= self.blocks() || lba + count > self.blocks() {
                    return self.check(5, 0x21);
                }
                if self.writable && self.image.sync_data().is_err() {
                    return self.check(3, 0x0c);
                }
                Vec::new()
            }
            0x15 | 0x55 => {
                // MODE SELECT(6)/(10). SP is unsupported; PF=0 permits the
                // SCSI-1 descriptor form as well as PF=1's SCSI-2 form.
                if cdb[1] & 0x0f != 0 {
                    return self.check(5, 0x24);
                }
                let length = if cdb[0] == 0x15 {
                    usize::from(cdb[4])
                } else {
                    usize::from(u16::from_be_bytes([cdb[7], cdb[8]]))
                };
                return Response {
                    data: vec![0; length],
                    data_out: length != 0,
                    status: 0,
                };
            }
            0x1a | 0x5a => {
                let page = cdb[2] & 0x3f;
                let control = cdb[2] >> 6;
                if !matches!(page, 0 | 0x3f) && self.mode_page(page, control).is_none() {
                    return self.check(5, 0x24);
                }
                if control == 3 {
                    return self.check(5, 0x39);
                }
                let ten = cdb[0] == 0x5a;
                let header = if ten { 8 } else { 4 };
                let mut data = vec![0; header];
                // CD-ROM's device-specific byte has no write-protect bit;
                // its read-only nature is inherent in peripheral type 5.
                data[if ten { 3 } else { 2 }] = if self.kind == MediaKind::Disk && !self.writable {
                    0x80
                } else {
                    0
                };
                if cdb[1] & 8 == 0 {
                    data[header - 1] = 8;
                    let size = if cdb[2] >> 6 == 2 {
                        self.kind.block_size()
                    } else {
                        self.block_size
                    };
                    let blocks = (self.image_bytes / u64::from(size)).min(0xffffff) as u32;
                    data.extend_from_slice(&blocks.to_be_bytes());
                    data.extend_from_slice(&size.to_be_bytes());
                }
                for code in [3, 4] {
                    if page == code || page == 0x3f {
                        if let Some(body) = self.mode_page(code, control) {
                            data.extend_from_slice(&body);
                        }
                    }
                }
                if ten {
                    data[1] = (data.len() - 2) as u8;
                } else {
                    data[0] = (data.len() - 1) as u8;
                }
                let length = if ten {
                    usize::from(u16::from_be_bytes([cdb[7], cdb[8]]))
                } else {
                    usize::from(cdb[4])
                };
                data.truncate(length);
                data
            }
            _ => return self.check(5, 0x20), // Invalid command operation code.
        };
        Response {
            data,
            ..Response::default()
        }
    }

    fn block_transfer(&mut self, cdb: &[u8]) -> Result<(u64, usize), Response> {
        let (lba, count) = if matches!(cdb[0], 0x08 | 0x0a) {
            (
                u32::from_be_bytes([0, cdb[1] & 31, cdb[2], cdb[3]]),
                if cdb[4] == 0 { 256 } else { u32::from(cdb[4]) },
            )
        } else {
            // DPO/FUA are supported by uncached reads and durable writes.
            // Relative addressing and reserved fields remain unsupported.
            if cdb[1] & 7 != 0 || cdb[6] != 0 {
                return Err(self.check(5, 0x24));
            }
            (
                u32::from_be_bytes(cdb[2..6].try_into().expect("CDB LBA")),
                u32::from(u16::from_be_bytes([cdb[7], cdb[8]])),
            )
        };
        if count != 0 && u64::from(lba) + u64::from(count) > self.blocks() {
            return Err(self.check(5, 0x21));
        }
        // READ/WRITE(10) bounds staging to 65535 logical blocks. A zero
        // ten-byte count is a no-op; a zero six-byte count means 256 blocks.
        Ok((
            u64::from(lba) * u64::from(self.block_size),
            (count * self.block_size) as usize,
        ))
    }

    fn write_blocks(&mut self, cdb: &[u8], data: &[u8]) -> Response {
        if !self.writable {
            return self.check(7, 0x27);
        }
        let (offset, length) = match self.block_transfer(cdb) {
            Ok(range) => range,
            Err(response) => return response,
        };
        if data.len() != length {
            return self.check(5, 0x1a);
        }
        // Stage the entire command before touching the image. Reset or DMA
        // failure during DATA OUT cannot leave a partially received write.
        // Host I/O failures can still leave partial data, like a media error.
        if self.image.metadata().map(|m| m.len()).ok() != Some(self.image_bytes)
            || self
                .image
                .seek(SeekFrom::Start(offset))
                .and_then(|_| self.image.write_all(data))
                .and_then(|_| self.image.sync_data())
                .is_err()
        {
            return self.check(3, 0x0c); // Write error; never report false GOOD.
        }
        self.writes += 1;
        Response::default()
    }

    /// SCSI-2 tables 163/171. A fixed virtual geometry, not host disk geometry.
    /// Capacity remains authoritative when the last cylinder is incomplete.
    fn mode_page(&self, page: u8, control: u8) -> Option<[u8; 24]> {
        if self.kind != MediaKind::Disk || !matches!(page, 3 | 4) {
            return None;
        }
        let mut data = [0; 24];
        data[0] = page; // PS=0: no saved mode parameters
        data[1] = 22;
        if control == 1 {
            return Some(data); // No changeable geometry fields.
        }
        if page == 3 {
            data[10..12].copy_from_slice(&64u16.to_be_bytes());
            data[12..14].copy_from_slice(&512u16.to_be_bytes());
            data[15] = 1; // 1:1 interleave
            data[20] = 0x40; // Hard sectors, fixed medium, cylinder ordering.
        } else {
            let cylinders = self.blocks().div_ceil(16 * 64) as u32;
            let cylinders = cylinders.to_be_bytes();
            data[2..5].copy_from_slice(&cylinders[1..]);
            data[5] = 16;
            data[6..9].copy_from_slice(&cylinders[1..]); // No precompensation.
            data[9..12].copy_from_slice(&cylinders[1..]); // No reduced current.
            data[14..17].fill(0xff); // No specified landing zone.
            data[20..22].copy_from_slice(&5400u16.to_be_bytes());
        }
        Some(data)
    }

    fn mode_select(&mut self, cdb: &[u8], data: &[u8]) -> Response {
        let header = if cdb[0] == 0x55 { 8 } else { 4 };
        if data.len() < header {
            return self.check(5, 0x1a);
        }
        let descriptors = if header == 8 {
            usize::from(u16::from_be_bytes([data[6], data[7]]))
        } else {
            usize::from(data[3])
        };
        if data.len() < header + descriptors {
            return self.check(5, 0x1a);
        }
        // Validate every page and descriptor before applying any mode.
        if !matches!(descriptors, 0 | 8) || data[..header - 1].iter().any(|&b| b != 0) {
            return self.check(5, 0x26);
        }
        let mut pages = &data[header + descriptors..];
        if !pages.is_empty() && cdb[1] & 0x10 == 0 {
            return self.check(5, 0x26); // Pages use the SCSI-2 PF=1 format.
        }
        while !pages.is_empty() {
            if pages.len() < 2 || pages.len() < usize::from(pages[1]) + 2 {
                return self.check(5, 0x1a);
            }
            let length = usize::from(pages[1]) + 2;
            let Some(mut expected) = self.mode_page(pages[0], 0) else {
                return self.check(5, 0x26);
            };
            if length != expected.len() {
                return self.check(5, 0x26);
            }
            if pages[0] == 3 {
                // Zero sector count/size select target defaults. Interleave is
                // ignored on MODE SELECT (SCSI-2 9.3.3.3).
                for offset in [10, 12] {
                    if pages[offset..offset + 2] == [0, 0] {
                        expected[offset..offset + 2].fill(0);
                    }
                }
                expected[14..16].copy_from_slice(&pages[14..16]);
            }
            if pages[..length] != expected {
                return self.check(5, 0x26); // Fixed geometry cannot change.
            }
            pages = &pages[length..];
        }
        if descriptors == 0 {
            return Response::default();
        }
        let descriptor = &data[header..];
        let density_valid =
            descriptor[0] == 0 || (self.kind == MediaKind::Cdrom && descriptor[0] == 1);
        let blocks = u32::from_be_bytes([0, descriptor[1], descriptor[2], descriptor[3]]);
        let size = u32::from_be_bytes([0, descriptor[5], descriptor[6], descriptor[7]]);
        let size_valid = match self.kind {
            MediaKind::Disk => size == 512,
            MediaKind::Cdrom => matches!(size, 512 | 1024 | 2048),
        };
        if !density_valid || descriptor[4] != 0 || !size_valid {
            return self.check(5, 0x26);
        }
        let capacity = self.image_bytes / u64::from(size);
        if capacity > 1_u64 << 32 || (blocks != 0 && u64::from(blocks) != capacity.min(0xffffff)) {
            return self.check(5, 0x26);
        }
        self.block_size = size;
        Response::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn activity_counts_completed_transfers_without_changing_target_state() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-scsi-activity-{}-{}.img",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, vec![0; 1024]).unwrap();
        let mut bus = ScsiBus::default();
        bus.attach_disk_rw(3, &path).unwrap();
        assert_eq!(bus.target(3).unwrap().activity(), (0, 0));
        let read = bus.execute(3, 0, &[0x08, 0, 0, 0, 1, 0]);
        assert_eq!(read.status, 0);
        assert_eq!(bus.target(3).unwrap().activity(), (1, 0));
        let write = bus.complete_data_out(3, &[0x0a, 0, 0, 0, 1, 0], &[0x5a; 512]);
        assert_eq!(write.status, 0);
        assert_eq!(bus.target(3).unwrap().activity(), (1, 1));
        std::fs::remove_file(path).unwrap();
    }
}
