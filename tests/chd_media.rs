#![cfg(feature = "chd")]
use helios::media::{companion_path, create_disk, inspect, ImageFormat, Media, MediaKind};
use libchdman_rs::{
    hd::{create_from_reader, HdCreateOptions},
    Chd, CHD_CODEC_ZLIB,
};
use std::{
    fs,
    io::{self, Cursor, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-chd-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn compressed(path: &Path, bytes: Vec<u8>) {
    let size = bytes.len() as u64;
    create_from_reader(
        Cursor::new(bytes),
        path,
        HdCreateOptions {
            logical_size: size,
            codecs: [CHD_CODEC_ZLIB, 0, 0, 0],
            ..Default::default()
        },
        &mut |_| {},
        &|| false,
    )
    .unwrap();
}
fn contents(path: &Path, size: usize) -> Vec<u8> {
    let mut disk = Media::open(path, MediaKind::Disk, false).unwrap();
    let mut bytes = vec![0; size];
    disk.read_exact_at(0, &mut bytes).unwrap();
    bytes
}

#[test]
fn blank_chd_has_exact_capacity_zeroes_and_durable_in_place_writes() {
    let directory = Directory::new();
    for size in [512, 1024 * 1024, 2 * 1024 * 1024 * 1024] {
        let path = directory.path(&format!("blank-{size}.chd"));
        assert_eq!(
            create_disk(&path, ImageFormat::Chd, size)
                .unwrap()
                .logical_bytes,
            size
        );
        assert!(fs::metadata(&path).unwrap().len() < 3 * 1024 * 1024);
        let mut image = Media::open(&path, MediaKind::Disk, true).unwrap();
        assert_eq!(image.info().format, ImageFormat::Chd);
        let mut sector = [1; 512];
        image.read_exact_at(size - 512, &mut sector).unwrap();
        assert_eq!(sector, [0; 512]);
        image.write_all_at(size - 512, &[0x6e; 512]).unwrap();
        image.sync().unwrap();
        drop(image);
        let mut reopened = Media::open(&path, MediaKind::Disk, false).unwrap();
        reopened.read_exact_at(size - 512, &mut sector).unwrap();
        assert_eq!(sector, [0x6e; 512]);
        assert!(!companion_path(&path).exists());
        let native = Chd::open(path.to_str().unwrap(), false, None).unwrap();
        assert_eq!(native.logical_bytes(), size);
        assert_eq!(native.sha1(), [0; 20]);
        assert_eq!(
            libchdman_rs::hd::read_geometry(&native)
                .unwrap()
                .sector_bytes,
            512
        );
    }
}

#[test]
fn compressed_disk_writes_reopen_without_modifying_parent_or_losing_zero_hunks() {
    let directory = Directory::new();
    let path = directory.path("compressed.renamed");
    let mut expected: Vec<_> = (0..16384).map(|i| (i * 37 + i / 512 + 1) as u8).collect();
    compressed(&path, expected.clone());
    let base = fs::read(&path).unwrap();
    let mut image = Media::open(&path, MediaKind::Disk, true).unwrap();
    // Cross both a sector and a hunk boundary, preserving both neighbours.
    image.write_all_at(4091, &[0x3a; 1030]).unwrap();
    expected[4091..5121].fill(0x3a);
    // Entire inherited, nonzero hunk becomes zero (native-library regression).
    image.write_all_at(8192, &[0; 4096]).unwrap();
    expected[8192..12288].fill(0);
    image.sync().unwrap();
    drop(image);
    assert_eq!(contents(&path, expected.len()), expected);
    assert_eq!(fs::read(&path).unwrap(), base);
    assert!(companion_path(&path).is_file());
    // Independent native reader follows the persisted child map.
    let parent = Chd::open(path.to_str().unwrap(), false, None).unwrap();
    let child = Chd::open(
        companion_path(&path).to_str().unwrap(),
        false,
        Some(&parent),
    )
    .unwrap();
    let mut bytes = vec![0; expected.len()];
    child.read_bytes(0, &mut bytes).unwrap();
    assert_eq!(bytes, expected);
}

#[test]
fn partial_write_can_zero_all_remaining_nonzero_parent_bytes() {
    let directory = Directory::new();
    let path = directory.path("partial.chd");
    let mut original = vec![0; 8192];
    original[512..1024].fill(0xa5);
    compressed(&path, original);
    let mut image = Media::open(&path, MediaKind::Disk, true).unwrap();
    image.write_all_at(512, &[0; 512]).unwrap();
    image.sync().unwrap();
    drop(image);
    assert_eq!(contents(&path, 8192), vec![0; 8192]);
}

#[test]
fn read_only_inspection_creates_nothing_and_sees_existing_child() {
    let directory = Directory::new();
    let path = directory.path("disk.chd");
    compressed(&path, vec![0x25; 8192]);
    let base = fs::read(&path).unwrap();
    assert_eq!(inspect(&path, MediaKind::Disk).unwrap().logical_bytes, 8192);
    assert!(!companion_path(&path).exists());
    let mut ro = Media::open(&path, MediaKind::Disk, false).unwrap();
    assert_eq!(
        ro.write_all_at(0, &[0; 512]).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    ro.sync().unwrap();
    drop(ro);
    let mut rw = Media::open(&path, MediaKind::Disk, true).unwrap();
    rw.write_all_at(0, &[0x17; 512]).unwrap();
    rw.sync().unwrap();
    drop(rw);
    let diff = fs::read(companion_path(&path)).unwrap();
    assert_eq!(&contents(&path, 8192)[..512], &[0x17; 512]);
    inspect(&path, MediaKind::Disk).unwrap();
    assert_eq!(fs::read(companion_path(&path)).unwrap(), diff);
    assert_eq!(fs::read(&path).unwrap(), base);
}

#[test]
fn mismatched_or_truncated_companions_are_never_ignored_or_overwritten() {
    let directory = Directory::new();
    let a = directory.path("a.chd");
    let b = directory.path("b.chd");
    compressed(&a, vec![0x12; 8192]);
    compressed(&b, vec![0x34; 8192]);
    drop(Media::open(&a, MediaKind::Disk, true).unwrap());
    let child = fs::read(companion_path(&a)).unwrap();
    for bad in [child, b"MComprHD".to_vec()] {
        fs::write(companion_path(&b), &bad).unwrap();
        for writable in [false, true] {
            assert!(Media::open(&b, MediaKind::Disk, writable).is_err());
        }
        assert_eq!(fs::read(companion_path(&b)).unwrap(), bad);
    }
    assert!(Media::open(companion_path(&a), MediaKind::Disk, true).is_err());
}

#[test]
fn malformed_headers_and_metadata_fail_before_companion_creation() {
    let directory = Directory::new();
    let path = directory.path("bad.chd");
    create_disk(&path, ImageFormat::Chd, 8192).unwrap();
    let valid = fs::read(&path).unwrap();
    let metadata = u64::from_be_bytes(valid[48..56].try_into().unwrap()) as usize;
    let edits = [
        (12, 4_u32.to_be_bytes().to_vec()),    // unsupported version
        (56, 0_u32.to_be_bytes().to_vec()),    // zero hunk
        (60, 2048_u32.to_be_bytes().to_vec()), // CD/DVD unit
        (32, u64::MAX.to_be_bytes().to_vec()), // overflow capacity
        (40, u64::MAX.to_be_bytes().to_vec()), // invalid map
        (48, u64::MAX.to_be_bytes().to_vec()), // invalid metadata head
        (metadata + 8, (metadata as u64).to_be_bytes().to_vec()), // cycle
        (metadata + 5, vec![0xff; 3]),         // truncated/oversized value
        (metadata, b"CHT2".to_vec()),          // CD metadata
    ];
    for (offset, replacement) in edits {
        let mut bad = valid.clone();
        bad[offset..offset + replacement.len()].copy_from_slice(&replacement);
        fs::write(&path, &bad).unwrap();
        assert!(
            Media::open(&path, MediaKind::Disk, true).is_err(),
            "offset {offset}"
        );
        assert_eq!(fs::read(&path).unwrap(), bad);
        assert!(!companion_path(&path).exists());
    }
}

#[test]
fn disk_bounds_and_external_truncation_do_not_grow_container() {
    let directory = Directory::new();
    let path = directory.path("disk.chd");
    create_disk(&path, ImageFormat::Chd, 8192).unwrap();
    let mut image = Media::open(&path, MediaKind::Disk, true).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(image.write_all_at(8191, &[1, 2]).is_err());
    assert!(image.read_exact_at(u64::MAX, &mut [0; 1]).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(100)
        .unwrap();
    assert!(image.write_all_at(0, &[1; 512]).is_err());
    assert!(image.read_exact_at(0, &mut [0; 512]).is_err());
    assert!(image.sync().is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), 100);
}

#[test]
fn hashless_parent_and_wrong_media_type_are_rejected_without_changes() {
    let directory = Directory::new();
    let path = directory.path("disk.chd");
    compressed(&path, vec![0x91; 8192]);
    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(84)).unwrap();
    file.write_all(&[0; 20]).unwrap();
    drop(file);
    assert!(Media::open(&path, MediaKind::Disk, true).is_err());
    assert!(!companion_path(&path).exists());
    assert!(Media::open(&path, MediaKind::Cdrom, false).is_err());
}

#[test]
fn editing_an_uncompressed_image_invalidates_its_old_content_hashes() {
    let directory = Directory::new();
    let path = directory.path("uncompressed.chd");
    create_from_reader(
        Cursor::new(vec![0xa5; 8192]),
        &path,
        HdCreateOptions {
            logical_size: 8192,
            codecs: [0; 4],
            ..Default::default()
        },
        &mut |_| {},
        &|| false,
    )
    .unwrap();
    // The native uncompressed creator leaves hashes empty. Supply the valid
    // hashes computed for identical data/metadata by a compressed reference,
    // representing an externally checksummed mutable v5 image.
    let reference = directory.path("reference.chd");
    compressed(&reference, vec![0xa5; 8192]);
    let hashes = fs::read(&reference).unwrap()[64..104].to_vec();
    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(64)).unwrap();
    file.write_all(&hashes).unwrap();
    drop(file);
    let before = Chd::open(path.to_str().unwrap(), false, None).unwrap();
    assert_ne!(before.sha1(), [0; 20]);
    assert_ne!(before.raw_sha1(), [0; 20]);
    drop(before);
    let header_before = fs::read(&path).unwrap()[..124].to_vec();
    let mut image = Media::open(&path, MediaKind::Disk, true).unwrap();
    // Merely opening does not rewrite metadata.
    assert_eq!(&fs::read(&path).unwrap()[..124], header_before);
    image.write_all_at(512, &[0x24; 512]).unwrap();
    image.sync().unwrap();
    drop(image);
    let native = Chd::open(path.to_str().unwrap(), false, None).unwrap();
    assert_eq!(native.sha1(), [0; 20]);
    assert_eq!(native.raw_sha1(), [0; 20]);
    assert_eq!(native.parent_sha1(), [0; 20]);
    let mut expected = vec![0xa5; 8192];
    expected[512..1024].fill(0x24);
    assert_eq!(contents(&path, 8192), expected);
    assert!(!companion_path(&path).exists());
}

#[test]
fn synchronized_writes_survive_process_exit_without_dropping_native_handles() {
    let directory = Directory::new();
    let path = directory.path("disk.chd");
    compressed(&path, vec![0xa5; 8192]);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "synchronized_child_writer", "--nocapture"])
        .env("HELIOS_CHD_TEST_CHILD", &path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut expected = vec![0xa5; 8192];
    expected[..4096].fill(0);
    expected[4608..5120].fill(0x73);
    assert_eq!(contents(&path, 8192), expected);
}

#[test]
fn synchronized_child_writer() {
    if let Some(path) = std::env::var_os("HELIOS_CHD_TEST_CHILD") {
        let mut image = Media::open(path, MediaKind::Disk, true).unwrap();
        image.write_all_at(0, &[0; 4096]).unwrap();
        image.write_all_at(4608, &[0x73; 512]).unwrap();
        image.sync().unwrap();
        // Deliberately skip destructors: durability must not depend on Drop.
        std::process::exit(0);
    }
}

#[test]
fn creation_and_existing_child_collisions_preserve_all_paths() {
    let directory = Directory::new();
    let path = directory.path("disk.chd");
    for size in [0, 513, (1_u64 << 41) + 512] {
        assert!(create_disk(&path, ImageFormat::Chd, size).is_err());
        assert!(!path.exists());
    }
    fs::write(companion_path(&path), b"valuable child").unwrap();
    assert!(create_disk(&path, ImageFormat::Chd, 8192).is_err());
    assert!(!path.exists());
    assert_eq!(fs::read(companion_path(&path)).unwrap(), b"valuable child");
    fs::remove_file(companion_path(&path)).unwrap();
    create_disk(&path, ImageFormat::Chd, 8192).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        create_disk(&path, ImageFormat::Chd, 8192)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn dangling_symlinks_are_collisions_not_permission_to_create_or_replace() {
    use std::os::unix::fs::symlink;
    let directory = Directory::new();
    let path = directory.path("disk.chd");
    let missing = directory.path("missing");
    symlink(&missing, &path).unwrap();
    assert!(create_disk(&path, ImageFormat::Chd, 8192).is_err());
    assert_eq!(fs::read_link(&path).unwrap(), missing);
    fs::remove_file(&path).unwrap();
    compressed(&path, vec![0x31; 8192]);
    symlink(&missing, companion_path(&path)).unwrap();
    assert!(Media::open(&path, MediaKind::Disk, true).is_err());
    assert!(!missing.exists());
}

#[test]
fn repeated_fragment_and_block_writes_match_a_byte_model_after_reopening() {
    let directory = Directory::new();
    let path = directory.path("fragmented.chd");
    const SIZE: usize = 2 * 1024 * 1024;
    create_disk(&path, ImageFormat::Chd, SIZE as u64).unwrap();
    let mut disk = Media::open(&path, MediaKind::Disk, true).unwrap();
    let mut expected = vec![0; SIZE];
    let mut state = 0x94db_60e2_u32;
    for iteration in 0..512 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        // FFS-sized fragments/blocks plus sector updates, including writes
        // crossing native hunks, overwrites, zeroes and a final-sector update.
        let length = [512, 1024, 2048, 4096, 16384, 32768][iteration % 6];
        let offset = if iteration % 17 == 0 {
            SIZE - length
        } else {
            (state as usize % ((SIZE - length) / 512 + 1)) * 512
        };
        let bytes: Vec<_> = (0..length)
            .map(|i| {
                if iteration % 7 == 0 {
                    0
                } else {
                    (i ^ iteration ^ (state as usize >> 16)) as u8
                }
            })
            .collect();
        disk.write_all_at(offset as u64, &bytes).unwrap();
        disk.sync().unwrap();
        expected[offset..offset + length].copy_from_slice(&bytes);
        if iteration % 32 == 31 {
            drop(disk);
            assert_eq!(contents(&path, SIZE), expected, "iteration {iteration}");
            disk = Media::open(&path, MediaKind::Disk, true).unwrap();
        }
    }
    drop(disk);
    // Independently decode the uncompressed v5 map. Comparing only native
    // reads with native writes would miss a shared cache/addressing defect.
    let file = fs::read(&path).unwrap();
    let hunk = u32::from_be_bytes(file[56..60].try_into().unwrap()) as usize;
    let map = u64::from_be_bytes(file[40..48].try_into().unwrap()) as usize;
    assert_eq!(&file[16..32], &[0; 16]);
    assert_eq!(&file[104..124], &[0; 20]);
    for (index, bytes) in expected.chunks(hunk).enumerate() {
        let entry = map + index * 4;
        let block = u32::from_be_bytes(file[entry..entry + 4].try_into().unwrap()) as usize;
        if block == 0 {
            assert!(
                bytes.iter().all(|byte| *byte == 0),
                "unallocated hunk {index}"
            );
        } else {
            assert_eq!(
                &file[block * hunk..block * hunk + bytes.len()],
                bytes,
                "hunk {index}"
            );
        }
    }
}
