use helios::bus::Bus;
use helios::firmware::{openbios_nvram, OpenBiosConfig, FW_CFG_BASE};
use helios::machine::MachineProfile;
use helios::mmu::{MemoryError, Srmmu, Width, PCR_EN};
use helios::system_bus::SystemBus;

fn bus() -> SystemBus {
    let mut bus = SystemBus::default();
    bus.install_openbios(OpenBiosConfig::new(MachineProfile::Ss5, 128 << 20, 1, true));
    bus
}

fn read_entry(bus: &mut SystemBus, key: u16, count: usize) -> Vec<u8> {
    bus.write16(FW_CFG_BASE, key).unwrap();
    (0..count)
        .map(|_| bus.read8(FW_CFG_BASE + 2).unwrap())
        .collect()
}

#[test]
fn configuration_is_explicit_and_streams_have_the_documented_endianness() {
    assert!(SystemBus::default().read8(FW_CFG_BASE + 2).is_err());
    let mut b = bus();
    assert_eq!(read_entry(&mut b, 0, 6), b"QEMU\0\0");
    assert_eq!(read_entry(&mut b, 1, 4), [1, 0, 0, 0]);
    assert_eq!(read_entry(&mut b, 3, 8), [0, 0, 0, 8, 0, 0, 0, 0]);
    assert_eq!(read_entry(&mut b, 4, 2), [1, 0]);
    assert_eq!(read_entry(&mut b, 5, 4), [1, 0, 0, 0]);
    assert_eq!(read_entry(&mut b, 6, 2), [32, 0]);
    assert_eq!(read_entry(&mut b, 0xffff, 8), [0; 8]);
    assert_eq!(read_entry(&mut b, 0x8001, 2), [0x80, 4]);
    assert_eq!(read_entry(&mut b, 0x8002, 2), [0x84, 3]);
    assert_eq!(read_entry(&mut b, 0x19, 4), [0; 4]);
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        b.install_openbios(OpenBiosConfig::new(profile, 512 << 20, 2, false));
        assert_eq!(read_entry(&mut b, 6, 2), [65, 0]);
        assert_eq!(read_entry(&mut b, 5, 2), [2, 0]);
        assert_eq!(read_entry(&mut b, 4, 2), [0, 0]);
    }
}

#[test]
fn bad_widths_and_atomics_do_not_consume_or_reselect_a_stream() {
    let mut b = bus();
    assert_eq!(read_entry(&mut b, 0, 1), b"Q");
    assert!(b.read16(FW_CFG_BASE + 2).is_err());
    assert!(b.read32(FW_CFG_BASE).is_err());
    assert!(b.read_doubleword(FW_CFG_BASE).is_err());
    assert!(b.write8(FW_CFG_BASE, 1).is_err());
    assert!(b.write32(FW_CFG_BASE, 1).is_err());
    assert!(b.write_doubleword(FW_CFG_BASE, [0, 0]).is_err());
    assert!(b.swap32(FW_CFG_BASE, 1).is_err());
    assert!(b.ldstub8(FW_CFG_BASE + 2).is_err());
    assert_eq!(b.read8(FW_CFG_BASE + 2).unwrap(), b'E');
    b.write8(FW_CFG_BASE + 2, 0xff).unwrap(); // read-only stream
    assert_eq!(b.read8(FW_CFG_BASE + 2).unwrap(), b'M');
    assert_eq!(read_entry(&mut b, 0, 1), b"Q"); // selection rewinds
    b.reset_devices();
    assert_eq!(b.read8(FW_CFG_BASE + 2).unwrap(), b'Q');
}

#[test]
fn ss5_extended_asi_is_admitted_only_for_the_installed_service_page() {
    let mut m = Srmmu::default();
    let mut b = SystemBus::default();
    assert!(matches!(
        m.store(&mut b, 0x510, 0x2d, Width::Half, 0),
        Err(MemoryError::UnsupportedAsi { .. })
    ));
    b.install_openbios(OpenBiosConfig::new(MachineProfile::Ss5, 128 << 20, 1, true));
    m.store(&mut b, 0x510, 0x2d, Width::Half, 0).unwrap();
    assert_eq!(
        m.load(&mut b, 0x512, 0x2d, Width::Byte).unwrap(),
        u32::from(b'Q')
    );
    assert!(matches!(
        m.load(&mut b, 0x1512, 0x2d, Width::Byte),
        Err(MemoryError::UnsupportedAsi { .. })
    ));
    assert!(matches!(
        m.load(&mut b, 0x512, 0x2e, Width::Byte),
        Err(MemoryError::UnsupportedAsi { .. })
    ));
}

#[test]
fn ss5_page_mapping_keeps_fw_cfg_separate_from_ram_and_diagnostic_edits() {
    for enabled in [false, true] {
        let mut b = if enabled { bus() } else { SystemBus::default() };
        b.map_ram(0, 0x10000).unwrap();
        for (address, word) in [
            (0x1000, 0x201),
            (0x2048, 0x301),
            (0x3034, 0x401),
            (0x4014, 0xd000_001e),
        ] {
            b.write32(address, word).unwrap();
        }
        b.write32(0x510, 0xaabb_ccdd).unwrap();
        let mut m = Srmmu::default();
        m.store(&mut b, 0x100, 4, Width::Word, 0x100).unwrap();
        // Freeze the entry in slot zero for the diagnostic-edit check.
        m.store(&mut b, 0x1000, 4, Width::Word, 0x40).unwrap();
        m.store(&mut b, 0, 4, Width::Word, PCR_EN).unwrap();
        assert_eq!(
            m.load(&mut b, 0x1234_5512, 11, Width::Byte).unwrap(),
            if enabled { u32::from(b'Q') } else { 0xcc }
        );
        assert_eq!(b.read32(0x510).unwrap(), 0xaabb_ccdd);
        assert_eq!(b.read32(0x4014).unwrap(), 0xd000_003e); // R update preserves PPN
        if enabled {
            // A diagnostic PTE edit describes hardware bits, not a firmware service.
            m.store(&mut b, 0, 6, Width::Word, 0x1e).unwrap();
            assert_eq!(m.load(&mut b, 0x1234_5512, 11, Width::Byte).unwrap(), 0xcc);
            m.store(&mut b, 0x400, 3, Width::Word, 0).unwrap();
            assert_eq!(
                m.load(&mut b, 0x1234_5512, 11, Width::Byte).unwrap(),
                u32::from(b'E')
            );
        }
    }
}

#[test]
fn chrp_environment_preserves_sun_identity_and_clock() {
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        let old = profile.default_nvram().image();
        let new = openbios_nvram(profile).image();
        assert_eq!(&new[0x1fd0..], &old[0x1fd0..]);
        assert_eq!(&new[2..10], b"\x01\xfdsystem");
        let sum: u32 = new[..16]
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, b)| u32::from(*b))
            .sum();
        assert_eq!(u32::from(new[1]), (sum & 255) + (sum >> 8));
        assert!(new[16..].starts_with(b"auto-boot?=false\0boot-device=disk\0\0"));
    }
}

// Minimal independently constructed ELF32/SPARC executable: one eight-byte
// branch/delay-slot loop, followed by eight bytes of BSS.
fn elf_fixture() -> Vec<u8> {
    let mut image = vec![0; 92];
    image[..7].copy_from_slice(b"\x7fELF\x01\x02\x01");
    for (offset, value) in [(16, 2u16), (18, 2), (40, 52), (42, 32), (44, 1)] {
        image[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, 0xffd0_0000),
        (28, 52),
        (52, 1),
        (56, 84),
        (60, 0xffd0_0000),
        (64, 0xffd0_0000),
        (68, 8),
        (72, 16),
        (76, 5),
        (80, 4),
    ] {
        image[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    image[84..].copy_from_slice(&[0x10, 0x80, 0, 0, 1, 0, 0, 0]);
    image
}

#[test]
fn elf_conversion_bounds_checks_endianness_addresses_and_zero_fills_bss() {
    use helios::firmware::decode_openbios_elf;
    let valid = elf_fixture();
    assert_eq!(
        decode_openbios_elf(&valid).unwrap(),
        [0x10, 0x80, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    for (offset, value) in [
        (5, 1),
        (19, 43),
        (24, 0),
        (60, 0),
        (64, 0),
        (69, 1),
        (73, 32),
        (28, 255),
        (45, 2),
        (43, 16),
    ] {
        let mut bad = valid.clone();
        bad[offset] = value;
        assert!(decode_openbios_elf(&bad).is_err(), "offset {offset}");
    }
    for end in [0, 7, 32, 51, 83, 91] {
        assert!(decode_openbios_elf(&valid[..end]).is_err());
    }
}

#[test]
fn configured_firmware_and_its_errors_always_take_precedence() {
    use helios::firmware::{FirmwareImage, FirmwareKind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "helios-firmware-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _scratch = Scratch(dir.clone());
    let obp = dir.join("chosen.rom");
    let replacement = dir.join("fallback.elf");
    let absent = dir.join("missing.rom");
    std::fs::write(&obp, [0x10, 0x80, 0, 0, 1, 0, 0, 0]).unwrap();
    std::fs::write(&replacement, elf_fixture()).unwrap();
    let candidates = [absent.clone(), replacement.clone()];
    let image = FirmwareImage::resolve(Some(&obp), &candidates).unwrap();
    assert_eq!(image.kind, FirmwareKind::Obp);
    assert_eq!(image.path, obp);
    assert!(FirmwareImage::resolve(Some(&absent), &candidates).is_err());
    std::fs::write(&obp, []).unwrap();
    assert!(FirmwareImage::resolve(Some(&obp), &candidates).is_err());
    let fallback = FirmwareImage::resolve(None, &candidates).unwrap();
    assert_eq!(fallback.kind, FirmwareKind::OpenBios);
    assert_eq!(fallback.path, replacement);
    std::fs::write(&absent, b"corrupt").unwrap();
    assert!(FirmwareImage::resolve(None, &candidates).is_err());
    assert!(FirmwareImage::resolve(None, &[]).is_err());
}

#[test]
fn firmware_environment_validation_and_names_protect_original_obp_state() {
    use helios::firmware::{validate_openbios_nvram, validate_openbios_ram, FirmwareKind};
    use helios::machine::InstalledVideo;
    let path = std::path::Path::new("named workstation.nvram.ss20");
    assert_eq!(FirmwareKind::Obp.nvram_path(path), path);
    assert_eq!(
        FirmwareKind::OpenBios.nvram_path(path),
        std::path::Path::new("named workstation.nvram.ss20.openbios")
    );
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        assert!(validate_openbios_nvram(&profile.default_nvram()).is_err());
        assert!(validate_openbios_nvram(&openbios_nvram(profile)).is_ok());
        let mut bytes = openbios_nvram(profile).image();
        bytes[1] ^= 1;
        assert!(validate_openbios_nvram(
            &helios::devices::nvram::Nvram::from_image(&bytes).unwrap()
        )
        .is_err());
        assert!(validate_openbios_ram(profile, 128 << 20, InstalledVideo::None).is_ok());
        assert!(validate_openbios_ram(profile, 16 << 20, InstalledVideo::None).is_err());
    }
    for (profile, gap, maximum) in [
        (MachineProfile::Ss5, 48, 256),
        (MachineProfile::Ss20, 48, 512),
        (MachineProfile::Axil320, 96, 512),
    ] {
        assert!(validate_openbios_ram(profile, gap << 20, InstalledVideo::None).is_err());
        assert!(validate_openbios_ram(profile, maximum << 20, InstalledVideo::None).is_ok());
    }
}

#[test]
fn replacement_rom_relocation_does_not_change_original_obp_boot_aliases() {
    for enabled in [false, true] {
        let mut b = if enabled { bus() } else { SystemBus::default() };
        b.map_rom(0x7000_0000, &[0x12, 0x34, 0x56, 0x78]).unwrap();
        let mut m = Srmmu::reset();
        assert_eq!(m.load(&mut b, 0, 9, Width::Word).unwrap(), 0x1234_5678);
        let relocated = m.load(&mut b, 0xffd0_0000, 9, Width::Word);
        if enabled {
            assert_eq!(relocated.unwrap(), 0x1234_5678);
        } else {
            assert!(relocated.is_err());
        }
    }
}
