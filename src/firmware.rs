//! Replacement-firmware interfaces. These are opt-in emulator services, not
//! Sun device registers. Original OBP machines do not install them.
mod image;
use crate::bus::BusErrorKind;
use crate::devices::nvram::Nvram;
use crate::machine::{InstalledVideo, MachineProfile};
pub use image::{decode_openbios_elf, FirmwareImage, FirmwareKind};

pub const FW_CFG_BASE: u64 = 0x000d_0000_0510;
pub(crate) const FW_CFG_PAGE: u64 = FW_CFG_BASE & !0xfff;

/// Limits of the currently documented SPARC32 firmware build. Validate before
/// allocating guest RAM or attaching writable media. This is a firmware limit,
/// not a change to the board's supported DSIMM populations.
pub fn validate_openbios_ram(
    profile: MachineProfile,
    ram_bytes: usize,
    video: InstalledVideo,
) -> Result<(), String> {
    let layout = profile
        .ram_layout(ram_bytes, video)
        .map_err(|e| e.to_string())?;
    if ram_bytes < 32 * 1024 * 1024 {
        return Err("OpenBIOS requires at least 32 MiB RAM (its allocator reserves the first 16 MiB). Original OBP supports smaller valid populations.".into());
    }
    let mut partial = false;
    let span = if profile == MachineProfile::Ss5 {
        32 << 20
    } else {
        64 << 20
    };
    for size in layout.bank_sizes() {
        if partial && size != 0 {
            return Err("This OpenBIOS build requires contiguous DRAM. Choose a population with no gap between installed modules (128 MiB works on all models), or select original OBP.".into());
        }
        partial |= size < span;
    }
    Ok(())
}

pub fn validate_openbios_nvram(nvram: &Nvram) -> Result<(), String> {
    let bytes = nvram.image();
    let size = usize::from(u16::from_be_bytes([bytes[2], bytes[3]])) * 16;
    let mut checksum = u16::from(bytes[0]);
    for byte in &bytes[2..16] {
        checksum += u16::from(*byte);
        if checksum > 255 {
            checksum = (checksum + 1) & 255;
        }
    }
    if bytes[0] != 0x70
        || bytes[1] != checksum as u8
        || !(32..=0x1fd0).contains(&size)
        || &bytes[4..10] != b"system"
    {
        return Err("OpenBIOS needs its own CHRP NVRAM environment; this image is invalid or belongs to OBP. Use a separate fresh OpenBIOS NVRAM file.".into());
    }
    Ok(())
}

/// QEMU's SPARC32 fw_cfg contract used by the pinned OpenBIOS image.
/// Control is a big-endian halfword; entry contents are byte streams, with
/// little-endian integers. No DMA, writable entries or host pointers exist.
#[derive(Debug)]
pub struct OpenBiosConfig {
    entries: Vec<(u16, Vec<u8>)>,
    selector: u16,
    offset: usize,
}

impl OpenBiosConfig {
    pub fn new(profile: MachineProfile, ram_bytes: usize, cpus: u16, serial: bool) -> Self {
        let machine_id = if profile == MachineProfile::Ss5 {
            32u16
        } else {
            65
        };
        let mut entries = vec![
            (0, b"QEMU".to_vec()),
            (1, 1u32.to_le_bytes().to_vec()),
            (2, vec![0; 16]),
            (3, (ram_bytes as u64).to_le_bytes().to_vec()),
            (4, u16::from(serial).to_le_bytes().to_vec()),
            (5, cpus.to_le_bytes().to_vec()),
            (6, machine_id.to_le_bytes().to_vec()),
            (12, u16::from(b'c').to_le_bytes().to_vec()),
            (15, cpus.to_le_bytes().to_vec()),
            (0x19, 0u32.to_be_bytes().to_vec()),
            (0x8000, 8u16.to_le_bytes().to_vec()),
            (0x8001, 1152u16.to_le_bytes().to_vec()),
            (0x8002, 900u16.to_le_bytes().to_vec()),
        ];
        for key in 7..=11 {
            entries.push((key, vec![0; 4]));
        }
        Self {
            entries,
            selector: 0,
            offset: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.selector = 0;
        self.offset = 0;
    }

    pub(crate) fn access(
        &mut self,
        address: u64,
        bytes: u8,
        write: Option<u32>,
    ) -> Result<u32, BusErrorKind> {
        match (address, bytes, write) {
            (FW_CFG_BASE, 2, Some(value)) => {
                self.selector = value as u16;
                self.offset = 0;
                Ok(0)
            }
            (FW_CFG_BASE, 2, None) => Ok(0),
            (address, 1, None) if address == FW_CFG_BASE + 2 => {
                let value = self
                    .entries
                    .iter()
                    .find(|(key, _)| *key == self.selector)
                    .and_then(|(_, value)| value.get(self.offset))
                    .copied();
                if value.is_some() {
                    self.offset += 1;
                }
                Ok(u32::from(value.unwrap_or(0)))
            }
            (address, 1, Some(_)) if address == FW_CFG_BASE + 2 => Ok(0),
            _ => Err(BusErrorKind::Unsupported(
                "OpenBIOS fw_cfg access width or offset",
            )),
        }
    }
}

/// A fresh CHRP environment for replacement firmware; never convert an OBP
/// environment in place. IDPROM and the deterministic clock retain the profile
/// identity. Frontends must keep replacement-firmware NVRAM in a separate file.
pub fn openbios_nvram(profile: MachineProfile) -> Nvram {
    let mut image = profile.default_nvram().image();
    image[..0x1fd0].fill(0);
    image[0] = 0x70; // CHRP system partition
    image[2..4].copy_from_slice(&0x01fdu16.to_be_bytes());
    image[4..10].copy_from_slice(b"system");
    let vars = b"auto-boot?=false\0boot-device=disk\0\0";
    image[16..16 + vars.len()].copy_from_slice(vars);
    let sum: u16 = image[..16].iter().map(|v| u16::from(*v)).sum();
    image[1] = ((sum & 255) + (sum >> 8)) as u8;
    Nvram::from_image(&image).expect("profile NVRAM and unchanged clock")
}
