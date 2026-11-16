use helios::devices::scsi::ScsiBus;
use helios::media::{inspect, ImageFormat, Media, MediaKind};
use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-media-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn image(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn raw_logical_transfers_are_durable_and_preserve_neighbours() {
    let dir = Directory::new();
    let mut expected: Vec<_> = (0..2048).map(|i| (i * 17 + i / 512) as u8).collect();
    let path = dir.image("disk.raw", &expected);
    let mut media = Media::open(&path, MediaKind::Disk, true).unwrap();
    assert!(media.writable());
    assert_eq!(media.info().logical_bytes, 2048);
    assert_eq!(media.info().format, ImageFormat::Raw);
    media.write_all_at(507, &[0xa7; 19]).unwrap();
    media.sync().unwrap();
    expected[507..526].fill(0xa7);
    assert_eq!(fs::read(&path).unwrap(), expected);
    drop(media);
    let mut media = Media::open(&path, MediaKind::Disk, false).unwrap();
    let mut actual = [0; 1024];
    media.read_exact_at(512, &mut actual).unwrap();
    assert_eq!(actual, expected[512..1536]);
}

#[test]
fn inspection_and_read_only_open_never_write_or_create_companions() {
    let dir = Directory::new();
    let path = dir.image("disk.raw", &[0x37; 512]);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(inspect(&path, MediaKind::Disk).unwrap().logical_bytes, 512);
    let mut media = Media::open(&path, MediaKind::Disk, false).unwrap();
    assert_eq!(
        media.write_all_at(0, &[0; 512]).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    media.sync().unwrap();
    assert_eq!(fs::read(&path).unwrap(), [0x37; 512]);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn ranges_and_external_truncation_cannot_grow_a_raw_disk() {
    let dir = Directory::new();
    let path = dir.image("disk.raw", &[0x51; 1024]);
    let mut media = Media::open(&path, MediaKind::Disk, true).unwrap();
    assert!(media.write_all_at(1023, &[1, 2]).is_err());
    assert!(media.write_all_at(u64::MAX, &[1]).is_err());
    assert_eq!(fs::read(&path).unwrap(), [0x51; 1024]);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(512)
        .unwrap();
    assert!(media.write_all_at(512, &[0; 512]).is_err());
    assert_eq!(fs::read(&path).unwrap(), [0x51; 512]);
    // READ(10) with count zero is a no-op even when its LBA is out of range.
    media.read_exact_at(u64::MAX, &mut []).unwrap();
}

#[test]
fn recognized_chd_never_falls_back_to_raw_guest_writes() {
    let dir = Directory::new();
    let mut container = [0; 512];
    container[..8].copy_from_slice(b"MComprHD");
    for name in ["renamed.raw", "disk.CHD"] {
        let path = dir.image(name, &container);
        for writable in [false, true] {
            assert!(Media::open(&path, MediaKind::Disk, writable).is_err());
        }
        let mut bus = ScsiBus::default();
        assert!(bus.attach_disk_rw(3, &path).is_err());
        assert!(bus.target(3).is_none());
        assert_eq!(fs::read(path).unwrap(), container);
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn malformed_chd_names_and_invalid_raw_geometry_are_rejected() {
    let dir = Directory::new();
    for (name, bytes, kind) in [
        ("empty.raw", vec![], MediaKind::Disk),
        ("partial.raw", vec![0; 513], MediaKind::Disk),
        ("partial.iso", vec![0; 512], MediaKind::Cdrom),
        ("short.chd", b"MComprH".to_vec(), MediaKind::Disk),
        ("fake.CHD", vec![0; 512], MediaKind::Disk),
    ] {
        let path = dir.image(name, &bytes);
        assert!(inspect(&path, kind).is_err(), "{name}");
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    assert!(inspect(&dir.0, MediaKind::Disk).is_err());
}

#[test]
fn raw_cd_is_read_only_and_byte_addressable_for_small_logical_blocks() {
    let dir = Directory::new();
    let expected: Vec<_> = (0..4096).map(|i| (i / 512) as u8).collect();
    let path = dir.image("cd.iso", &expected);
    assert!(Media::open(&path, MediaKind::Cdrom, true).is_err());
    let mut media = Media::open(&path, MediaKind::Cdrom, false).unwrap();
    let mut actual = [0; 1024];
    media.read_exact_at(1536, &mut actual).unwrap();
    assert_eq!(&actual[..512], &[3; 512]);
    assert_eq!(&actual[512..], &[4; 512]);
    assert_eq!(fs::read(&path).unwrap(), expected);
}

#[test]
fn raw_creation_is_explicit_zero_filled_and_never_clobbers() {
    use helios::media::{companion_path, create_disk};
    let directory = Directory::new();
    let path = directory.0.join("new.raw");
    for size in [0, 513, u64::MAX] {
        assert!(create_disk(&path, ImageFormat::Raw, size).is_err());
        assert!(!path.exists());
    }
    create_disk(&path, ImageFormat::Raw, 8192).unwrap();
    assert_eq!(fs::read(&path).unwrap(), vec![0; 8192]);
    fs::write(&path, [0x21; 512]).unwrap();
    assert_eq!(
        create_disk(&path, ImageFormat::Raw, 8192)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&path).unwrap(), [0x21; 512]);
    let other = directory.0.join("other.raw");
    fs::write(companion_path(&other), b"existing").unwrap();
    assert!(create_disk(&other, ImageFormat::Raw, 8192).is_err());
    assert!(!other.exists());
    assert!(create_disk(directory.0.join("bad.chd"), ImageFormat::Raw, 8192).is_err());
}

#[test]
fn create_disk_cli_checks_arguments_before_creating_any_file() {
    let directory = Directory::new();
    let output = directory.0.join("new.raw");
    for args in [
        vec!["--format", "raw"],
        vec!["--format", "bogus", "--size-mib", "1"],
        vec!["--format", "raw", "--size-mib", "0"],
        vec!["--format", "raw", "--size-mib", "-1"],
        vec!["--format", "raw", "--size-mib", "18446744073709551615"],
        vec!["--format", "raw", "--size-mib", "1", "--size-mib", "2"],
    ] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_helios"))
            .arg("create-disk")
            .arg(&output)
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!output.exists());
    }
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_helios"))
        .arg("create-disk")
        .arg(&output)
        .args(["--size-mib", "1", "--format", "raw"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(fs::metadata(&output).unwrap().len(), 1024 * 1024);
}
