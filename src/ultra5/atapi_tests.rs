//! Drive the CMD646 through its firmware-programmed Simba I/O window.
use crate::ultra5::{BootFlash, ResetNvram, Ultra5ResetBoard, FLASH_SIZE, RESET_NVRAM_SIZE};
use crate::v9::V9DataBus;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

const PCI: u64 = 0x01fe_0100_0000;
const IDE: u64 = PCI + 0x11800;
const TASK: u64 = 0x01fe_02c0_0010;
const CONTROL: u64 = TASK + 10;

struct Disc {
    path: PathBuf,
    image: Vec<u8>,
}
impl Disc {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-atapi-{}-{}.iso",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let image: Vec<u8> = (0..4096u32)
            .map(|i| (i ^ (i >> 7) ^ (i >> 9)) as u8)
            .collect();
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap()
            .write_all(&image)
            .unwrap();
        Self { path, image }
    }
    fn board(&self) -> Ultra5ResetBoard {
        let mut b = Ultra5ResetBoard::new(
            BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
            ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
        );
        b.attach_cdrom(&self.path).unwrap();
        for (a, v) in [
            (PCI + 0x919, 1),
            (PCI + 0x91a, 1),
            (PCI + 0x904, 3),
            (PCI + 0x9de, 0x40),
            (IDE + 4, 1),
            (IDE + 9, 0x8f),
        ] {
            b.write_bypass_byte(a, v).unwrap();
        }
        for (i, v) in [0xc00001, 0xc00009, 0xc00011, 0xc00019]
            .into_iter()
            .enumerate()
        {
            b.write_bypass_le_word(IDE + 0x10 + 4 * i as u64, v)
                .unwrap();
        }
        b
    }
}
impl Drop for Disc {
    fn drop(&mut self) {
        fs::remove_file(&self.path).unwrap();
    }
}

fn packet(b: &mut Ultra5ResetBoard, cdb: [u8; 12], limit: u16) {
    b.write_bypass_byte(TASK + 4, limit as u8).unwrap();
    b.write_bypass_byte(TASK + 5, (limit >> 8) as u8).unwrap();
    b.write_bypass_byte(TASK + 7, 0xa0).unwrap();
    assert_eq!(b.read_bypass_byte(TASK + 2).unwrap(), 1);
    for pair in cdb.chunks_exact(2) {
        b.write_bypass_half(TASK, u16::from_be_bytes([pair[0], pair[1]]))
            .unwrap();
    }
}
fn data(b: &mut Ultra5ResetBoard, n: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..n.div_ceil(2) {
        bytes.extend(b.read_bypass_half(TASK).unwrap().to_be_bytes());
    }
    bytes.truncate(n);
    bytes
}

#[test]
fn probe_select_reset_and_identify_packet_device() {
    let disc = Disc::new();
    let mut b = disc.board();
    assert!(b.attach_cdrom(&disc.path).is_err());
    assert_eq!(b.read_bypass_byte(TASK - 16 + 7).unwrap(), 0xff);
    b.write_bypass_byte(TASK + 6, 0xb0).unwrap();
    assert_eq!(b.read_bypass_byte(TASK + 7).unwrap(), 0); // absent slave on a populated cable
    b.write_bypass_byte(CONTROL, 4).unwrap();
    b.write_bypass_byte(CONTROL, 0).unwrap();
    assert_eq!(b.read_bypass_byte(TASK + 4).unwrap(), 0x14);
    assert_eq!(b.read_bypass_byte(TASK + 5).unwrap(), 0xeb);
    b.write_bypass_byte(TASK + 7, 0xec).unwrap();
    assert_eq!(b.read_bypass_byte(TASK + 1).unwrap(), 4);
    b.write_bypass_byte(TASK + 7, 0xa1).unwrap();
    let id = data(&mut b, 512);
    assert_eq!(&id[..2], &[0xc0, 0x85]);
    assert_eq!(&id[54..60], b"EHILSO");
    assert_eq!(b.read_bypass_byte(CONTROL).unwrap() & 9, 0);
    b.write_bypass_byte(IDE + 4, 0).unwrap();
    assert!(b.read_bypass_byte(TASK + 7).is_err());
}

#[test]
fn packet_reads_native_sectors_in_bounded_phases_and_preserves_media_on_errors() {
    let disc = Disc::new();
    let mut b = disc.board();
    let mut cmd = [0u8; 12];
    cmd[0] = 0x28;
    cmd[5] = 1;
    cmd[8] = 1;
    packet(&mut b, cmd, 510);
    let mut bytes = Vec::new();
    for expected_len in [510, 510, 510, 510, 8] {
        assert_eq!(b.read_bypass_byte(TASK + 2).unwrap(), 2);
        assert_eq!(
            usize::from(b.read_bypass_byte(TASK + 4).unwrap())
                + 256 * usize::from(b.read_bypass_byte(TASK + 5).unwrap()),
            expected_len
        );
        bytes.extend(data(&mut b, expected_len));
    }
    assert_eq!(bytes, disc.image[2048..]);
    assert_eq!(b.read_bypass_byte(TASK + 2).unwrap(), 3);
    for (opcode, lba, key) in [(0x28, 2, 5), (0x2a, 0, 7)] {
        cmd[0] = opcode;
        cmd[5] = lba;
        packet(&mut b, cmd, 2048);
        assert_eq!(b.read_bypass_byte(TASK + 7).unwrap() & 9, 1);
        assert_eq!(b.read_bypass_byte(TASK + 1).unwrap() >> 4, key);
        packet(&mut b, [3, 0, 0, 0, 17, 0, 0, 0, 0, 0, 0, 0], 32);
        assert_eq!(data(&mut b, 17)[2], key);
    }
    assert_eq!(fs::read(&disc.path).unwrap(), disc.image);
}

#[test]
fn read12_checks_the_complete_range_and_survives_reset_mid_transfer() {
    let disc = Disc::new();
    let mut b = disc.board();
    let mut read = [0; 12];
    read[0] = 0xa8;
    read[9] = 2;
    packet(&mut b, read, 2048);
    assert_eq!(data(&mut b, 4096), disc.image);
    read[5] = 1;
    packet(&mut b, read, 2048);
    assert_eq!(b.read_bypass_byte(TASK + 7).unwrap() & 9, 1);
    assert_eq!(b.read_bypass_byte(TASK + 1).unwrap(), 0x50);
    assert!(b.read_bypass_half(TASK).is_err());
    read[5] = 0;
    packet(&mut b, read, 2048);
    assert_eq!(data(&mut b, 2), disc.image[..2]);
    b.write_bypass_byte(CONTROL, 4).unwrap();
    b.write_bypass_byte(CONTROL, 0).unwrap();
    assert!(b.read_bypass_half(TASK).is_err());
    read[9] = 0;
    packet(&mut b, read, 2048);
    assert_eq!(b.read_bypass_byte(TASK + 7).unwrap() & 9, 0);
    assert_eq!(b.read_bypass_byte(TASK + 2).unwrap(), 3);
}

#[test]
fn primary_disk_pio_persists_whole_sectors_and_rejects_an_out_of_range_batch() {
    let optical = Disc::new();
    let dir = optical.path.with_extension("ide0.raw");
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&dir)
        .unwrap()
        .set_len(4096)
        .unwrap();
    let mut disk = super::ata::Disk::open(&dir, true).unwrap();
    let address = |disk: &mut super::ata::Disk, sector, count| {
        disk.write(6, 0xe0).unwrap();
        disk.write(2, count).unwrap();
        disk.write(3, sector).unwrap();
        disk.write(4, 0).unwrap();
        disk.write(5, 0).unwrap();
    };
    address(&mut disk, 3, 2);
    disk.write(7, 0x30).unwrap();
    let bytes: Vec<u8> = (0..1024u32).map(|i| (i ^ (i >> 8)) as u8).collect();
    for word in bytes.chunks_exact(2) {
        disk.write_data([word[0], word[1]]).unwrap();
    }
    assert_eq!((disk.read(2).unwrap(), disk.read(3).unwrap()), (0, 5));
    disk.write(7, 0xe7).unwrap();
    assert_eq!(disk.read(7).unwrap() & 9, 0);
    drop(disk);
    let saved = fs::read(&dir).unwrap();
    assert_eq!(&saved[3 * 512..5 * 512], &bytes);
    assert!(saved[..3 * 512]
        .iter()
        .chain(&saved[5 * 512..])
        .all(|&b| b == 0));
    let mut disk = super::ata::Disk::open(&dir, false).unwrap();
    address(&mut disk, 3, 2);
    disk.write(7, 0x20).unwrap();
    for word in bytes.chunks_exact(2) {
        assert_eq!(&disk.read_data().unwrap(), word);
    }
    address(&mut disk, 7, 2);
    disk.write(7, 0x20).unwrap();
    assert_eq!(
        (disk.read(7).unwrap() & 9, disk.read(1).unwrap()),
        (1, 0x10)
    );
    address(&mut disk, 0, 1);
    disk.write(7, 0x30).unwrap();
    assert_eq!((disk.read(7).unwrap() & 9, disk.read(1).unwrap()), (1, 4));
    drop(disk);
    fs::remove_file(dir).unwrap();
}

#[test]
fn cmd646_dma_translates_each_fragment_and_checks_the_last_page_before_writing() {
    let optical = Disc::new();
    let path = optical.path.with_extension("ide0.raw");
    let payload: Vec<u8> = (0..512u16).map(|i| (i ^ (i >> 3)) as u8).collect();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(&payload).unwrap();
    file.set_len(1024).unwrap();
    drop(file);
    let mut board = optical.board();
    board.attach_disk(&path, true).unwrap();
    board.write_bypass_byte(IDE + 4, 5).unwrap();
    let task = TASK - 16;
    let pbm = 0x01fe_0000_0000;
    board.write_bypass_xword(pbm + 0x2028, 0x40).unwrap();
    let table = 0x1001_0000;
    board.write_bypass_xword(pbm + 0x208, table).unwrap();
    board.write_bypass_xword(pbm + 0x200, 1).unwrap();
    let valid = 1 << 63;
    let prd = 0x1002_0000;
    for (index, pte) in [
        (0, valid | prd),
        (1, valid | 0x1003_2002),
        (2, valid | 0x1005_0000),
    ] {
        board.write_cacheable_xword(table + index * 8, pte).unwrap();
    }
    // Little-endian PRD points eight bytes before an I/O page boundary.
    board
        .write_cacheable_xword(prd, 0xf83f_00c0_0002_0080)
        .unwrap();
    board.write_cacheable_xword(0x1003_3ff8, u64::MAX).unwrap();
    board.write_bypass_le_word(IDE + 0x74, 0xc000_0000).unwrap();
    let command = |b: &mut Ultra5ResetBoard, sector, op, master| {
        for (off, value) in [(6, 0xe0), (2, 1), (3, sector), (4, 0), (5, 0), (7, op)] {
            b.write_bypass_byte(task + off, value).unwrap();
        }
        b.write_bypass_byte(IDE + 0x70, master).unwrap();
        let status = b.read_bypass_byte(IDE + 0x72).unwrap();
        b.write_bypass_byte(IDE + 0x70, 0).unwrap();
        b.write_bypass_byte(IDE + 0x72, 6).unwrap();
        status & 7
    };
    assert_eq!(command(&mut board, 0, 0xc9, 9), 6);
    assert_eq!(board.read_cacheable_xword(0x1003_3ff8).unwrap(), u64::MAX);
    board
        .write_cacheable_xword(table + 16, valid | 0x1005_0002)
        .unwrap();
    board.write_bypass_xword(pbm + 0x210, 0xc000_4000).unwrap();
    assert_eq!(command(&mut board, 0, 0xc9, 9), 4);
    assert_eq!(
        board
            .read_cacheable_xword(0x1003_3ff8)
            .unwrap()
            .to_be_bytes(),
        payload[..8]
    );
    assert_eq!(
        board
            .read_cacheable_xword(0x1005_0000)
            .unwrap()
            .to_be_bytes(),
        payload[8..16]
    );
    assert_eq!(command(&mut board, 1, 0xcb, 1), 4);
    drop(board);
    assert_eq!(&fs::read(&path).unwrap()[512..], payload);
    fs::remove_file(path).unwrap();
}
