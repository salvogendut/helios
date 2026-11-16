//! CHD v5 hard disks. libchdman provides codecs; Helios owns files, persistence
//! and the supported format boundary. No native handle crosses a worker thread.
use super::create::{ensure_absent, TemporaryImage};
use super::{companion_path, validate_size, MediaKind};
use libchdman_rs::Chd;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

const HEADER_BYTES: u64 = 124;
const MAX_HUNKS: u64 = 4 * 1024 * 1024;
const MAX_HUNK_BYTES: u32 = 4 * 1024 * 1024;
const GDDD: u32 = u32::from_be_bytes(*b"GDDD");

#[derive(Debug, Clone, PartialEq, Eq)]
struct Header {
    logical: u64,
    hunk: u32,
    compressed: bool,
    map: Range<u64>,
    sha1: [u8; 20],
    raw_sha1: [u8; 20],
    parent: [u8; 20],
    geometry: [u32; 4],
}

impl Header {
    fn read(file: &mut File) -> io::Result<Self> {
        let length = file.metadata()?.len();
        file.rewind()?;
        let mut h = [0; HEADER_BYTES as usize];
        file.read_exact(&mut h)?;
        let word = |p| u32::from_be_bytes(h[p..p + 4].try_into().unwrap());
        let long = |p| u64::from_be_bytes(h[p..p + 8].try_into().unwrap());
        if &h[..8] != b"MComprHD" || word(8) != 124 || word(12) != 5 {
            return Err(invalid("Only CHD v5 hard disks are supported"));
        }
        let logical = long(32);
        validate_size(logical, MediaKind::Disk)?;
        let hunk = word(56);
        if word(60) != 512 || hunk == 0 || hunk % 512 != 0 || hunk > MAX_HUNK_BYTES {
            return Err(invalid(
                "CHD requires 512-byte units and sector-aligned hunks up to 4 MiB",
            ));
        }
        let hunks = logical.div_ceil(u64::from(hunk));
        if hunks > MAX_HUNKS {
            return Err(invalid("CHD map exceeds the supported 4,194,304 hunks"));
        }
        let compressed = (16..32).step_by(4).any(|p| word(p) != 0);
        let map_start = long(40);
        let map_bytes = if compressed { 16 } else { hunks * 4 };
        let map_end = map_start
            .checked_add(map_bytes)
            .ok_or_else(|| invalid("CHD map overflow"))?;
        if map_start < HEADER_BYTES || map_end > length {
            return Err(invalid("Truncated CHD map"));
        }
        if compressed {
            file.seek(SeekFrom::Start(map_start))?;
            let mut map_header = [0; 16];
            file.read_exact(&mut map_header)?;
            let encoded = u64::from(u32::from_be_bytes(map_header[..4].try_into().unwrap()));
            if encoded > MAX_HUNKS * 16 || encoded > length - map_end {
                return Err(invalid("Truncated or oversized compressed CHD map"));
            }
        }
        // Validate the metadata chain before entering native iteration; corrupt
        // links must not loop indefinitely or request unbounded allocations.
        let mut at = long(48);
        let mut seen = HashSet::new();
        let mut total = 0_u64;
        let mut geometry = None;
        while at != 0 {
            if at < HEADER_BYTES
                || at > length.saturating_sub(16)
                || !seen.insert(at)
                || seen.len() > 1024
            {
                return Err(invalid("Invalid or cyclic CHD metadata chain"));
            }
            file.seek(SeekFrom::Start(at))?;
            let mut m = [0; 16];
            file.read_exact(&mut m)?;
            let tag = u32::from_be_bytes(m[..4].try_into().unwrap());
            let size = u32::from_be_bytes([0, m[5], m[6], m[7]]);
            total += u64::from(size);
            if size > 1024 * 1024 || total > 16 * 1024 * 1024 || u64::from(size) > length - at - 16
            {
                return Err(invalid("Truncated or oversized CHD metadata"));
            }
            if matches!(
                &m[..4],
                b"CHTR" | b"CHT2" | b"CHGD" | b"CHGT" | b"DVD " | b"AVAV"
            ) {
                return Err(invalid("Only hard-disk CHD metadata is supported"));
            }
            if tag == GDDD {
                if geometry.is_some() || size > 256 {
                    return Err(invalid("Invalid or duplicate CHD hard-disk geometry"));
                }
                let mut bytes = vec![0; size as usize];
                file.read_exact(&mut bytes)?;
                geometry = Some(parse_geometry(&bytes)?);
            }
            at = u64::from_be_bytes(m[8..].try_into().unwrap());
        }
        Ok(Self {
            logical,
            hunk,
            compressed,
            map: map_start..map_end,
            sha1: h[84..104].try_into().unwrap(),
            raw_sha1: h[64..84].try_into().unwrap(),
            parent: h[104..124].try_into().unwrap(),
            geometry: geometry.ok_or_else(|| invalid("CHD has no hard-disk geometry"))?,
        })
    }
}

fn parse_geometry(bytes: &[u8]) -> io::Result<[u32; 4]> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid("Invalid CHD geometry text"))?
        .trim_end_matches('\0');
    let fields: Vec<_> = text.split(',').collect();
    if fields.len() != 4 {
        return Err(invalid("Invalid CHD geometry fields"));
    }
    let mut values = [0; 4];
    for (i, prefix) in ["CYLS:", "HEADS:", "SECS:", "BPS:"].iter().enumerate() {
        values[i] = fields[i]
            .strip_prefix(prefix)
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|v| *v != 0)
            .ok_or_else(|| invalid("Invalid CHD geometry value"))?;
    }
    if values[3] != 512
        || values
            .iter()
            .try_fold(1_u64, |n, &v| n.checked_mul(u64::from(v)))
            .is_none()
    {
        return Err(invalid("Unsupported CHD geometry"));
    }
    // The v5 logical byte count, not rounded legacy CHS, is capacity.
    Ok(values)
}

// Fields drop in declaration order: the child must close before its parent.
pub(super) struct Disk {
    image: Chd,
    parent: Option<Chd>,
    file: SharedFile,
    parent_file: Option<SharedFile>,
    logical: u64,
    poisoned: bool,
    invalidate_hashes: bool,
}

impl std::fmt::Debug for Disk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChdDisk")
            .field("logical_bytes", &self.logical)
            .field("has_parent", &self.parent.is_some())
            .field("poisoned", &self.poisoned)
            .finish()
    }
}

impl Disk {
    pub fn open(path: &Path, mut base: File, writable: bool) -> io::Result<Self> {
        let header = Header::read(&mut base)?;
        if header.parent != [0; 20] {
            return Err(invalid(
                "Attach the CHD base, not a child; arbitrary parent chains are unsupported",
            ));
        }
        let sidecar = companion_path(path);
        let has_sidecar = match fs::symlink_metadata(&sidecar) {
            Ok(m) if m.is_file() => true,
            Ok(_) => {
                return Err(invalid(
                    "CHD companion must be a regular file, not a symlink or directory",
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        let needs_child = has_sidecar || (writable && header.compressed);
        if !needs_child {
            if writable {
                base = File::options().read(true).write(true).open(path)?;
                if Header::read(&mut base)? != header {
                    return Err(invalid("CHD changed while opening"));
                }
            }
            let file = SharedFile::new(base, &header, writable)?;
            let image = Chd::open_custom(file.clone(), writable, None).map_err(native_error)?;
            return Ok(Self {
                image,
                parent: None,
                file,
                parent_file: None,
                logical: header.logical,
                poisoned: false,
                invalidate_hashes: header.sha1 != [0; 20] || header.raw_sha1 != [0; 20],
            });
        }
        if header.sha1 == [0; 20] {
            return Err(invalid(
                "A CHD parent must have a nonzero SHA-1; hashless disks are writable only in place",
            ));
        }
        let parent_file = SharedFile::new(base, &header, false)?;
        let parent = Chd::open_custom(parent_file.clone(), false, None).map_err(native_error)?;
        if !has_sidecar {
            let temporary = TemporaryImage::new(&sidecar)?;
            let mut child = Chd::create_with_parent(
                native_path(&temporary.path)?,
                header.logical,
                header.hunk,
                [0; 4],
                &parent,
            )
            .map_err(native_error)?;
            child.clone_all_metadata(&parent).map_err(native_error)?;
            drop(child);
            // No replacement if another process published a companion first.
            temporary.publish(&sidecar)?;
        }
        let mut child_file = File::options().read(true).write(writable).open(&sidecar)?;
        let child_header = Header::read(&mut child_file)?;
        if child_header.compressed
            || child_header.parent != header.sha1
            || child_header.logical != header.logical
            || child_header.hunk != header.hunk
            || child_header.geometry != header.geometry
        {
            return Err(invalid(
                "CHD companion does not match its parent or is compressed",
            ));
        }
        let file = SharedFile::new(child_file, &child_header, writable)?;
        let image =
            Chd::open_custom(file.clone(), writable, Some(&parent)).map_err(native_error)?;
        Ok(Self {
            image,
            parent: Some(parent),
            file,
            parent_file: Some(parent_file),
            logical: header.logical,
            poisoned: false,
            invalidate_hashes: child_header.sha1 != [0; 20] || child_header.raw_sha1 != [0; 20],
        })
    }

    pub fn logical_bytes(&self) -> u64 {
        self.logical
    }

    pub fn validate_writable(&self, path: &Path) -> io::Result<()> {
        if self.parent.is_some() {
            File::options()
                .read(true)
                .write(true)
                .open(companion_path(path))?;
        } else {
            let info = self.image.info().map_err(native_error)?;
            if info.compressed {
                if info.sha1 == [0; 20] {
                    return Err(invalid(
                        "Compressed CHD needs a nonzero SHA-1 for a writable child",
                    ));
                }
            } else {
                File::options().read(true).write(true).open(path)?;
            }
        }
        Ok(())
    }

    fn ready(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "CHD write failed; detach and reopen before further I/O",
            ));
        }
        self.file.check_length()?;
        if let Some(parent) = &self.parent_file {
            parent.check_length()?;
        }
        Ok(())
    }

    pub fn read(&mut self, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
        self.ready()?;
        // The native API accepts u32 lengths. Chunk without truncating usize.
        for (i, chunk) in bytes.chunks_mut(1024 * 1024).enumerate() {
            self.image
                .read_bytes(offset + (i * 1024 * 1024) as u64, chunk)
                .map_err(native_error)?;
        }
        Ok(())
    }

    pub fn write(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.ready()?;
        let result = (|| {
            if self.invalidate_hashes {
                // Mutable uncompressed CHDs must not retain checksums for old
                // data. Clear only this file's raw/overall hashes, preserving
                // the parent identity at 104..124. Persist this before data.
                let mut state = self.file.0.borrow_mut();
                state.file.seek(SeekFrom::Start(64))?;
                state.file.write_all(&[0; 40])?;
                state.file.sync_data()?;
                self.invalidate_hashes = false;
            }
            if self.parent.is_some() {
                self.claim_zeroed_hunks(offset, bytes)?;
            }
            for (i, chunk) in bytes.chunks(1024 * 1024).enumerate() {
                self.image
                    .write_bytes(offset + (i * 1024 * 1024) as u64, chunk)
                    .map_err(native_error)?;
            }
            Ok(())
        })();
        if result.is_err() {
            // Native caches/maps can have advanced before a host I/O failure.
            // Never turn a retry into GOOD using that stale in-memory state.
            self.poisoned = true;
            if let Some((kind, message)) = self.file.0.borrow_mut().error.take() {
                return Err(io::Error::new(kind, message));
            }
        }
        result
    }

    fn claim_zeroed_hunks(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let size = u64::from(self.image.hunk_bytes());
        let end = offset + bytes.len() as u64;
        let mut buffer = Vec::new();
        for hunk in offset / size..end.div_ceil(size) {
            let start = hunk * size;
            let lo = (offset.max(start) - start) as usize;
            let hi = (end.min(start + size) - start) as usize;
            let input = &bytes[(start + lo as u64 - offset) as usize..][..hi - lo];
            if input.iter().any(|b| *b != 0)
                || self
                    .image
                    .hunk_info(hunk as u32)
                    .map_err(native_error)?
                    .compbytes
                    != 0
            {
                continue;
            }
            buffer.resize(size as usize, 0);
            self.image
                .read_hunk(hunk as u32, &mut buffer)
                .map_err(native_error)?;
            if buffer[lo..hi].iter().all(|b| *b == 0)
                || buffer[..lo].iter().chain(&buffer[hi..]).any(|b| *b != 0)
            {
                continue;
            }
            // MAME skips an all-zero write to an unallocated v5 hunk, even if
            // its parent is nonzero. Allocate the inherited data first, then
            // the ordinary write can replace it with zeroes in place.
            self.image
                .write_hunk(hunk as u32, &buffer)
                .map_err(native_error)?;
        }
        Ok(())
    }

    pub fn sync(&mut self) -> io::Result<()> {
        self.ready()?;
        let result = self.file.sync();
        self.poisoned |= result.is_err();
        result
    }
}

#[derive(Clone)]
struct SharedFile(Rc<RefCell<FileState>>);
struct FileState {
    file: File,
    length: u64,
    writable: bool,
    map: Range<u64>,
    error: Option<(io::ErrorKind, String)>,
}

impl SharedFile {
    fn new(file: File, header: &Header, writable: bool) -> io::Result<Self> {
        let length = file.metadata()?.len();
        Ok(Self(Rc::new(RefCell::new(FileState {
            file,
            length,
            writable,
            map: header.map.clone(),
            error: None,
        }))))
    }
    fn check_length(&self) -> io::Result<()> {
        let state = self.0.borrow();
        if state.file.metadata()?.len() != state.length {
            return Err(invalid("CHD size changed while attached"));
        }
        Ok(())
    }
    fn sync(&self) -> io::Result<()> {
        self.0.borrow().file.sync_data()
    }
}

impl Read for SharedFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().file.read(bytes)
    }
}
impl Seek for SharedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().file.seek(position)
    }
}
impl Write for SharedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.0.borrow_mut();
        let result = (|| {
            if !state.writable {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Read-only CHD",
                ));
            }
            let position = state.file.stream_position()?;
            let end = position
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("CHD file offset overflow"))?;
            if position < state.map.end && end > state.map.start {
                // A durable map must never point at an undurable new hunk.
                // libchdman writes hunk data before its map entry. Flush that
                // data here; the command's final sync then persists the map.
                state.file.sync_data()?;
            }
            let count = state.file.write(bytes)?;
            state.length = state.length.max(position + count as u64);
            Ok(count)
        })();
        if let Err(error) = &result {
            state.error = Some((error.kind(), error.to_string()));
        }
        result
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sync()
    }
}

pub(super) fn create_blank(path: &Path, logical: u64) -> io::Result<()> {
    ensure_absent(path)?;
    let mut hunk = 4096_u32;
    while logical.div_ceil(u64::from(hunk)) > MAX_HUNKS {
        hunk *= 2;
    }
    let mut chd =
        Chd::create(native_path(path)?, logical, hunk, 512, [0; 4]).map_err(native_error)?;
    let sectors = logical / 512;
    let heads = if sectors % 16 == 0 { 16 } else { 1 };
    let per_track = if sectors % (heads * 64) == 0 { 64 } else { 1 };
    let cylinders = sectors / heads / per_track;
    let metadata = format!("CYLS:{cylinders},HEADS:{heads},SECS:{per_track},BPS:512\0");
    chd.write_metadata(GDDD, 0, metadata.as_bytes(), 1)
        .map_err(native_error)
}

fn native_path(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| invalid("CHD creation requires a UTF-8 destination path"))
}
fn native_error(error: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("CHD: {error:?}"))
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_write_error_poisons_cache_until_reopen() {
        let staging =
            TemporaryImage::new(&std::env::temp_dir().join("helios-chd-failure.chd")).unwrap();
        create_blank(&staging.path, 8192).unwrap();
        let mut disk = Disk::open(&staging.path, File::open(&staging.path).unwrap(), true).unwrap();
        // Simulate a host rejecting writes after successful attachment, without
        // depending on root permissions, a full host disk or unsafe fd tricks.
        disk.file.0.borrow_mut().file = File::open(&staging.path).unwrap();
        assert!(disk.write(0, &[0x37; 512]).is_err());
        assert!(disk.poisoned);
        disk.file.0.borrow_mut().file = File::options()
            .read(true)
            .write(true)
            .open(&staging.path)
            .unwrap();
        assert!(disk.write(0, &[0x37; 512]).is_err());
        assert!(disk.read(0, &mut [0; 512]).is_err());
        assert!(disk.sync().is_err());
        drop(disk);
        let mut reopened =
            Disk::open(&staging.path, File::open(&staging.path).unwrap(), true).unwrap();
        let mut bytes = [1; 512];
        reopened.read(0, &mut bytes).unwrap();
        assert_eq!(bytes, [0; 512]);
        reopened.write(0, &[0x37; 512]).unwrap();
        reopened.sync().unwrap();
    }
}
