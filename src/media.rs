//! Host media boundary. Guest transfers address logical bytes, not containers.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(feature = "chd")]
mod chd;
mod create;
pub use create::create_disk;

/// Persistent child for a compressed disk. The base filename is retained.
pub fn companion_path(path: impl AsRef<Path>) -> std::path::PathBuf {
    let mut path = path.as_ref().as_os_str().to_os_string();
    path.push(".diff.chd");
    path.into()
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Raw,
    Chd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaInfo {
    pub kind: MediaKind,
    pub format: ImageFormat,
    pub logical_bytes: u64,
}

/// Inspect existing media without writing to it or creating companion files.
pub fn inspect(path: impl AsRef<Path>, kind: MediaKind) -> io::Result<MediaInfo> {
    Ok(Media::open(path, kind, false)?.info())
}

/// Validate a draft attachment without creating a child or writing media.
/// A new compressed-disk child is created only on writable attachment; a
/// later creation/permission failure is still reported by `Media::open`.
pub fn validate_attachment(
    path: impl AsRef<Path>,
    kind: MediaKind,
    writable: bool,
) -> io::Result<MediaInfo> {
    let path = path.as_ref();
    let media = Media::open(path, kind, false)?;
    if writable {
        if kind == MediaKind::Cdrom {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "CD-ROM is read-only",
            ));
        }
        match &media.backend {
            Backend::Raw(_) => {
                File::options().read(true).write(true).open(path)?;
            }
            #[cfg(feature = "chd")]
            Backend::Chd(disk) => disk.validate_writable(path)?,
        }
    }
    Ok(media.info())
}

#[derive(Debug)]
pub struct Media {
    backend: Backend,
    info: MediaInfo,
    writable: bool,
}

#[derive(Debug)]
enum Backend {
    Raw(File),
    #[cfg(feature = "chd")]
    Chd(chd::Disk),
}

impl Media {
    /// Open existing media. This never creates, truncates or replaces a base.
    pub fn open(path: impl AsRef<Path>, kind: MediaKind, writable: bool) -> io::Result<Self> {
        let path = path.as_ref();
        if writable && kind == MediaKind::Cdrom {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "CD-ROM is read-only",
            ));
        }
        let mut file = File::open(path)?;
        match identify(&mut file, path)? {
            ImageFormat::Chd => {
                #[cfg(feature = "chd")]
                {
                    if kind != MediaKind::Disk {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "CHD CD-ROMs are not supported; use a cooked ISO",
                        ));
                    }
                    let disk = chd::Disk::open(path, file, writable)?;
                    let logical_bytes = disk.logical_bytes();
                    return Ok(Self {
                        backend: Backend::Chd(disk),
                        info: MediaInfo {
                            kind,
                            format: ImageFormat::Chd,
                            logical_bytes,
                        },
                        writable,
                    });
                }
                #[cfg(not(feature = "chd"))]
                return Err(unsupported_chd());
            }
            ImageFormat::Raw => {}
        }
        if writable {
            file = File::options().read(true).write(true).open(path)?;
            // A path replaced between inspection and reopening must not turn
            // container bytes into writable raw guest sectors.
            if identify(&mut file, path)? != ImageFormat::Raw {
                return Err(unsupported_chd());
            }
        }
        let logical_bytes = file.metadata()?.len();
        validate_size(logical_bytes, kind)?;
        Ok(Self {
            backend: Backend::Raw(file),
            info: MediaInfo {
                kind,
                format: ImageFormat::Raw,
                logical_bytes,
            },
            writable,
        })
    }

    pub fn info(&self) -> MediaInfo {
        self.info
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    pub fn read_exact_at(&mut self, offset: u64, bytes: &mut [u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        check_range(self.info.logical_bytes, offset, bytes.len())?;
        match &mut self.backend {
            #[cfg(feature = "chd")]
            Backend::Chd(disk) => disk.read(offset, bytes),
            Backend::Raw(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(bytes)
            }
        }
    }

    /// The caller stages a complete guest command before calling this method.
    /// Host failures can leave partial data; errors must propagate to the guest.
    pub fn write_all_at(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        if !self.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Media is read-only",
            ));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        check_range(self.info.logical_bytes, offset, bytes.len())?;
        match &mut self.backend {
            #[cfg(feature = "chd")]
            Backend::Chd(disk) => disk.write(offset, bytes),
            Backend::Raw(file) => {
                if file.metadata()?.len() != self.info.logical_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Raw image size changed while attached",
                    ));
                }
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(bytes)
            }
        }
    }

    /// Make preceding writes durable. Read-only attachments perform no writes.
    pub fn sync(&mut self) -> io::Result<()> {
        if !self.writable {
            return Ok(());
        }
        match &mut self.backend {
            Backend::Raw(file) => file.sync_data(),
            #[cfg(feature = "chd")]
            Backend::Chd(disk) => disk.sync(),
        }
    }
}

fn identify(file: &mut File, path: &Path) -> io::Result<ImageFormat> {
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Media must be a regular file",
        ));
    }
    file.rewind()?;
    let mut header = [0; 8];
    let signature = match file.read_exact(&mut header) {
        Ok(()) => &header == b"MComprHD",
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => false,
        Err(error) => return Err(error),
    };
    if signature {
        return Ok(ImageFormat::Chd);
    }
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("chd"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid or truncated CHD signature",
        ));
    }
    Ok(ImageFormat::Raw)
}

fn validate_size(bytes: u64, kind: MediaKind) -> io::Result<()> {
    let block = u64::from(kind.block_size());
    if bytes == 0 || bytes % block != 0 || bytes / block > 1_u64 << 32 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "SCSI media must contain whole logical blocks (512 bytes for disk, 2048 for CD-ROM), at most 2^32 blocks"));
    }
    Ok(())
}

fn check_range(length: u64, offset: u64, count: usize) -> io::Result<()> {
    if offset
        .checked_add(count as u64)
        .is_some_and(|end| end <= length)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Media transfer outside logical capacity",
        ))
    }
}

fn unsupported_chd() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "CHD decoding is unavailable in this build; container bytes cannot be used as a raw image",
    )
}
