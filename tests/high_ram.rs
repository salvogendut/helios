use helios::bus::{Bus, BusErrorKind};
use helios::cpu::Cpu;
use helios::dma::{DmaError, DmaFault};
use helios::machine::{InstalledVideo, Machine, MachineProfile, ProcessorModuleState};
use helios::mmu::Width;
use helios::system_bus::SystemBus;
use helios::topology::ProcessorModule;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

const MIB: usize = 1024 * 1024;
static HEAVY: Mutex<()> = Mutex::new(());

fn heavy() -> MutexGuard<'static, ()> {
    HEAVY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const PAGE: u32 = 0xff00_0000;

fn machine(profile: MachineProfile, mib: usize, video: InstalledVideo) -> Machine {
    Machine::with_profile_video(
        profile,
        mib * MIB,
        &0x0100_0000u32.to_be_bytes(),
        profile.default_nvram(),
        video,
    )
    .unwrap()
}

fn configure_iommu(m: &mut Machine, table: u64) {
    let io = m.bus.layout().io_control;
    m.bus.write32(io, 1).unwrap();
    m.bus.write32(io + 4, (table >> 4) as u32).unwrap();
    m.bus.write32(io + 0x1008, 1 << 20).unwrap(); // standalone onboard master 5
    m.bus.write32(io + 0x2000, 1 << 21).unwrap(); // SS-5 master 5
    m.bus.write32(io + 0x14, 0).unwrap(); // configure helper starts from a cold I/O cache
}

fn flush_io(m: &mut Machine) {
    let io = m.bus.layout().io_control;
    m.bus.write32(io + 0x14, 0).unwrap();
}

fn map_page(m: &mut Machine, table: u64, page: u32, physical: u64) {
    let entry = table + u64::from((page & 0x00ff_f000) >> 10);
    m.bus
        .write32(entry, ((physical >> 4) as u32 & 0xffff_ff00) | 0x6)
        .unwrap();
}

fn high_pointer(m: &mut Machine, address: u64, value: u32) {
    m.bus.write32(address, value).unwrap();
}

struct Image(PathBuf);

impl Image {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-high-ram-{}-{}.img",
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
        let _ = std::fs::remove_file(&self.0);
    }
}

fn cpu_mapping(cpu: &mut Cpu, bus: &mut SystemBus, base: u64, context: u32, physical: u32) {
    bus.write32(
        base + u64::from(context) * 4,
        ((base + 0x1000) >> 4) as u32 | 1,
    )
    .unwrap();
    bus.write32(base + 0x1000, ((base + 0x2000) >> 4) as u32 | 1)
        .unwrap();
    bus.write32(base + 0x2000, ((base + 0x3000) >> 4) as u32 | 1)
        .unwrap();
    bus.write32(base + 0x3010, (physical >> 4) | 0x8e).unwrap();
    cpu.mmu
        .store(bus, 0x100, 4, Width::Word, (base >> 4) as u32)
        .unwrap();
    cpu.mmu.store(bus, 0x200, 4, Width::Word, context).unwrap();
    cpu.mmu.store(bus, 0, 4, Width::Word, 1).unwrap();
}

#[test]
fn ss20_iommu_tables_and_dma_use_both_halves_of_a_32_mib_module() {
    let mut m = machine(MachineProfile::Ss20, 32, InstalledVideo::None);
    // /memory/reg advertises the second 16 MiB region at PA 32 MiB.
    // An IOPTE table there must not overwrite the sentinel at PA zero.
    let table = 32 * MIB as u64;
    m.bus.write32(0, 0x1234_5678).unwrap();
    configure_iommu(&mut m, table);
    map_page(&mut m, table, PAGE, 0x0201_0000);
    assert_eq!(m.bus.read32(0).unwrap(), 0x1234_5678);
    m.bus.write32(0x0001_0000, 0x5566_7788).unwrap();
    m.dma_write(5, PAGE, Width::Word, 0xaabb_ccdd).unwrap();
    assert_eq!(m.bus.read32(0x0201_0000).unwrap(), 0xaabb_ccdd);
    assert_eq!(m.bus.read32(0x0301_0000).unwrap(), 0xaabb_ccdd);
    assert_eq!(m.bus.read32(0x0001_0000).unwrap(), 0x5566_7788);
    assert_eq!(m.dma_read(5, PAGE, Width::Word), Ok(0xaabb_ccdd));
}

#[test]
fn high_tables_targets_bypass_and_empty_banks_are_board_bounded() {
    let _guard = heavy();
    let mut m = machine(MachineProfile::Ss20, 272, InstalledVideo::Sx);
    configure_iommu(&mut m, 0x10000);
    map_page(&mut m, 0x10000, PAGE, 0x10fe_f000);
    high_pointer(&mut m, 0x10fe_fffc, 0x1111_2222);
    assert_eq!(m.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0x1111_2222));

    // A page table above 256 MiB is usable when installed and does not alias
    // the low table that maps the same DVMA page to a different high address.
    configure_iommu(&mut m, 0x1000_0000);
    m.bus.write32(0x1000_0004, 0x806).unwrap(); // second page -> low PA 0x8000
    high_pointer(&mut m, 0x8000, 0x5555_6666);
    flush_io(&mut m);
    assert_eq!(m.dma_read(5, PAGE + 0x1000, Width::Word), Ok(0x5555_6666));
    map_page(&mut m, 0x1000_0000, PAGE, 0x10ff_f000);
    high_pointer(&mut m, 0x10ff_fffc, 0x3333_4444);
    flush_io(&mut m);
    assert_eq!(m.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0x3333_4444));
    assert_eq!(
        m.dma_read(5, PAGE + 1, Width::Word),
        Err(DmaError::Fault {
            address: PAGE + 1,
            kind: DmaFault::Misaligned
        })
    );

    m.bus.write32(0x1000_0000, 0x010f_ff02).unwrap(); // high target, read-only
    flush_io(&mut m);
    assert_eq!(
        m.dma_write(5, PAGE + 0xffc, Width::Word, 0),
        Err(DmaError::Fault {
            address: PAGE + 0xffc,
            kind: DmaFault::WriteProtected
        })
    );
    assert_eq!(m.bus.read32(0x10ff_fffc).unwrap(), 0x3333_4444);
    m.bus.write32(0x1000_0000, 0).unwrap();
    flush_io(&mut m);
    assert_eq!(
        m.dma_read(5, PAGE + 0xffc, Width::Word),
        Err(DmaError::Fault {
            address: PAGE + 0xffc,
            kind: DmaFault::InvalidPte
        })
    );
    assert_eq!(m.bus.read32(0x10ff_fffc).unwrap(), 0x3333_4444);

    // The same address is not a table location when its bank is empty.
    configure_iommu(&mut m, 0x1400_0000);
    assert_eq!(
        m.dma_read(5, PAGE, Width::Word),
        Err(DmaError::Unsupported("I/O page table outside DRAM"))
    );

    // Empty DSIMM banks inside the aperture retain zero-read/discard-write
    // completion, but are not valid page-table storage.
    configure_iommu(&mut m, 0x10000);
    map_page(&mut m, 0x10000, PAGE, 0x1400_0000);
    m.dma_write(5, PAGE, Width::Word, 0xdead_beef).unwrap();
    assert_eq!(m.dma_read(5, PAGE, Width::Word), Ok(0));
    assert_eq!(m.bus.read32(0x1400_0000).unwrap(), 0);

    // The exclusive aperture endpoint is not a DVMA target.
    map_page(&mut m, 0x10000, PAGE, 0x2000_0000);
    flush_io(&mut m);
    assert_eq!(
        m.dma_read(5, PAGE, Width::Word),
        Err(DmaError::Unsupported("non-DRAM DVMA target"))
    );

    // Bypass follows the board aperture, unlike the raw 256 MiB bound.
    let io = m.bus.layout().io_control;
    m.bus.write32(io + 0x1010, 1).unwrap();
    m.bus.write32(io + 0x1008, 1 << 16).unwrap();
    high_pointer(&mut m, 0x1000_0000, 0x3333_4444);
    assert_eq!(m.dma_read(0, 0x1000_0000, Width::Word), Ok(0x3333_4444));
    assert_eq!(
        m.dma_read(0, 0x2000_0000, Width::Word),
        Err(DmaError::Fault {
            address: 0x2000_0000,
            kind: DmaFault::InvalidBypassAddress
        })
    );

    // A high mapping remains cached until the explicit page invalidation.
    configure_iommu(&mut m, 0x10000);
    map_page(&mut m, 0x10000, PAGE, 0x10fe_f000);
    high_pointer(&mut m, 0x10fe_fffc, 0xaaaa_bbbb);
    assert_eq!(m.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0xaaaa_bbbb));
    map_page(&mut m, 0x10000, PAGE, 0x10ff_f000);
    assert_eq!(m.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0xaaaa_bbbb));
    m.bus.write32(io + 0x18, PAGE).unwrap();
    assert_eq!(m.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0x3333_4444));
}

#[test]
fn full_capacities_keep_last_words_and_endpoint_faults() {
    let _guard = heavy();
    // SS-5 retains its 256 MiB integrated bound.
    let mut ss5 = machine(MachineProfile::Ss5, 256, InstalledVideo::None);
    configure_iommu(&mut ss5, 0x10000);
    map_page(&mut ss5, 0x10000, PAGE, 0x0fff_f000);
    high_pointer(&mut ss5, 0x0fff_fffc, 0x5555_6666);
    assert_eq!(ss5.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0x5555_6666));
    ss5.cpu
        .mmu
        .store(&mut ss5.bus, 0x0fff_fffc, 0x20, Width::Word, 0x7777_8888)
        .unwrap();
    assert_eq!(
        ss5.cpu
            .mmu
            .load(&mut ss5.bus, 0x0fff_fffc, 0x20, Width::Word)
            .unwrap(),
        0x7777_8888
    );
    map_page(&mut ss5, 0x10000, PAGE, 0x1000_0000);
    flush_io(&mut ss5);
    assert_eq!(
        ss5.dma_read(5, PAGE, Width::Word),
        Err(DmaError::Unsupported("non-DRAM DVMA target"))
    );
    drop(ss5);

    // SS-20/SX has 448 MiB of DRAM; bank 7 remains the empty VSIMM bank.
    let mut sx = machine(MachineProfile::Ss20, 448, InstalledVideo::Sx);
    configure_iommu(&mut sx, 0x10000);
    map_page(&mut sx, 0x10000, PAGE, 0x1bff_f000);
    high_pointer(&mut sx, 0x1bff_fffc, 0x1234_5678);
    assert_eq!(sx.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0x1234_5678));
    sx.cpu
        .mmu
        .store(&mut sx.bus, 0x1bff_fffc, 0x20, Width::Word, 0x9abc_def0)
        .unwrap();
    assert_eq!(sx.bus.read32(0x1bff_fffc).unwrap(), 0x9abc_def0);
    assert_eq!(sx.bus.read32(0x1c00_0000).unwrap(), 0);
    map_page(&mut sx, 0x10000, PAGE, 0x1c00_0000);
    flush_io(&mut sx);
    assert_eq!(
        sx.dma_write(5, PAGE, Width::Word, 0xffff_ffff),
        Err(DmaError::Unsupported("non-DRAM DVMA target"))
    );
    assert_eq!(sx.bus.read32(0x1c00_0000).unwrap(), 0);
    configure_iommu(&mut sx, 0x1c00_0000);
    assert_eq!(
        sx.dma_read(5, PAGE, Width::Word),
        Err(DmaError::Unsupported("I/O page table outside DRAM"))
    );
    drop(sx);

    // SS-20 without a VSIMM reaches bank 7, including a high-resident table.
    let mut full = machine(MachineProfile::Ss20, 512, InstalledVideo::None);
    configure_iommu(&mut full, 0x1ff0_0000);
    map_page(&mut full, 0x1ff0_0000, PAGE, 0x1fff_f000);
    high_pointer(&mut full, 0x1fff_fffc, 0xfeed_face);
    assert_eq!(full.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0xfeed_face));
    full.cpu
        .mmu
        .store(&mut full.bus, 0x1fff_fffc, 0x20, Width::Word, 0x0bad_f00d)
        .unwrap();
    assert_eq!(full.bus.read32(0x1fff_fffc).unwrap(), 0x0bad_f00d);
    full.cpu
        .mmu
        .store(&mut full.bus, 0x1ffe_0000, 0x20, Width::Byte, 0xab)
        .unwrap();
    full.cpu
        .mmu
        .store(&mut full.bus, 0x1ffe_0002, 0x20, Width::Half, 0xcdef)
        .unwrap();
    full.cpu
        .mmu
        .store(&mut full.bus, 0x1ffe_0004, 0x20, Width::Word, 0x1234_5678)
        .unwrap();
    full.cpu
        .mmu
        .store_doubleword(&mut full.bus, 0x1ffe_0008, 0x20, [0x89ab_cdef, 0x0123_4567])
        .unwrap();
    assert_eq!(
        full.bus.read_doubleword(0x1ffe_0008).unwrap(),
        [0x89ab_cdef, 0x0123_4567]
    );
    full.cpu
        .mmu
        .store(&mut full.bus, 0x1ffe_0010, 0x20, Width::Word, 0x55aa_55aa)
        .unwrap();
    assert_eq!(
        full.cpu
            .mmu
            .swap(&mut full.bus, 0x1ffe_0010, 0x20, 0xaa55_aa55)
            .unwrap(),
        0x55aa_55aa
    );
    full.bus.write8(0x1ffe_0014, 0x3c).unwrap();
    assert_eq!(
        full.cpu
            .mmu
            .load_store_byte(&mut full.bus, 0x1ffe_0014, 0x20)
            .unwrap(),
        0x3c
    );
    assert_eq!(full.bus.read8(0x1ffe_0014).unwrap(), 0xff);
    assert_eq!(
        full.bus.read32(0x2000_0000).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    map_page(&mut full, 0x1ff0_0000, PAGE, 0x2000_0000);
    flush_io(&mut full);
    assert_eq!(
        full.dma_read(5, PAGE, Width::Word),
        Err(DmaError::Unsupported("non-DRAM DVMA target"))
    );

    // Independent CPU-2 translation reaches a different high page.
    let mut cpu2 = ProcessorModuleState::new(
        MachineProfile::Ss20,
        ProcessorModule::from_processor(2).unwrap(),
    )
    .unwrap();
    cpu_mapping(&mut full.cpu, &mut full.bus, 0x1000_0000, 0, 0x1ffe_0000);
    cpu_mapping(
        &mut cpu2.cpu,
        &mut full.bus,
        0x1100_0000,
        0x100,
        0x1fff_0000,
    );
    full.cpu
        .mmu
        .store(&mut full.bus, 0x4000, 11, Width::Word, 0x1111_1111)
        .unwrap();
    cpu2.cpu
        .mmu
        .store(&mut full.bus, 0x4000, 11, Width::Word, 0x2222_2222)
        .unwrap();
    assert_eq!(
        full.cpu
            .mmu
            .load(&mut full.bus, 0x4000, 11, Width::Word)
            .unwrap(),
        0x1111_1111
    );
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut full.bus, 0x4000, 11, Width::Word)
            .unwrap(),
        0x2222_2222
    );

    // Whole reset retains RAM and the selected capacity.
    full.reset();
    assert_eq!(full.bus.dram().unwrap().bank_sizes()[7], 64 * MIB);
    assert_eq!(full.bus.read32(0x1fff_fffc).unwrap(), 0x0bad_f00d);
}

#[test]
fn axil_uses_the_same_high_memory_dma_path() {
    let _guard = heavy();
    let mut m = machine(MachineProfile::Axil320, 272, InstalledVideo::None);
    configure_iommu(&mut m, 0x1000_0000);
    map_page(&mut m, 0x1000_0000, PAGE, 0x10ff_f000);
    high_pointer(&mut m, 0x10ff_fffc, 0xa5a5_5a5a);
    assert_eq!(m.dma_read(5, PAGE + 0xffc, Width::Word), Ok(0xa5a5_5a5a));
}

fn scsi_irq(m: &mut Machine, esp: u64) -> (u8, u8, u8) {
    for _ in 0..1000 {
        m.service_scsi().unwrap();
        m.advance_ns(100);
        if m.bus.devices().unwrap().esp.irq() {
            let status = m.bus.read8(esp + 16).unwrap();
            let sequence = m.bus.read8(esp + 24).unwrap();
            let interrupt = m.bus.read8(esp + 20).unwrap();
            return (status, sequence, interrupt);
        }
    }
    panic!("ESP did not interrupt");
}

fn scsi_dma(m: &mut Machine, esp: u64, dma: u64, address: u32, count: u16, write: bool) {
    m.bus
        .write32(dma, 0x2220 | if write { 0x100 } else { 0 })
        .unwrap();
    m.bus.write32(dma + 8, u32::from(count)).unwrap();
    m.bus.write32(dma + 4, address).unwrap();
    m.bus.write8(esp, count as u8).unwrap();
    m.bus.write8(esp + 4, (count >> 8) as u8).unwrap();
}

fn scsi_write(m: &mut Machine, esp: u64, dma: u64) {
    m.bus.write8(esp + 16, 3).unwrap();
    m.bus.write8(esp + 8, 0x80).unwrap();
    for byte in [0x2au8, 0, 0, 0, 0, 1, 0, 0, 1, 0] {
        m.bus.write8(esp + 8, byte).unwrap();
    }
    m.bus.write8(esp + 12, 0x42).unwrap();
    let (status, sequence, interrupt) = scsi_irq(m, esp);
    assert!(matches!(status & 7, 0 | 1));
    assert_eq!(sequence, 4);
    assert_eq!(interrupt, 0x18);
    scsi_dma(m, esp, dma, PAGE, 512, false);
    m.bus.write8(esp + 12, 0x90).unwrap();
    scsi_irq(m, esp);
}

#[test]
fn scsi_writes_high_memory_to_a_disk_sector() {
    let _guard = heavy();
    let mut m = machine(MachineProfile::Ss20, 272, InstalledVideo::Sx);
    let esp = m.bus.layout().esp;
    let dma = m.bus.layout().scsi_dma;
    m.bus.write8(esp + 12, 0).unwrap();
    m.bus.write8(esp + 32, 7).unwrap();
    m.bus.write8(esp + 20, 1).unwrap();
    configure_iommu(&mut m, 0x10000);
    map_page(&mut m, 0x10000, PAGE, 0x10ff_e000);
    let original = vec![0x33; 2048];
    let image = Image::new(&original);
    m.bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach_disk_rw(3, &image.0)
        .unwrap();
    let payload: Vec<u8> = (0..512).map(|i| (i * 37 + 11) as u8).collect();
    m.load_image(0x10ff_e000, &payload).unwrap();
    scsi_write(&mut m, esp, dma);
    let mut expected = original;
    expected[512..1024].copy_from_slice(&payload);
    assert_eq!(std::fs::read(&image.0).unwrap(), expected);
}

fn ethernet_csr(m: &mut Machine, mac: u64, register: u16, value: u16) {
    m.bus.write16(mac + 2, register).unwrap();
    m.bus.write16(mac, value).unwrap();
}

fn ethernet_status(m: &mut Machine, mac: u64) -> u16 {
    m.bus.write16(mac + 2, 0).unwrap();
    m.bus.read16(mac).unwrap()
}

fn ethernet_pump(m: &mut Machine, count: usize) {
    for _ in 0..count {
        m.service_ethernet().unwrap();
    }
}

fn ethernet_start(m: &mut Machine, mac: u64) -> u16 {
    ethernet_pump(m, 12);
    let status = ethernet_status(m, mac);
    assert_eq!(status, 0x1c1);
    ethernet_csr(m, mac, 0, 0x142);
    status
}

#[test]
fn ethernet_dma_uses_high_descriptors_and_buffers() {
    let _guard = heavy();
    let mut m = machine(MachineProfile::Ss20, 272, InstalledVideo::Sx);
    let mac = m.bus.layout().ethernet;
    let dma = m.bus.layout().ethernet_dma();
    let irq = m.bus.layout().system_interrupt;
    configure_iommu(&mut m, 0x10000);
    let base = 0x1000_0000u64;
    map_page(&mut m, 0x10000, PAGE, base);
    for (index, word) in [
        0u16, 0x0002, 0, 0x0100, 0, 0, 0, 0, 0x100, 0x2000, 0x200, 0x2000,
    ]
    .into_iter()
    .enumerate()
    {
        m.bus.write16(base + index as u64 * 2, word).unwrap();
    }
    ethernet_csr(&mut m, mac, 1, 0);
    ethernet_csr(&mut m, mac, 2, 0);
    ethernet_csr(&mut m, mac, 3, 4);
    m.bus.write32(dma, 0x10).unwrap();
    m.bus.write32(irq + 8, (1 << 31) | (1 << 16)).unwrap();
    ethernet_csr(&mut m, mac, 0, 0x41);
    ethernet_start(&mut m, mac);

    for (index, word) in [0x0300u16, 0x8300, 0xffc4, 0].into_iter().enumerate() {
        m.bus
            .write16(base + 0x200 + index as u64 * 2, word)
            .unwrap();
    }
    let packet: Vec<_> = (0..60).map(|byte| byte as u8).collect();
    for (index, byte) in packet.iter().enumerate() {
        m.bus.write8(base + 0x300 + index as u64, *byte).unwrap();
    }
    ethernet_csr(&mut m, mac, 0, 0x248);
    ethernet_pump(&mut m, 64);
    assert_eq!(m.bus.read16(base + 0x202).unwrap(), 0x8300);
    assert!(m.bus.devices_mut().unwrap().ethernet.take_frame().is_none());
    ethernet_pump(&mut m, 1);
    assert_eq!(m.bus.read16(base + 0x202).unwrap(), 0x0300);
    assert_eq!(
        m.bus.devices_mut().unwrap().ethernet.take_frame(),
        Some(packet)
    );

    // Receive into the same high page, including the FCS and ownership write.
    for (index, word) in [0x0300u16, 0x8000, 0xff80, 0].into_iter().enumerate() {
        m.bus
            .write16(base + 0x100 + index as u64 * 2, word)
            .unwrap();
    }
    let mut received: Vec<_> = (0..60).map(|byte| byte as u8).collect();
    received[..6].fill(0xff);
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .ethernet
        .receive_frame(&received));
    ethernet_pump(&mut m, 68);
    assert_eq!(m.bus.read16(base + 0x102).unwrap(), 0x8000);
    assert_eq!(m.bus.read16(base + 0x106).unwrap(), 0);
    ethernet_pump(&mut m, 1);
    assert_eq!(m.bus.read16(base + 0x106).unwrap(), 64);
    assert_eq!(m.bus.read16(base + 0x102).unwrap(), 0x8000);
    ethernet_pump(&mut m, 1);
    assert_eq!(m.bus.read16(base + 0x102).unwrap(), 0x0300);
    for (index, expected) in received.iter().enumerate() {
        assert_eq!(m.bus.read8(base + 0x300 + index as u64).unwrap(), *expected);
    }
    assert_eq!(
        &(0..4)
            .map(|index| m.bus.read8(base + 0x33c + index).unwrap())
            .collect::<Vec<_>>(),
        &[142, 62, 90, 255]
    );
}
