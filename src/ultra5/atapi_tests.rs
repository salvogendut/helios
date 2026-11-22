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
    assert_eq!(b.read_bypass_byte(TASK + 7).unwrap(), 0xff);
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
