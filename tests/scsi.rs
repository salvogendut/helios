use helios::bus::Bus;
use helios::devices::scsi::{MediaKind, ScsiBus};
use helios::machine::Machine;
use helios::system_bus::{ESP_BASE as ESP, IO_CONTROL_BASE as IO, SCSI_DMA_BASE as DMA};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Image(PathBuf);
impl Image {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-scsi-{}-{}.img",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
        Self(path)
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

fn fixture() -> Machine {
    let mut m = Machine::sun4m(0x20000, &[1, 0, 0, 0]).unwrap();
    m.bus.write8(ESP + 12, 0).unwrap();
    m.bus.write8(ESP + 32, 7).unwrap();
    m.bus.write8(ESP + 20, 1).unwrap();
    m.bus.write32(IO, 1).unwrap();
    m.bus.write32(IO + 4, 0x1000).unwrap();
    m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
    m.bus.write32(0x10000, 0x806).unwrap(); // ff000000 -> 8000, writable.
    m.bus.write32(0x10004, 0x902).unwrap(); // ff001000 -> 9000, read-only.
    m
}
fn attach(m: &mut Machine, kind: MediaKind, bytes: &[u8]) -> Image {
    let image = Image::new(bytes);
    let id = if kind == MediaKind::Disk { 3 } else { 6 };
    m.bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach(id, kind, &image.0)
        .unwrap();
    image
}
fn attach_rw(m: &mut Machine, bytes: &[u8]) -> Image {
    let image = Image::new(bytes);
    m.bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach_disk_rw(3, &image.0)
        .unwrap();
    image
}
fn irq(m: &mut Machine) -> (u8, u8, u8) {
    for _ in 0..1000 {
        m.service_scsi().unwrap();
        m.advance_ns(100);
        if m.bus.devices().unwrap().esp.irq() {
            let status = m.bus.read8(ESP + 16).unwrap();
            let sequence = m.bus.read8(ESP + 24).unwrap();
            let interrupt = m.bus.read8(ESP + 20).unwrap();
            return (status, sequence, interrupt);
        }
    }
    panic!("ESP did not interrupt: {:?}", m.bus.devices().unwrap().esp);
}
fn select(m: &mut Machine, id: u8, lun: u8, cdb: &[u8]) -> (u8, u8, u8) {
    m.bus.write8(ESP + 16, id).unwrap();
    m.bus.write8(ESP + 8, 0x80 | lun).unwrap();
    for &b in cdb {
        m.bus.write8(ESP + 8, b).unwrap();
    }
    m.bus.write8(ESP + 12, 0x42).unwrap();
    irq(m)
}
fn data(m: &mut Machine) -> Vec<u8> {
    let mut bytes = Vec::new();
    while m.bus.read8(ESP + 16).unwrap() & 7 == 1 {
        m.bus.write8(ESP + 12, 0x10).unwrap();
        let (_, _, interrupt) = irq(m);
        assert_eq!(interrupt, 0x10);
        assert_eq!(m.bus.read8(ESP + 28).unwrap(), 1);
        bytes.push(m.bus.read8(ESP + 8).unwrap());
    }
    bytes
}
fn complete(m: &mut Machine) -> u8 {
    assert_eq!(m.bus.read8(ESP + 16).unwrap() & 7, 3);
    m.bus.write8(ESP + 12, 0x11).unwrap();
    let (status, _, interrupt) = irq(m);
    assert_eq!((status & 7, interrupt), (7, 8));
    assert_eq!(m.bus.read8(ESP + 28).unwrap(), 2);
    let status = m.bus.read8(ESP + 8).unwrap();
    assert_eq!(m.bus.read8(ESP + 8).unwrap(), 0);
    m.bus.write8(ESP + 12, 0x12).unwrap();
    assert_eq!(irq(m).2, 0x20);
    status
}
fn dma(m: &mut Machine, address: u32, count: u16, write: bool) {
    m.bus
        .write32(DMA, 0x2220 | if write { 0x100 } else { 0 })
        .unwrap();
    m.bus.write32(DMA + 8, u32::from(count)).unwrap();
    m.bus.write32(DMA + 4, address).unwrap();
    m.bus.write8(ESP, count as u8).unwrap();
    m.bus.write8(ESP + 4, (count >> 8) as u8).unwrap();
}

#[test]
fn disk_and_cdrom_inquiry_have_distinct_type_capacity_and_removable_bits() {
    let mut m = fixture();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 4096]);
    let _cd = attach(&mut m, MediaKind::Cdrom, &[0; 8192]);
    for (id, peripheral, removable, product, capacity) in [
        (3, 0, 0, b"SCSI DISK       ", [0, 0, 0, 7, 0, 0, 2, 0]),
        (6, 5, 0x80, b"SCSI CD-ROM     ", [0, 0, 0, 3, 0, 0, 8, 0]),
    ] {
        assert_eq!(
            select(&mut m, id, 0, &[0x12, 0, 0, 0, 96, 0]),
            (0x81, 4, 0x18)
        );
        let response = data(&mut m);
        assert_eq!(&response[..8], &[peripheral, removable, 2, 2, 31, 0, 0, 0]);
        assert_eq!(&response[8..16], b"HELIOS  ");
        assert_eq!(&response[16..32], product);
        assert_eq!(&response[32..], b"0001");
        assert_eq!(complete(&mut m), 0);
        select(&mut m, id, 0, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(data(&mut m), capacity);
        assert_eq!(complete(&mut m), 0);
    }
}

#[test]
fn absent_targets_time_out_without_consuming_fifo_and_bus_can_select_again() {
    let mut m = fixture();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 512]);
    for id in [0, 1, 2, 4, 5, 6, 7] {
        m.bus.write8(ESP + 16, id).unwrap();
        m.bus.write8(ESP + 8, 0x80).unwrap();
        m.bus.write8(ESP + 12, 0x42).unwrap();
        m.service_scsi().unwrap();
        m.advance_ns(409_599); // timeout * 8192 * CCF(2) / 40 MHz.
        assert!(!m.bus.devices().unwrap().esp.irq());
        m.advance_ns(1);
        assert_eq!(irq(&mut m), (0x80, 0, 0x20));
        assert_eq!(m.bus.read8(ESP + 28).unwrap(), 1);
        assert_eq!(m.bus.read8(ESP + 8).unwrap(), 0x80);
    }
    select(&mut m, 3, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
}

#[test]
fn dma_cdb_and_split_inquiry_cross_iommu_with_exact_residuals_and_status() {
    let mut m = fixture();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 512]);
    m.load_image(0x8001, &[0x80, 0x12, 0, 0, 0, 96, 0]).unwrap();
    dma(&mut m, 0xff00_0001, 7, false);
    m.bus.write8(ESP + 16, 3).unwrap();
    m.bus.write8(ESP + 12, 0xc2).unwrap();
    assert_eq!(irq(&mut m), (0x91, 4, 0x18));
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_0008);
    assert_eq!(m.bus.read32(DMA + 8).unwrap(), 0);
    assert_eq!(m.bus.read32(DMA).unwrap() & 0x4001, 0x4001);
    dma(&mut m, 0xff00_0101, 9, true);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m), (0x91, 4, 0x10));
    assert_eq!(m.bus.read8(ESP).unwrap(), 0);
    dma(&mut m, 0xff00_010a, 87, true);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m), (0x83, 4, 0x10)); // Target short reply, TC remains clear.
    assert_eq!(m.bus.read8(ESP).unwrap(), 60);
    assert_eq!(m.bus.read32(DMA + 8).unwrap(), 60);
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_0125);
    let received: Vec<_> = (0..36).map(|i| m.bus.read8(0x8101 + i).unwrap()).collect();
    assert_eq!(&received[8..], b"HELIOS  SCSI DISK       0001");
    assert_eq!(m.bus.read8(0x8100).unwrap(), 0);
    assert_eq!(m.bus.read8(0x8125).unwrap(), 0);
    assert_eq!(complete(&mut m), 0);
}

#[test]
fn arbitration_denial_and_iommu_write_fault_retain_the_untransferred_byte() {
    let mut m = fixture();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 512]);
    select(&mut m, 3, 0, &[0x12, 0, 0, 0, 36, 0]);
    dma(&mut m, 0xff00_1000, 36, true);
    m.bus.write32(IO + 0x2000, 0).unwrap();
    m.bus.write8(ESP + 12, 0x90).unwrap();
    let esp = m.bus.devices().unwrap().esp.clone();
    m.service_scsi().unwrap();
    assert_eq!(m.bus.devices().unwrap().esp, esp);
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_1000);
    m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
    m.service_scsi().unwrap();
    assert_eq!(m.bus.devices().unwrap().esp, esp);
    assert_eq!(m.bus.read32(DMA).unwrap() & 2, 2);
    assert_eq!(m.bus.read32(0x9000).unwrap(), 0);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    // Guest invalidates the error, fixes the DMA address and retries.
    m.bus.write32(DMA, 0x2320).unwrap();
    m.bus.write32(DMA + 4, 0xff00_0000).unwrap();
    assert_eq!(irq(&mut m).2, 0x10);
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0x0000_0202);
    assert_eq!(complete(&mut m), 0);
}

#[test]
fn raw_block_reads_write_protection_and_request_sense_preserve_image() {
    let mut m = fixture();
    let bytes: Vec<_> = (0..1536).map(|i| (i / 512 * 73 + i % 251) as u8).collect();
    let disk = attach(&mut m, MediaKind::Disk, &bytes);
    select(&mut m, 3, 0, &[8, 0, 0, 1, 1, 0]);
    assert_eq!(data(&mut m), bytes[512..1024]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 0, &[0x28, 0, 0, 0, 0, 2, 0, 0, 1, 0]);
    assert_eq!(data(&mut m), bytes[1024..]);
    assert_eq!(complete(&mut m), 0);
    for (cdb, key, asc) in [
        ([8, 0, 0, 3, 1, 0], 5, 0x21),
        ([0x0a, 0, 0, 0, 1, 0], 7, 0x27),
        ([0x1f, 0, 0, 0, 0, 0], 5, 0x20),
    ] {
        select(&mut m, 3, 0, &cdb);
        assert_eq!(complete(&mut m), 2);
        select(&mut m, 3, 0, &[3, 0, 0, 0, 18, 0]);
        let response = data(&mut m);
        assert_eq!(
            (
                response[0],
                response[2],
                response[7],
                response[12],
                response[13]
            ),
            (0x70, key, 10, asc, 0)
        );
        assert_eq!(complete(&mut m), 0);
    }
    assert_eq!(std::fs::read(disk.0.as_path()).unwrap(), bytes);
}

#[test]
fn unsupported_lun_and_inquiry_allocation_do_not_invent_devices() {
    let mut m = fixture();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 512]);
    select(&mut m, 3, 1, &[0x12, 0, 0, 0, 5, 0]);
    assert_eq!(data(&mut m), [0x7f, 0, 2, 2, 31]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 1, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 3, 0, &[0x12, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
}

#[test]
fn attachment_validates_raw_media_and_target_ids_before_replacement() {
    let image = Image::new(&[0; 512]);
    let short = Image::new(&[0; 511]);
    let empty = Image::new(&[]);
    let mut bus = ScsiBus::default();
    for path in [&short.0, &empty.0] {
        assert!(bus.attach(3, MediaKind::Disk, path).is_err());
    }
    assert!(bus.attach(6, MediaKind::Cdrom, &image.0).is_err());
    assert!(bus.attach(7, MediaKind::Disk, &image.0).is_err());
    assert!(bus.attach(255, MediaKind::Disk, &image.0).is_err());
    bus.attach(3, MediaKind::Disk, &image.0).unwrap();
    assert!(bus.attach(3, MediaKind::Disk, &image.0).is_err());
    assert_eq!(bus.target(3).unwrap().blocks(), 1);
    assert_eq!(bus.target(3).unwrap().kind(), MediaKind::Disk);
}

#[test]
fn preloaded_identify_precedes_dma_cdb_and_bus_reset_reports_unit_attention() {
    let mut m = fixture();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 512]);
    m.load_image(0x8000, &[0x12, 0, 0, 0, 5, 0]).unwrap();
    m.bus.write8(ESP + 8, 0x80).unwrap();
    dma(&mut m, 0xff00_0000, 6, false);
    m.bus.write8(ESP + 16, 3).unwrap();
    m.bus.write8(ESP + 12, 0xc2).unwrap();
    assert_eq!(irq(&mut m), (0x91, 4, 0x18));
    assert_eq!(data(&mut m), [0, 0, 2, 2, 31]);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_0006);

    m.bus.write8(ESP + 12, 3).unwrap();
    assert_eq!(irq(&mut m).2, 0x80);
    m.advance_ns(6500);
    // INQUIRY must neither fail for, nor consume, the pending reset attention.
    select(&mut m, 3, 0, &[0x12, 0, 0, 0, 5, 0]);
    assert_eq!(data(&mut m), [0, 0, 2, 2, 31]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 3, 0, &[3, 0, 0, 0, 18, 0]);
    let sense = data(&mut m);
    assert_eq!((sense[2], sense[12]), (6, 0x29));
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(m.bus.devices().unwrap().scsi.target(3).unwrap().blocks(), 1);
}

#[test]
fn dma_chaining_reloads_address_and_count_and_stops_at_last_buffer() {
    let mut m = fixture();
    m.bus
        .write32(
            helios::system_bus::SYSTEM_INTERRUPT_BASE + 8,
            (1 << 31) | (1 << 18),
        )
        .unwrap();
    let _disk = attach(&mut m, MediaKind::Disk, &[0; 512]);
    select(&mut m, 3, 0, &[0x12, 0, 0, 0, 36, 0]);
    // The first buffer has 8 bytes; the next receives the 28 identity bytes.
    m.bus.write32(DMA, 0x0100_2330).unwrap();
    m.bus.write32(DMA + 8, 8).unwrap();
    m.bus.write32(DMA + 4, 0xff00_0000).unwrap();
    m.bus.write32(DMA + 8, 28).unwrap();
    m.bus.write32(DMA + 4, 0xff00_0100).unwrap();
    m.bus.write8(ESP, 36).unwrap();
    m.bus.write8(ESP + 12, 0x90).unwrap();
    for _ in 0..8 {
        m.service_scsi().unwrap();
    }
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_0100);
    assert_eq!(m.bus.read32(DMA + 8).unwrap(), 28);
    assert_eq!(m.bus.read32(DMA).unwrap() & 0x4001, 0x4001);
    m.advance_ns(0);
    assert_eq!(m.bus.interrupt_levels() & (1 << 4), 1 << 4);
    m.bus.write32(DMA, 0x0100_6310).unwrap(); // Chaining TC W1C.
    assert_eq!(m.bus.read32(DMA).unwrap() & 0x4001, 0);
    assert_eq!(irq(&mut m), (0x93, 4, 0x10));
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_011c);
    assert_eq!(m.bus.read32(DMA).unwrap() & 0x0e00_4001, 0x4001);
    let identity: Vec<_> = (0..28).map(|i| m.bus.read8(0x8100 + i).unwrap()).collect();
    assert_eq!(identity, b"HELIOS  SCSI DISK       0001");
    assert_eq!(complete(&mut m), 0);
}

#[test]
fn cdrom_reads_use_2048_byte_sectors_and_dma_zero_count_means_65536() {
    let mut m = fixture();
    let bytes: Vec<_> = (0..6144).map(|i| (i / 2048 * 89 + i % 251) as u8).collect();
    let cdrom = attach(&mut m, MediaKind::Cdrom, &bytes);
    select(&mut m, 6, 0, &[0x28, 0, 0, 0, 0, 1, 0, 0, 1, 0]);
    dma(&mut m, 0xff00_0000, 0, true);
    // Counter disabled in DMA2; the ESP's zero count is the 16-bit maximum.
    m.bus.write32(DMA, 0x300).unwrap();
    m.bus.write8(ESP + 12, 0x90).unwrap();
    for _ in 0..2048 {
        m.service_scsi().unwrap();
    }
    assert_eq!(irq(&mut m), (0x83, 4, 0x10));
    assert_eq!(m.bus.read8(ESP).unwrap(), 0);
    assert_eq!(m.bus.read8(ESP + 4).unwrap(), 0xf8);
    let received: Vec<_> = (0..2048)
        .map(|i| m.bus.read8(0x8000 + i).unwrap())
        .collect();
    assert_eq!(received, bytes[2048..4096]);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(std::fs::read(cdrom.0.as_path()).unwrap(), bytes);
}

fn parameters(m: &mut Machine, bytes: &[u8]) {
    for chunk in bytes.chunks(16) {
        for &byte in chunk {
            m.bus.write8(ESP + 8, byte).unwrap();
        }
        m.bus.write8(ESP + 12, 0x10).unwrap();
        assert_eq!(irq(m).2, 0x10);
    }
}

#[test]
fn mode_select_cd_blocks_changes_capacity_and_reads_across_sector_boundaries() {
    let mut m = fixture();
    let bytes: Vec<_> = (0..8192).map(|i| (i / 512 * 43 + i % 251) as u8).collect();
    let cdrom = attach(&mut m, MediaKind::Cdrom, &bytes);
    select(&mut m, 6, 0, &[0x1a, 0, 0x3f, 0, 12, 0]);
    assert_eq!(data(&mut m), [11, 0, 0, 8, 0, 0, 0, 4, 0, 0, 8, 0]);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(
        select(&mut m, 6, 0, &[0x15, 0x10, 0, 0, 12, 0]),
        (0x80, 4, 0x18)
    );
    parameters(&mut m, &[0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 2, 0]);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(
        m.bus
            .devices()
            .unwrap()
            .scsi
            .target(6)
            .unwrap()
            .block_size(),
        512
    );
    select(&mut m, 6, 0, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(data(&mut m), [0, 0, 0, 15, 0, 0, 2, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[8, 0, 0, 3, 2, 0]);
    assert_eq!(data(&mut m), bytes[1536..2560]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x1a, 0, 0, 0, 12, 0]);
    assert_eq!(data(&mut m), [11, 0, 0, 8, 0, 0, 0, 16, 0, 0, 2, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x1a, 8, 0x3f, 0, 4, 0]);
    assert_eq!(data(&mut m), [3, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x1a, 0, 0xbf, 0, 12, 0]);
    assert_eq!(data(&mut m), [11, 0, 0, 8, 0, 0, 0, 4, 0, 0, 8, 0]);
    assert_eq!(complete(&mut m), 0);
    m.bus.write8(ESP + 12, 3).unwrap();
    assert_eq!(irq(&mut m).2, 0x80);
    assert_eq!(
        m.bus
            .devices()
            .unwrap()
            .scsi
            .target(6)
            .unwrap()
            .block_size(),
        2048
    );
    assert_eq!(std::fs::read(cdrom.0.as_path()).unwrap(), bytes);
}

#[test]
fn mode_select_dma_partial_parameters_commit_only_when_complete_and_valid() {
    let mut m = fixture();
    let _cdrom = attach(&mut m, MediaKind::Cdrom, &[0; 8192]);
    let payload = [0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 2, 0];
    // PF=0's descriptor form is used by older initiators too.
    select(&mut m, 6, 0, &[0x15, 0, 0, 0, 12, 0]);
    m.load_image(0x8000, &payload).unwrap();
    dma(&mut m, 0xff00_0000, 6, false);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m), (0x90, 4, 0x10));
    assert_eq!(
        m.bus
            .devices()
            .unwrap()
            .scsi
            .target(6)
            .unwrap()
            .block_size(),
        2048
    );
    dma(&mut m, 0xff00_0006, 24, false);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m), (0x83, 4, 0x10));
    assert_eq!(m.bus.read8(ESP).unwrap(), 18);
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_000c);
    assert_eq!(
        m.bus
            .devices()
            .unwrap()
            .scsi
            .target(6)
            .unwrap()
            .block_size(),
        512
    );
    assert_eq!(complete(&mut m), 0);
    for (payload, asc) in [
        (vec![0, 0, 0], 0x1a),
        (vec![0, 0, 0, 8], 0x1a),
        (vec![0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 3, 0], 0x26),
        (vec![0, 0, 0, 8, 0, 0, 0, 0, 1, 0, 8, 0], 0x26),
        // A valid descriptor followed by an unsupported page cannot partly commit.
        (vec![0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 8, 0, 0x0e, 0], 0x26),
    ] {
        select(&mut m, 6, 0, &[0x15, 0x10, 0, 0, payload.len() as u8, 0]);
        parameters(&mut m, &payload);
        assert_eq!(complete(&mut m), 2);
        assert_eq!(
            m.bus
                .devices()
                .unwrap()
                .scsi
                .target(6)
                .unwrap()
                .block_size(),
            512
        );
        select(&mut m, 6, 0, &[3, 0, 0, 0, 18, 0]);
        let sense = data(&mut m);
        assert_eq!((sense[2], sense[12]), (5, asc));
        assert_eq!(complete(&mut m), 0);
    }
}

#[test]
fn ten_byte_mode_commands_use_eight_byte_headers_and_zero_length_is_noop() {
    let mut m = fixture();
    let _cdrom = attach(&mut m, MediaKind::Cdrom, &[0; 8192]);
    select(&mut m, 6, 0, &[0x55, 0x10, 0, 0, 0, 0, 0, 0, 16, 0]);
    parameters(&mut m, &[0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 4, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x5a, 0, 0x3f, 0, 0, 0, 0, 0, 16, 0]);
    assert_eq!(
        data(&mut m),
        [0, 14, 0, 0, 0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 4, 0]
    );
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x55, 0x10, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(
        m.bus
            .devices()
            .unwrap()
            .scsi
            .target(6)
            .unwrap()
            .block_size(),
        1024
    );
    select(&mut m, 6, 0, &[0x15, 0x11, 0, 0, 12, 0]);
    assert_eq!(complete(&mut m), 2); // Saving mode parameters is not supported.
}

#[test]
fn start_stop_unit_controls_readiness_and_rejects_unsupported_eject() {
    let mut m = fixture();
    let _cdrom = attach(&mut m, MediaKind::Cdrom, &[0; 2048]);
    select(&mut m, 6, 0, &[0x1b, 1, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 6, 0, &[3, 0, 0, 0, 18, 0]);
    let sense = data(&mut m);
    assert_eq!((sense[2], sense[12], sense[13]), (2, 4, 2));
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x1b, 1, 0, 0, 1, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x1b, 0, 0, 0, 2, 0]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 6, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
}

#[test]
fn cd_medium_lock_survives_invalid_requests_and_clears_on_allow_or_bus_reset() {
    let mut m = fixture();
    let _cdrom = attach(&mut m, MediaKind::Cdrom, &[0; 2048]);
    let locked = |m: &Machine| {
        m.bus
            .devices()
            .unwrap()
            .scsi
            .target(6)
            .unwrap()
            .removal_prevented()
    };
    assert!(!locked(&m));
    select(&mut m, 6, 0, &[0x1e, 0, 0, 0, 1, 0]);
    assert_eq!(complete(&mut m), 0);
    assert!(locked(&m));
    select(&mut m, 6, 0, &[0x1e, 0, 0, 0, 2, 0]);
    assert_eq!(complete(&mut m), 2);
    assert!(locked(&m));
    select(&mut m, 6, 0, &[0x1b, 0, 0, 0, 2, 0]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 6, 0, &[3, 0, 0, 0, 18, 0]);
    let sense = data(&mut m);
    assert_eq!((sense[2], sense[12], sense[13]), (5, 0x53, 2));
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 6, 0, &[0x1e, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    assert!(!locked(&m));
    select(&mut m, 6, 0, &[0x1e, 0, 0, 0, 1, 0]);
    assert_eq!(complete(&mut m), 0);
    m.bus.write8(ESP + 12, 3).unwrap();
    assert!(!locked(&m));
}

#[test]
fn writable_disk_reports_unprotected_modes_and_pio_writes_persist_across_reopen() {
    let mut m = fixture();
    let mut expected = vec![0x5a; 2048];
    let disk = attach_rw(&mut m, &expected);
    assert!(m.bus.devices().unwrap().scsi.target(3).unwrap().writable());
    for (cdb, wp) in [
        (vec![0x1a, 8, 0x3f, 0, 4, 0], 2),
        (vec![0x5a, 8, 0x3f, 0, 0, 0, 0, 0, 8, 0], 3),
    ] {
        select(&mut m, 3, 0, &cdb);
        assert_eq!(data(&mut m)[wp] & 0x80, 0);
        assert_eq!(complete(&mut m), 0);
    }
    let payload: Vec<_> = (0..512).map(|i| (i * 37) as u8).collect();
    assert_eq!(select(&mut m, 3, 0, &[0x0a, 0, 0, 1, 1, 0]).0 & 7, 0);
    parameters(&mut m, &payload);
    assert_eq!(complete(&mut m), 0);
    expected[512..1024].copy_from_slice(&payload);
    assert_eq!(std::fs::read(&disk.0).unwrap(), expected);
    drop(m);
    let mut reopened = fixture();
    reopened
        .bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach(3, MediaKind::Disk, &disk.0)
        .unwrap();
    select(&mut reopened, 3, 0, &[0x28, 0, 0, 0, 0, 1, 0, 0, 1, 0]);
    assert_eq!(data(&mut reopened), payload);
    assert_eq!(complete(&mut reopened), 0);
    assert!(!reopened
        .bus
        .devices()
        .unwrap()
        .scsi
        .target(3)
        .unwrap()
        .writable());
}

fn geometry_fixture() -> (Machine, Image) {
    let image = Image::new(&[]);
    std::fs::OpenOptions::new()
        .write(true)
        .open(image.0.as_path())
        .unwrap()
        .set_len(2 * 1024 * 1024 * 1024)
        .unwrap();
    let mut m = fixture();
    m.bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach_disk_rw(3, &image.0)
        .unwrap();
    (m, image)
}

// SCSI-2 tables 163 and 171, with the defined 2 GiB virtual geometry.
const FORMAT_PAGE: [u8; 24] = [
    3, 22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 2, 0, 0, 1, 0, 0, 0, 0, 0x40, 0, 0, 0,
];
const GEOMETRY_PAGE: [u8; 24] = [
    4, 22, 0, 0x10, 0, 16, 0, 0x10, 0, 0, 0x10, 0, 0, 0, 0xff, 0xff, 0xff, 0, 0, 0, 0x15, 0x18, 0,
    0,
];

#[test]
fn disk_mode_pages_describe_capacity_and_current_default_and_changeable_geometry() {
    let (mut m, _image) = geometry_fixture();
    for control in [0, 1, 2] {
        for expected in [FORMAT_PAGE, GEOMETRY_PAGE] {
            // Actual Solaris format auto-configuration command, page 3 then 4.
            select(
                &mut m,
                3,
                0,
                &[0x1a, 0, (control << 6) | expected[0], 0, 255, 0],
            );
            let bytes = data(&mut m);
            assert_eq!(&bytes[..12], &[35, 0, 0, 8, 0, 0x40, 0, 0, 0, 0, 2, 0]);
            assert_eq!(&bytes[12..14], &expected[..2]);
            assert_eq!(
                &bytes[14..],
                if control == 1 {
                    &[0; 22]
                } else {
                    &expected[2..]
                }
            );
            assert_eq!(complete(&mut m), 0);
        }
    }
    select(&mut m, 3, 0, &[0x5a, 8, 0x3f, 0, 0, 0, 0, 0, 255, 0]);
    let all = data(&mut m);
    assert_eq!(&all[..8], &[0, 54, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&all[8..32], &FORMAT_PAGE);
    assert_eq!(&all[32..56], &GEOMETRY_PAGE);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 0, &[0x1a, 8, 3, 0, 3, 0]);
    assert_eq!(data(&mut m), [27, 0, 0]); // Full length survives allocation truncation.
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 0, &[0x1a, 8, 0xc3, 0, 255, 0]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 3, 0, &[3, 0, 0, 0, 18, 0]);
    let sense = data(&mut m);
    assert_eq!((sense[2], sense[12]), (5, 0x39));
    assert_eq!(complete(&mut m), 0);
    let _cd = attach(&mut m, MediaKind::Cdrom, &[0; 2048]);
    select(&mut m, 6, 0, &[0x1a, 0, 3, 0, 255, 0]);
    assert_eq!(complete(&mut m), 2);
}

#[test]
fn fixed_geometry_mode_select_accepts_unchanged_pages_and_rejects_partial_or_changed_parameters() {
    let (mut m, image) = geometry_fixture();
    let mut unchanged = vec![0; 4];
    unchanged.extend_from_slice(&FORMAT_PAGE);
    unchanged.extend_from_slice(&GEOMETRY_PAGE);
    select(&mut m, 3, 0, &[0x15, 0x10, 0, 0, unchanged.len() as u8, 0]);
    parameters(&mut m, &unchanged);
    assert_eq!(complete(&mut m), 0);
    let mut defaults = unchanged.clone();
    defaults[14..18].fill(0); // Page 3 sector count/bytes zero -> target defaults.
    defaults[18..20].fill(0xff); // Interleave ignored on MODE SELECT.
    select(&mut m, 3, 0, &[0x15, 0x10, 0, 0, defaults.len() as u8, 0]);
    parameters(&mut m, &defaults);
    assert_eq!(complete(&mut m), 0);
    let mut changed = unchanged.clone();
    changed[33] = 8; // Geometry page head count, not changeable.
    let mut saved = unchanged.clone();
    saved[4] |= 0x80; // PS reserved on MODE SELECT.
    for (payload, asc) in [
        (changed, 0x26),
        (saved, 0x26),
        (unchanged[..51].to_vec(), 0x1a),
    ] {
        select(&mut m, 3, 0, &[0x15, 0x10, 0, 0, payload.len() as u8, 0]);
        parameters(&mut m, &payload);
        assert_eq!(complete(&mut m), 2);
        select(&mut m, 3, 0, &[3, 0, 0, 0, 18, 0]);
        let sense = data(&mut m);
        assert_eq!((sense[2], sense[12]), (5, asc));
        assert_eq!(complete(&mut m), 0);
    }
    select(&mut m, 3, 0, &[0x1a, 8, 4, 0, 255, 0]);
    assert_eq!(&data(&mut m)[4..], &GEOMETRY_PAGE);
    assert_eq!(complete(&mut m), 0);
    assert_eq!(
        std::fs::metadata(image.0.as_path()).unwrap().len(),
        2 * 1024 * 1024 * 1024
    );
}

#[test]
fn disk_dma_write_commits_only_full_payload_with_exact_residual_and_fua() {
    let mut m = fixture();
    let original = vec![0x33; 2048];
    let disk = attach_rw(&mut m, &original);
    let payload: Vec<_> = (0..1024).map(|i| (i / 251 + i * 13) as u8).collect();
    m.load_image(0x8000, &payload).unwrap();
    select(&mut m, 3, 0, &[0x2a, 0x18, 0, 0, 0, 1, 0, 0, 2, 0]); // DPO/FUA
    dma(&mut m, 0xff00_0000, 256, false);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m).0 & 7, 0);
    assert_eq!(std::fs::read(&disk.0).unwrap(), original);
    dma(&mut m, 0xff00_0100, 1000, false);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m).0 & 7, 3);
    assert_eq!(m.bus.read32(DMA + 4).unwrap(), 0xff00_0400);
    assert_eq!(m.bus.read32(DMA + 8).unwrap(), 232);
    assert_eq!(m.bus.read8(ESP).unwrap(), 232);
    assert_eq!(complete(&mut m), 0);
    let mut expected = original;
    expected[512..1536].copy_from_slice(&payload);
    assert_eq!(std::fs::read(disk.0.as_path()).unwrap(), expected);
}

#[test]
fn partial_disk_write_then_dma_fault_and_bus_reset_preserve_image() {
    let mut m = fixture();
    let original = vec![0x72; 1024];
    let disk = attach_rw(&mut m, &original);
    m.load_image(0x8000, &[0xcc; 128]).unwrap();
    select(&mut m, 3, 0, &[0x0a, 0, 0, 0, 1, 0]);
    dma(&mut m, 0xff00_0000, 128, false);
    m.bus.write8(ESP + 12, 0x90).unwrap();
    assert_eq!(irq(&mut m).0 & 7, 0);
    dma(&mut m, 0xff00_2000, 384, false); // unmapped IOPTE
    m.bus.write8(ESP + 12, 0x90).unwrap();
    m.service_scsi().unwrap();
    assert_ne!(m.bus.read32(DMA).unwrap() & 2, 0);
    assert_eq!(std::fs::read(&disk.0).unwrap(), original);
    m.bus.write8(ESP + 12, 3).unwrap();
    assert_eq!(irq(&mut m).2, 0x80);
    assert_eq!(std::fs::read(disk.0.as_path()).unwrap(), original);
    assert!(m.bus.devices().unwrap().scsi.target(3).unwrap().writable());
}

#[test]
fn disk_write_lengths_bounds_reserved_fields_and_cache_sync_are_checked() {
    let mut m = fixture();
    let original = vec![0x42; 257 * 512];
    let disk = attach_rw(&mut m, &original);
    for (cdb, asc) in [
        (vec![0x2a, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 1, 0], 0x21),
        (vec![0x0a, 0, 1, 0, 2, 0], 0x21),
        (vec![0x2a, 1, 0, 0, 0, 0, 0, 0, 1, 0], 0x24),
        (vec![0x2a, 0, 0, 0, 0, 0, 1, 0, 1, 0], 0x24),
        (vec![0x0a, 0, 0, 0, 1, 1], 0x24),
        (vec![0x35, 2, 0, 0, 0, 0, 0, 0, 0, 0], 0x24),
        (vec![0x35, 0, 0, 0, 1, 0, 0, 0, 2, 0], 0x21),
    ] {
        select(&mut m, 3, 0, &cdb);
        assert_eq!(complete(&mut m), 2);
        select(&mut m, 3, 0, &[3, 0, 0, 0, 18, 0]);
        let sense = data(&mut m);
        assert_eq!((sense[2], sense[12]), (5, asc));
        assert_eq!(complete(&mut m), 0);
    }
    select(&mut m, 3, 0, &[0x2a, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0); // ten-byte zero count is a no-op
    assert_eq!(std::fs::read(&disk.0).unwrap(), original);
    select(&mut m, 3, 0, &[0x0a, 0, 0, 1, 0, 0]); // six-byte zero means 256 blocks
    parameters(&mut m, &vec![0x99; 256 * 512]);
    assert_eq!(complete(&mut m), 0);
    select(&mut m, 3, 0, &[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(complete(&mut m), 0);
    let written = std::fs::read(disk.0.as_path()).unwrap();
    assert_eq!(&written[..512], &original[..512]);
    assert_eq!(&written[512..], vec![0x99; 256 * 512]);
}

#[test]
fn disk_write_host_failure_returns_sense_without_growing_changed_image() {
    let mut m = fixture();
    let disk = attach_rw(&mut m, &[0; 512]);
    select(&mut m, 3, 0, &[0x0a, 0, 0, 0, 1, 0]);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&disk.0)
        .unwrap()
        .set_len(0)
        .unwrap();
    parameters(&mut m, &[0x55; 512]);
    assert_eq!(complete(&mut m), 2);
    select(&mut m, 3, 0, &[3, 0, 0, 0, 18, 0]);
    let sense = data(&mut m);
    assert_eq!((sense[2], sense[12]), (3, 0x0c));
    assert_eq!(complete(&mut m), 0);
    assert_eq!(std::fs::metadata(disk.0.as_path()).unwrap().len(), 0);
}

#[test]
fn cdrom_still_rejects_writes_and_writable_attach_never_creates_missing_files() {
    let mut m = fixture();
    let cd = attach(&mut m, MediaKind::Cdrom, &[0x81; 2048]);
    for cdb in [
        vec![0x0a, 0, 0, 0, 1, 0],
        vec![0x2a, 0, 0, 0, 0, 0, 0, 0, 1, 0],
    ] {
        select(&mut m, 6, 0, &cdb);
        assert_eq!(complete(&mut m), 2);
        select(&mut m, 6, 0, &[3, 0, 0, 0, 18, 0]);
        let sense = data(&mut m);
        assert_eq!((sense[2], sense[12]), (7, 0x27));
        assert_eq!(complete(&mut m), 0);
    }
    let missing = cd.0.with_extension("missing");
    assert!(!missing.exists());
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach_disk_rw(3, &missing)
        .is_err());
    assert!(!missing.exists());
    assert!(m.bus.devices().unwrap().scsi.target(3).is_none());
    assert_eq!(std::fs::read(cd.0.as_path()).unwrap(), [0x81; 2048]);
}
