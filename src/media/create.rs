use super::{validate_size, ImageFormat, MediaInfo, MediaKind};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Create an unpartitioned, zero-filled disk, refusing to overwrite any path.
/// CHDs are sparse, uncompressed v5 containers; raw images are sparse files.
pub fn create_disk(
    destination: impl AsRef<Path>,
    format: ImageFormat,
    logical_bytes: u64,
) -> io::Result<MediaInfo> {
    validate_size(logical_bytes, MediaKind::Disk)?;
    let destination = destination.as_ref();
    if format == ImageFormat::Raw
        && destination
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("chd"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A raw image cannot use the .chd extension",
        ));
    }
    ensure_absent(destination)?;
    ensure_absent(&super::companion_path(destination))?;
    let temporary = TemporaryImage::new(destination)?;
    match format {
        ImageFormat::Raw => File::create(&temporary.path)?.set_len(logical_bytes)?,
        ImageFormat::Chd => {
            #[cfg(feature = "chd")]
            super::chd::create_blank(&temporary.path, logical_bytes)?;
            #[cfg(not(feature = "chd"))]
            return Err(super::unsupported_chd());
        }
    }
    temporary.publish(destination)?;
    Ok(MediaInfo {
        kind: MediaKind::Disk,
        format,
        logical_bytes,
    })
}

pub(super) fn ensure_absent(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Refusing to overwrite {}", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// A private directory beside the destination keeps native creation away from
/// existing files and on the same filesystem as the eventual no-clobber link.
pub(super) struct TemporaryImage {
    pub path: PathBuf,
    directory: PathBuf,
}

impl TemporaryImage {
    pub fn new(destination: &Path) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = parent_directory(destination);
        if destination.file_name().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Missing image filename",
            ));
        }
        for _ in 0..128 {
            let directory = parent.join(format!(
                ".helios-image-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        path: directory.join("image.chd"),
                        directory,
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Cannot reserve image staging directory",
        ))
    }

    pub fn publish(&self, destination: &Path) -> io::Result<()> {
        File::options()
            .read(true)
            .write(true)
            .open(&self.path)?
            .sync_all()?;
        // hard_link, unlike rename, fails if a file or dangling symlink exists.
        fs::hard_link(&self.path, destination)?;
        #[cfg(unix)]
        File::open(parent_directory(destination))?
            .sync_all()
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "Created {}, but directory sync failed: {e}",
                        destination.display()
                    ),
                )
            })?;
        Ok(())
    }
}

impl Drop for TemporaryImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
