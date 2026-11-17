use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_S};
use helios::devices::serial::Channel;
use helios::machine::{InstalledVideo, Machine, MachineProfile};
use helios::mmu::Width;

fn machine(profile: MachineProfile, cpus: usize, guards: bool) -> Machine {
    let mut m = Machine::with_profile_processors_video(
        profile,
        16 * 1024 * 1024,
        &[1, 0, 0, 0],
        profile.default_nvram(),
        cpus,
        InstalledVideo::None,
    )
    .unwrap();
    m.set_device_service_guards(guards);
    // Enable traps with an actual guest WRPSR, followed by its spacing NOPs.
    // A RAM branch loop leaves both devices and exception entry observable.
    let words = [
        0x8188_6000u32,
        0x0100_0000,
        0x0100_0000,
        0x0100_0000,
        0x1080_0000,
        0x0100_0000,
    ];
    let code: Vec<_> = words.into_iter().flat_map(u32::to_be_bytes).collect();
    m.load_image(0x1000, &code).unwrap();
    for cpu in [0, 2].into_iter().take(cpus) {
        if cpu == 0 {
            m.cpu.mmu.store(&mut m.bus, 0, 4, Width::Word, 0).unwrap();
            m.cpu.write_reg(1, PSR_S | PSR_ET);
            m.cpu.set_pc_npc(0x1000, 0x1004);
            for _ in 0..4 {
                m.cpu.step(&mut m.bus).unwrap();
            }
        } else {
            let (module, bus) = m.secondary_cpu_with_bus_mut().unwrap();
            module.cpu.mmu.store(bus, 0, 4, Width::Word, 0).unwrap();
            module.cpu.write_reg(1, PSR_S | PSR_ET);
            module.cpu.set_pc_npc(0x1000, 0x1004);
            for _ in 0..4 {
                module.cpu.step(bus).unwrap();
            }
        }
    }
    if cpus == 2 {
        assert!(m.set_processor_arbitration(2, true));
    }
    m
}

fn same(a: &Machine, b: &Machine, phase: &str) {
    assert_eq!(
        a.diagnostic_snapshot()
            .differences(&b.diagnostic_snapshot()),
        Vec::<&'static str>::new(),
        "{phase}"
    );
}

fn step_pair(a: &mut Machine, b: &mut Machine, scheduled: bool) {
    if scheduled {
        assert_eq!(a.step_scheduled().unwrap(), b.step_scheduled().unwrap());
    } else {
        assert_eq!(a.step().unwrap(), b.step().unwrap());
    }
}

fn enable_serial_and_inject(m: &mut Machine, dma_reset: bool) {
    let l = m.bus.layout();
    for (reg, val) in [(3, 0xc1), (1, 0x10), (9, 8)] {
        m.bus.write8(l.serial + 4, reg).unwrap();
        m.bus.write8(l.serial + 4, val).unwrap();
    }
    // Serial is system source 15, delivered at PIL 12.
    m.bus
        .write32(l.system_interrupt + 8, (1 << 31) | (1 << 15))
        .unwrap();
    m.bus
        .write32(l.scsi_dma, if dma_reset { 0x80 } else { 0 })
        .unwrap();
    assert_eq!(m.bus.interrupt_levels(), 0);
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .serial
        .receive(Channel::A, b'X'));
    assert!(m.bus.devices().unwrap().serial.irq());
    assert_eq!(m.bus.interrupt_levels(), 0);
}

#[test]
fn ordinary_idle_serial_wakeup_matches_reference() {
    let mut a = machine(MachineProfile::Ss5, 1, true);
    let mut b = machine(MachineProfile::Ss5, 1, false);
    for m in [&mut a, &mut b] {
        enable_serial_and_inject(m, false);
    }
    let ra = a.step().unwrap();
    let rb = b.step().unwrap();
    assert!(matches!(ra, Step::InterruptTaken { level: 12, .. }));
    assert_eq!(ra, rb);
    same(&a, &b, "ordinary IRQ wakeup");
}

#[test]
fn held_scsi_reset_preserves_serial_interrupt_boundary_up() {
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        let mut a = machine(profile, 1, true);
        let mut b = machine(profile, 1, false);
        for m in [&mut a, &mut b] {
            enable_serial_and_inject(m, true);
        }
        same(&a, &b, "before first step");
        let ra = a.step().unwrap();
        let rb = b.step().unwrap();
        assert_eq!(
            ra, rb,
            "{profile:?}: held DMA reset must not change IRQ entry boundary"
        );
        same(&a, &b, "after first step");
    }
}

#[test]
fn held_scsi_reset_preserves_serial_interrupt_boundary_smp() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut a = machine(profile, 2, true);
        let mut b = machine(profile, 2, false);
        for m in [&mut a, &mut b] {
            enable_serial_and_inject(m, true);
        }
        same(&a, &b, "before tied deadline");
        assert_eq!(a.step_scheduled(), b.step_scheduled(), "{profile:?}");
        same(&a, &b, "after tied deadline");
    }
}

#[test]
fn timeout_and_irq_acknowledgement_match_reference() {
    let mut a = machine(MachineProfile::Ss5, 1, true);
    let mut b = machine(MachineProfile::Ss5, 1, false);
    for m in [&mut a, &mut b] {
        let l = m.bus.layout();
        m.bus.write8(l.esp + 12, 0).unwrap();
        m.bus.write8(l.esp + 32, 7).unwrap();
        m.bus.write8(l.esp + 20, 1).unwrap();
        m.bus.write8(l.esp + 16, 3).unwrap();
        m.bus.write8(l.esp + 8, 0x80).unwrap();
        m.bus.write8(l.esp + 12, 0x42).unwrap();
    }
    step_pair(&mut a, &mut b, false); // enter timed missing-target selection
    same(&a, &b, "timeout started");
    for _ in 0..5000 {
        step_pair(&mut a, &mut b, false);
    }
    same(&a, &b, "timeout completed");
    for m in [&mut a, &mut b] {
        let l = m.bus.layout();
        assert!(m.bus.devices().unwrap().esp.irq());
        assert_eq!(m.bus.read8(l.esp + 20).unwrap(), 0x20);
        assert!(!m.bus.devices().unwrap().esp.irq());
    }
    step_pair(&mut a, &mut b, false);
    same(&a, &b, "timeout IRQ acknowledged");
}

fn iommu(m: &mut Machine) {
    let io = m.bus.layout().io_control;
    m.bus.write32(io, 1).unwrap();
    m.bus.write32(io + 4, 0x1000).unwrap();
    grant_dma(m, true);
    m.bus.write32(0x10000, 0x806).unwrap(); // ff000000 -> 8000, writable
    m.bus.write32(0x10004, 0x902).unwrap(); // ff001000 -> 9000, read-only
}

fn grant_dma(m: &mut Machine, granted: bool) {
    let l = m.bus.layout();
    let (offset, bit) = if l.ncr_windows {
        (0x2000, 21)
    } else {
        (0x1008, 20)
    };
    let address = l.io_control + offset;
    let old = m.bus.read32(address).unwrap();
    m.bus
        .write32(
            address,
            if granted {
                old | (1 << bit)
            } else {
                old & !(1 << bit)
            },
        )
        .unwrap();
}

fn csr(m: &mut Machine, reg: u16, value: u16) {
    let mac = m.bus.layout().ethernet;
    m.bus.write16(mac + 2, reg).unwrap();
    m.bus.write16(mac, value).unwrap();
}

fn ethernet_init(m: &mut Machine, mode: u16) {
    iommu(m);
    // Independent LANCE init block: one descriptor per ring; MAC 02:00:00:00:00:01.
    let words = [mode, 0x0002, 0, 0x0100, 0, 0, 0, 0, 0x100, 0, 0x200, 0];
    for (i, word) in words.into_iter().enumerate() {
        m.bus.write16(0x8000 + i as u64 * 2, word).unwrap();
    }
    for (reg, val) in [(1, 0), (2, 0), (3, 4), (0, 0x41)] {
        csr(m, reg, val);
    }
    let dma = m.bus.layout().ethernet_dma();
    m.bus.write32(dma, 0x10).unwrap();
}

fn descriptor(m: &mut Machine, address: u64, words: [u16; 4]) {
    for (i, word) in words.into_iter().enumerate() {
        m.bus.write16(address + i as u64 * 2, word).unwrap();
    }
}

#[test]
fn ethernet_rx_filtering_dma_and_tx_poll_deadline_match_reference() {
    for (profile, cpus) in [
        (MachineProfile::Ss5, 1),
        (MachineProfile::Ss20, 2),
        (MachineProfile::Axil320, 2),
    ] {
        let mut a = machine(profile, cpus, true);
        let mut b = machine(profile, cpus, false);
        for m in [&mut a, &mut b] {
            ethernet_init(m, 0);
        }
        let scheduled = cpus == 2;
        for _ in 0..24 {
            step_pair(&mut a, &mut b, scheduled);
        }
        for m in [&mut a, &mut b] {
            assert_eq!(
                m.bus.devices().unwrap().ethernet.mac_address(),
                Some([2, 0, 0, 0, 0, 1])
            );
            csr(m, 0, 0x142);
        }
        for _ in 0..8 {
            step_pair(&mut a, &mut b, scheduled);
        }
        same(&a, &b, "Ethernet initialized and idle poll scheduled");
        let mut frame = vec![0x5a; 60];
        frame[..6].fill(0xff);
        for m in [&mut a, &mut b] {
            descriptor(m, 0x8100, [0x400, 0x8000, 0xff00, 0]);
            let rejected = vec![0x02; 60];
            assert!(m
                .bus
                .devices_mut()
                .unwrap()
                .ethernet
                .receive_frame(&rejected));
            assert!(m.bus.devices_mut().unwrap().ethernet.receive_frame(&frame));
        }
        for _ in 0..150 {
            step_pair(&mut a, &mut b, scheduled);
        }
        for m in [&mut a, &mut b] {
            assert_eq!(m.bus.read16(0x8102).unwrap() & 0x8300, 0x0300);
            assert_eq!(m.bus.read16(0x8106).unwrap(), 64);
            let data: Vec<_> = (0..60).map(|i| m.bus.read8(0x8400 + i).unwrap()).collect();
            assert_eq!(data, frame);
            descriptor(m, 0x8200, [0x300, 0x8300, 0xffc4, 0]);
            m.load_image(0x8300, &frame).unwrap();
        }
        same(&a, &b, "RX filtering and payload");
        for m in [&mut a, &mut b] {
            m.advance_ns(1_600_000);
        }
        for _ in 0..150 {
            step_pair(&mut a, &mut b, scheduled);
        }
        for m in [&mut a, &mut b] {
            assert_eq!(
                m.bus.devices_mut().unwrap().ethernet.take_frame(),
                Some(frame.clone())
            );
            assert_eq!(m.bus.read16(0x8202).unwrap() & 0x8000, 0);
        }
        same(&a, &b, "TX poll deadline and payload");
    }
}

struct Image(std::path::PathBuf);
impl Image {
    fn new() -> Self {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "helios-device-service-inquiry-scsi3-{}.img",
            std::process::id()
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap()
            .write_all(&[0; 512])
            .unwrap();
        Self(path)
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

#[test]
fn scsi_dma_denial_fault_and_completion_match_reference() {
    let image = Image::new();
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        let mut a = machine(profile, 1, true);
        let mut b = machine(profile, 1, false);
        for m in [&mut a, &mut b] {
            let l = m.bus.layout();
            iommu(m);
            m.bus
                .devices_mut()
                .unwrap()
                .scsi
                .attach(3, helios::devices::scsi::MediaKind::Disk, &image.0)
                .unwrap();
            m.bus.write8(l.esp + 12, 0).unwrap();
            m.bus.write8(l.esp + 32, 7).unwrap();
            m.bus.write8(l.esp + 16, 3).unwrap();
            for byte in [0x80, 0x12, 0, 0, 0, 36, 0] {
                m.bus.write8(l.esp + 8, byte).unwrap();
            }
            m.bus.write8(l.esp + 12, 0x42).unwrap();
        }
        for _ in 0..8 {
            step_pair(&mut a, &mut b, false);
        }
        same(&a, &b, "PIO CDB and execute");
        for m in [&mut a, &mut b] {
            let l = m.bus.layout();
            assert_eq!(m.bus.read8(l.esp + 20).unwrap(), 0x18);
            m.bus.write32(l.scsi_dma, 0x2320).unwrap();
            m.bus.write32(l.scsi_dma + 8, 36).unwrap();
            m.bus.write32(l.scsi_dma + 4, 0xff00_1000).unwrap();
            m.bus.write8(l.esp, 36).unwrap();
            m.bus.write8(l.esp + 4, 0).unwrap();
            grant_dma(m, false);
            m.bus.write8(l.esp + 12, 0x90).unwrap();
        }
        for _ in 0..8 {
            step_pair(&mut a, &mut b, false);
        }
        same(&a, &b, "DMA request retained during arbitration denial");
        for m in [&mut a, &mut b] {
            let l = m.bus.layout();
            assert_ne!(m.bus.read32(l.scsi_dma).unwrap() & (1 << 10), 0);
            assert_eq!(m.bus.read32(l.scsi_dma + 8).unwrap(), 36);
            grant_dma(m, true);
        }
        step_pair(&mut a, &mut b, false);
        same(&a, &b, "write-protected DMA target");
        for m in [&mut a, &mut b] {
            let l = m.bus.layout();
            assert_ne!(m.bus.read32(l.scsi_dma).unwrap() & 2, 0);
            assert_eq!(m.bus.read32(l.scsi_dma + 8).unwrap(), 36);
            m.bus.write32(l.scsi_dma, 0x2320).unwrap();
            m.bus.write32(l.scsi_dma + 4, 0xff00_0100).unwrap();
        }
        for _ in 0..4 {
            step_pair(&mut a, &mut b, false);
        }
        for m in [&mut a, &mut b] {
            let l = m.bus.layout();
            assert_eq!(m.bus.read32(l.scsi_dma + 8).unwrap(), 0);
            assert_eq!(m.bus.read32(l.scsi_dma + 4).unwrap(), 0xff00_0124);
            assert_eq!(m.bus.read8(l.esp + 20).unwrap(), 0x10);
            let vendor: Vec<_> = (0..8).map(|i| m.bus.read8(0x8108 + i).unwrap()).collect();
            assert_eq!(vendor, b"HELIOS  ");
        }
        same(&a, &b, "DMA completion and IRQ acknowledgement");
    }
}
