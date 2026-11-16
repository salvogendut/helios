//! Host-side firmware selection shared by the CLI and graphical frontends.
//! Files are read once, before machine allocation or writable media attachment.
use super::openbios_nvram;
use crate::devices::nvram::Nvram;
use crate::machine::{MachineProfile, MAX_PROM_BYTES};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareKind {
    Obp,
    OpenBios,
}

impl FirmwareKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Obp => "OBP / custom PROM",
            Self::OpenBios => "OpenBIOS",
        }
    }

    pub fn default_nvram(self, profile: MachineProfile) -> Nvram {
        match self {
            Self::Obp => profile.default_nvram(),
            Self::OpenBios => openbios_nvram(profile),
        }
    }

    /// Keep existing OBP filenames unchanged. Replacement firmware must never
    /// interpret or overwrite the original firmware's environment.
    pub fn nvram_path(self, obp_path: &Path) -> PathBuf {
        if self == Self::Obp {
            return obp_path.to_owned();
        }
        let mut name = obp_path.as_os_str().to_owned();
        name.push(".openbios");
        name.into()
    }
}

#[derive(Debug)]
pub struct FirmwareImage {
    pub kind: FirmwareKind,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

impl FirmwareImage {
    /// An explicit PROM always wins, including its errors. Fallback candidates
    /// are searched only when the PROM path is omitted. An existing but invalid
    /// fallback also errors rather than silently choosing a different image.
    pub fn resolve(configured: Option<&Path>, fallbacks: &[PathBuf]) -> Result<Self, String> {
        if let Some(path) = configured {
            return Self::read(path, false);
        }
        for path in fallbacks {
            match std::fs::metadata(path) {
                Ok(_) => return Self::read(path, true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("OpenBIOS {}: {error}", path.display())),
            }
        }
        Err("No OBP image configured and no OpenBIOS fallback found. Build the documented Helios OpenBIOS image with tools/build_openbios.py, or select an original OBP image. HELIOS_OPENBIOS may specify the fallback ELF file.".into())
    }

    /// Runtime search, without downloads or changes to the user's configuration.
    pub fn load(configured: Option<&Path>) -> Result<Self, String> {
        if configured.is_some() {
            return Self::resolve(configured, &[]);
        }
        if let Some(path) = std::env::var_os("HELIOS_OPENBIOS") {
            // An explicitly configured fallback is authoritative, too.
            return Self::read(Path::new(&path), true);
        }
        let filename = "openbios-helios-sparc32.elf";
        let mut paths = vec![PathBuf::from("local/firmware").join(filename)];
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            paths.push(PathBuf::from(data).join("helios/firmware").join(filename));
        } else if let Some(home) = std::env::var_os("HOME") {
            paths.push(
                PathBuf::from(home)
                    .join(".local/share/helios/firmware")
                    .join(filename),
            );
        }
        paths.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("local/firmware")
                .join(filename),
        );
        paths.push(PathBuf::from("/usr/local/share/helios/firmware").join(filename));
        paths.push(PathBuf::from("/usr/share/helios/firmware").join(filename));
        Self::resolve(None, &paths)
    }

    fn read(path: &Path, require_openbios: bool) -> Result<Self, String> {
        let file = std::fs::File::open(path)
            .map_err(|error| format!("Firmware {}: {error}", path.display()))?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        // ELF headers/section metadata are outside the ROM load segment.
        const MAX_FILE: usize = MAX_PROM_BYTES + 65536;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE as u64 {
            return Err(format!(
                "Firmware {}: expected a nonempty regular file of at most {MAX_FILE} bytes",
                path.display()
            ));
        }
        let mut bytes = Vec::new();
        file.take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Firmware {}: {error}", path.display()))?;
        if bytes.is_empty() || bytes.len() > MAX_FILE {
            return Err(format!("Firmware {}: invalid file size", path.display()));
        }
        let kind = if bytes.starts_with(b"\x7fELF") {
            bytes = decode_openbios_elf(&bytes)
                .map_err(|error| format!("Firmware {}: {error}", path.display()))?;
            FirmwareKind::OpenBios
        } else if bytes.len() == 847872 && crc32(&bytes) == 0x7911_fd60 {
            // Recognition of the existing pinned raw conversion, not an
            // authenticity guarantee. Fetch/build helpers verify provenance.
            FirmwareKind::OpenBios
        } else if require_openbios {
            return Err(format!(
                "OpenBIOS {}: expected a SPARC32 OpenBIOS ELF image",
                path.display()
            ));
        } else if bytes.len() <= MAX_PROM_BYTES {
            FirmwareKind::Obp
        } else {
            return Err(format!(
                "Firmware {}: raw PROM exceeds {MAX_PROM_BYTES} bytes",
                path.display()
            ));
        };
        Ok(Self {
            kind,
            path: path.to_owned(),
            bytes,
        })
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

/// The supported OpenBIOS SPARC32 ELF contract: one big-endian SPARC load
/// segment, entry and load address ffd00000, bounded by the 1 MiB PROM.
/// BSS is zero-filled, with endian conversion only at the file boundary.
pub fn decode_openbios_elf(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    const BAD: &str = "invalid OpenBIOS ELF32 big-endian SPARC image";
    let half = |offset: usize| -> Result<u16, &'static str> {
        let b = bytes.get(offset..offset + 2).ok_or(BAD)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    };
    let word = |offset: usize| -> Result<u32, &'static str> {
        let b = bytes.get(offset..offset + 4).ok_or(BAD)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    if bytes.get(..7) != Some(b"\x7fELF\x01\x02\x01")
        || half(16)? != 2
        || half(18)? != 2
        || word(20)? != 1
        || word(24)? != 0xffd0_0000
        || half(40)? != 52
        || half(42)? != 32
    {
        return Err(BAD);
    }
    let start = word(28)? as usize;
    let end = start
        .checked_add(usize::from(half(44)?).checked_mul(32).ok_or(BAD)?)
        .ok_or(BAD)?;
    if end > bytes.len() || start < 52 {
        return Err(BAD);
    }
    let mut segment = None;
    for offset in (start..end).step_by(32) {
        if word(offset)? != 1 {
            continue;
        }
        if segment.is_some()
            || word(offset + 8)? != 0xffd0_0000
            || word(offset + 12)? != 0xffd0_0000
        {
            return Err(BAD);
        }
        let file = word(offset + 4)? as usize;
        let size = word(offset + 16)? as usize;
        let memory = word(offset + 20)? as usize;
        if size == 0 || size > memory || memory > MAX_PROM_BYTES {
            return Err(BAD);
        }
        let data = bytes
            .get(file..file.checked_add(size).ok_or(BAD)?)
            .ok_or(BAD)?;
        let mut image = vec![0; memory];
        image[..size].copy_from_slice(data);
        segment = Some(image);
    }
    segment.ok_or(BAD)
}
