use helios::bus::{Bus, BusErrorKind, MapError};
use helios::devices::{ethernet::Ethernet, ethernet_dma::EthernetDma, nvram::Nvram};
use helios::machine::Machine;
use helios::system_bus::{
    SystemBus, ETHERNET_BASE as MAC, ETHERNET_DMA_BASE as DMA, IO_CONTROL_BASE as IO,
    SYSTEM_INTERRUPT_BASE as IRQ,
};

fn csr(m: &mut Machine, reg: u16, value: u16) {
    m.bus.write16(MAC + 2, reg).unwrap();
    m.bus.write16(MAC, value).unwrap();
}
fn status(m: &mut Machine) -> u16 {
    m.bus.write16(MAC + 2, 0).unwrap();
    m.bus.read16(MAC).unwrap()
}
fn pump(m: &mut Machine, count: usize) {
    for _ in 0..count {
        m.service_ethernet().unwrap();
    }
}
fn fixture(swap: bool) -> Machine {
    let mut m = Machine::sun4m(0x20000, &0x3080_0000u32.to_be_bytes()).unwrap();
    m.bus.write32(IO, 1).unwrap();
    m.bus.write32(IO + 4, 0x1000).unwrap();
    m.bus.write32(IO + 0x2000, 1 << 21).unwrap(); // NCR89C100 master 5
    m.bus.write32(0x10000, 0x806).unwrap(); // ff000000 -> PA 8000
                                            // Independently encoded init block: MAC 02:00:00:00:00:01,
                                            // two-entry RX ring ff000100, two-entry TX ring ff000200.
    for (i, word) in [
        0, 0x0002, 0, 0x0100, 0, 0, 0, 0, 0x100, 0x2000, 0x200, 0x2000,
    ]
    .into_iter()
    .enumerate()
    {
        m.bus.write16(0x8000 + i as u64 * 2, word).unwrap();
    }
    csr(&mut m, 1, 0);
    csr(&mut m, 2, 0);
    csr(&mut m, 3, if swap { 4 } else { 0 });
    m.bus.write32(DMA, 0x10).unwrap();
    m.bus.write32(IRQ + 8, (1 << 31) | (1 << 16)).unwrap();
    csr(&mut m, 0, 0x41);
    m
}
fn start(m: &mut Machine) {
    pump(m, 12);
    assert_eq!(status(m), 0x1c1);
    csr(m, 0, 0x142); // acknowledge IDON, enable IRQ and start
}
fn descriptor(m: &mut Machine, address: u64, words: [u16; 4]) {
    for (i, word) in words.into_iter().enumerate() {
        m.bus.write16(address + i as u64 * 2, word).unwrap();
    }
}
fn frame() -> Vec<u8> {
    let mut frame: Vec<u8> = (0..60).collect();
    frame[..6].fill(255);
    frame
}

#[test]
fn dma_identity_reset_base_lanes_and_control_masks() {
    let mut d = EthernetDma::default();
    assert_eq!(d.read(0).unwrap(), 0xa000_0000);
    assert_eq!(d.read(12).unwrap(), 0xff00_0000);
    d.write(12, 0xf012_3456).unwrap();
    assert_eq!(d.read(12).unwrap(), 0xf000_0000);
    d.write(0, 0x0077_9810).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa077_9810);
    let before = d.clone();
    for (offset, value) in [(0, 0x80000), (0, 0xc0000), (4, 0), (8, 0)] {
        assert!(d.write(offset, value).unwrap_err().is_host_stop());
        assert_eq!(d, before);
    }
    d.write(0, 0x80).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa000_0080);
    d.write(12, 0).unwrap();
    assert_eq!(d.read(12).unwrap(), 0xff00_0000);
    d.write(0, 0).unwrap();
    d.write(0, 0xff80_676f).unwrap(); // read-only/reserved bits plus empty-cache pulses
    assert_eq!(d.read(0).unwrap(), 0xa000_0000);
}

#[test]
fn slave_width_errors_and_reset_are_shared_by_dma_and_mac() {
    let mut b = SystemBus::default();
    b.install_sun4m(Nvram::default()).unwrap();
    b.write16(MAC + 2, 1).unwrap();
    b.write16(MAC, 0x1234).unwrap();
    for error in [
        b.write8(DMA, 0).unwrap_err(),
        b.read16(DMA).unwrap_err(),
        b.write8(MAC + 2, 0).unwrap_err(),
        b.read32(MAC).unwrap_err(),
        b.read_doubleword(DMA).unwrap_err(),
    ] {
        assert_eq!(error.kind, BusErrorKind::SBusError);
    }
    assert_eq!(b.read32(DMA).unwrap(), 0xa000_0040);
    assert_eq!(b.read16(MAC + 2).unwrap(), 1);
    assert_eq!(b.read16(MAC).unwrap(), 0x1234);
    b.write32(DMA, 0x40).unwrap();
    for base in [DMA, MAC] {
        assert_eq!(b.map_ram(base, 4), Err(MapError::Overlap));
        assert!(b.swap32(base, 0).unwrap_err().kind.is_host_stop());
        assert!(b.ldstub8(base).unwrap_err().kind.is_host_stop());
        assert!(b
            .write32_masked(base, 0, 0xffff)
            .unwrap_err()
            .kind
            .is_host_stop());
        assert!(b.load_image(base, &[0; 4]).unwrap_err().kind.is_host_stop());
    }
    assert_eq!(b.read32(DMA).unwrap(), 0xa000_0000);
    b.write32(DMA, 0x80).unwrap();
    b.write16(MAC + 2, 2).unwrap();
    b.write16(MAC, 0xffff).unwrap();
    assert_eq!(b.read16(MAC).unwrap(), 0);
    b.write32(DMA, 0).unwrap();
    assert_eq!(b.read16(MAC + 2).unwrap(), 0);
    assert_eq!(b.read16(MAC).unwrap(), 4);
    assert_eq!(b.read32(DMA).unwrap(), 0xa000_0000);
}

#[test]
fn mac_register_masks_stop_and_ncr_ipg_fields() {
    let mut e = Ethernet::default();
    assert_eq!(e.read(0).unwrap(), 4);
    e.write(0, 0xfff8).unwrap();
    assert_eq!(e.read(0).unwrap(), 4); // read-only status and IEN while stopped
    for (reg, written, expected) in [(1, 0xffff, 0xfffe), (2, 0xffff, 0xff), (3, 0xffff, 0xfe07)] {
        e.write(2, reg).unwrap();
        e.write(0, written).unwrap();
        assert_eq!(e.read(0).unwrap(), expected);
    }
    e.write(0, 0x7fff).unwrap();
    assert_eq!(e.read(0).unwrap(), 7);
    e.write(2, 0).unwrap();
    e.write(0, 4).unwrap();
    e.write(2, 1).unwrap();
    assert_eq!(e.read(0).unwrap(), 0xfffe);
    e.write(2, 3).unwrap();
    assert_eq!(e.read(0).unwrap(), 0);
    e.reset();
    assert_eq!(e.read(0).unwrap(), 4);
}

#[test]
fn init_requires_real_iommu_reads_and_irq_acknowledgement_is_level_sensitive() {
    let mut m = fixture(true);
    let cpu = m.cpu.clone();
    let timers = m.bus.devices().unwrap().timers.clone();
    pump(&mut m, 11);
    assert_eq!(status(&mut m), 0x41);
    assert_eq!(m.bus.interrupt_levels(), 0);
    pump(&mut m, 1);
    assert_eq!(status(&mut m), 0x1c1);
    assert_eq!(
        m.bus.devices().unwrap().ethernet.mac_address(),
        Some([2, 0, 0, 0, 0, 1])
    );
    assert_eq!(m.bus.interrupt_levels(), 1 << 6);
    assert_eq!(m.bus.read32(DMA).unwrap(), 0xa000_0011);
    m.bus.write32(DMA, 0x30).unwrap(); // invalidate does not ack MAC
    assert_eq!(m.bus.interrupt_levels(), 1 << 6);
    csr(&mut m, 0, 0x100);
    assert_eq!(status(&mut m), 1);
    assert_eq!(m.bus.interrupt_levels(), 0);
    assert_eq!(
        (m.cpu.pc(), m.cpu.npc(), m.cpu.psr()),
        (cpu.pc(), cpu.npc(), cpu.psr())
    );
    assert_eq!(m.bus.devices().unwrap().timers, timers);
}

#[test]
fn arbiter_denial_retries_and_invalid_mapping_reports_device_memory_error() {
    let mut m = fixture(true);
    m.bus.write32(IO + 0x2000, 0).unwrap();
    pump(&mut m, 100);
    assert_eq!(status(&mut m), 0x41);
    m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
    m.bus.write32(0x10000, 0).unwrap();
    pump(&mut m, 1);
    assert_eq!(status(&mut m), 0x88c1);
    assert_eq!(m.bus.read32(DMA).unwrap(), 0xa000_0013);
    assert_eq!(m.bus.interrupt_levels(), 1 << 6);
    csr(&mut m, 0, 0x840);
    assert_eq!(m.bus.read32(DMA).unwrap(), 0xa000_0012);
    m.bus.write32(DMA, 0x30).unwrap();
    assert_eq!(m.bus.interrupt_levels(), 0);
}

#[test]
fn unsupported_dma_target_stops_before_cpu_or_timer_advance_and_can_be_retried() {
    let mut m = fixture(true);
    m.bus.write32(0x10000, 0x0100_0006).unwrap(); // maps to non-DRAM PA 10000000
    let before = m.bus.devices().unwrap().ethernet.clone();
    let timers = m.bus.devices().unwrap().timers.clone();
    for _ in 0..2 {
        let error = m.step().unwrap_err();
        assert!(matches!(error.kind, helios::cpu::FaultKind::DeviceDma(_)));
        assert_eq!(m.cpu.pc(), 0);
        assert_eq!(m.bus.devices().unwrap().ethernet, before);
        assert_eq!(m.bus.devices().unwrap().timers, timers);
    }
}

#[test]
fn transmit_packet_byte_lanes_and_ring_wrap_preserve_metadata_in_both_swap_modes() {
    for swap in [false, true] {
        let mut m = fixture(swap);
        start(&mut m);
        let packet = frame();
        for (i, byte) in packet.iter().enumerate() {
            m.bus
                .write8((0x8400 + i as u64) ^ u64::from(!swap), *byte)
                .unwrap();
        }
        for n in 0..3 {
            let desc = 0x8200 + (n % 2) * 8;
            descriptor(&mut m, desc, [0x400, 0x8300, 0xffc4, 0]);
            csr(&mut m, 0, 0x248);
            pump(&mut m, 64); // descriptor + payload; ownership still belongs to MAC
            assert_eq!(m.bus.read16(desc + 2).unwrap(), 0x8300);
            assert!(m.bus.devices_mut().unwrap().ethernet.take_frame().is_none());
            pump(&mut m, 1);
            assert_eq!(m.bus.read16(desc + 2).unwrap(), 0x0300);
            assert_eq!(
                m.bus.devices_mut().unwrap().ethernet.take_frame(),
                Some(packet.clone())
            );
            assert_eq!(status(&mut m) & 0x280, 0x280);
        }
    }
}

#[test]
fn receive_writes_payload_and_fcs_before_count_and_ownership() {
    for swap in [false, true] {
        let mut m = fixture(swap);
        start(&mut m);
        descriptor(&mut m, 0x8100, [0x600, 0x8000, 0xff80, 0]);
        let packet = frame();
        assert!(m.bus.devices_mut().unwrap().ethernet.receive_frame(&packet));
        pump(&mut m, 68); // four descriptor words, then 64 packet bytes including FCS
        assert_eq!(m.bus.read16(0x8102).unwrap(), 0x8000);
        assert_eq!(m.bus.read16(0x8106).unwrap(), 0);
        pump(&mut m, 1);
        assert_eq!(m.bus.read16(0x8106).unwrap(), 64);
        assert_eq!(m.bus.read16(0x8102).unwrap(), 0x8000);
        pump(&mut m, 1);
        assert_eq!(m.bus.read16(0x8102).unwrap(), 0x300);
        for (i, byte) in packet.iter().enumerate() {
            assert_eq!(
                m.bus.read8((0x8600 + i as u64) ^ u64::from(!swap)).unwrap(),
                *byte
            );
        }
        let fcs: Vec<_> = (60..64)
            .map(|i| m.bus.read8((0x8600 + i) ^ u64::from(!swap)).unwrap())
            .collect();
        // Independent CRC-32/ISO-HDLC fixture, computed using zlib.
        assert_eq!(fcs, [142, 62, 90, 255]);
        assert_eq!(status(&mut m) & 0x480, 0x480);
        assert_eq!(m.bus.interrupt_levels(), 1 << 6);
    }
}

#[test]
fn filtered_packets_do_not_raise_miss_but_matching_packets_without_buffers_do() {
    let mut m = fixture(true);
    start(&mut m);
    let mut packet = frame();
    packet[..6].copy_from_slice(&[2, 0, 0, 0, 0, 2]);
    assert!(m.bus.devices_mut().unwrap().ethernet.receive_frame(&packet));
    pump(&mut m, 10);
    assert_eq!(status(&mut m) & 0x7f80, 0);
    packet[..6].fill(255);
    assert!(m.bus.devices_mut().unwrap().ethernet.receive_frame(&packet));
    pump(&mut m, 4);
    assert_eq!(status(&mut m) & 0x9080, 0x9080);
}

#[test]
fn unsupported_descriptor_chaining_does_not_release_ownership_or_emit_a_frame() {
    let mut m = fixture(true);
    start(&mut m);
    descriptor(&mut m, 0x8200, [0x400, 0x8200, 0xffc4, 0]);
    pump(&mut m, 3);
    let before = m.bus.devices().unwrap().ethernet.clone();
    for _ in 0..2 {
        assert!(m.service_ethernet().is_err());
        assert_eq!(m.bus.devices().unwrap().ethernet, before);
        assert_eq!(m.bus.read16(0x8202).unwrap(), 0x8200);
    }
    assert!(m.bus.devices_mut().unwrap().ethernet.take_frame().is_none());
}

#[test]
fn internal_loopback_moves_runt_packet_through_real_tx_and_rx_descriptors() {
    let mut m = fixture(true);
    m.bus.write16(0x8000, 0x44).unwrap(); // LBK + ILB, transmit CRC enabled
    start(&mut m);
    descriptor(&mut m, 0x8200, [0x400, 0x8300, 0xfff0, 0]);
    descriptor(&mut m, 0x8100, [0x600, 0x8000, 0xffc0, 0]);
    let packet = [
        2, 0, 0, 0, 0, 1, 8, 0, 0x20, 0xc0, 0xff, 0xee, 0xaa, 0xbb, 0xcc, 0xdd,
    ];
    m.bus.load_image(0x8400, &packet).unwrap();
    // External broadcasts cannot enter the internal loopback path.
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .ethernet
        .receive_frame(&frame()));
    pump(&mut m, 100);
    assert_eq!(m.bus.read16(0x8202).unwrap(), 0x300);
    assert_eq!(m.bus.read16(0x8102).unwrap(), 0x300);
    assert_eq!(m.bus.read16(0x8106).unwrap(), 20);
    for (i, byte) in packet.iter().enumerate() {
        assert_eq!(m.bus.read8(0x8600 + i as u64).unwrap(), *byte);
    }
    assert_eq!(status(&mut m) & 0x680, 0x680);
    assert!(m.bus.devices_mut().unwrap().ethernet.take_frame().is_none());
}

#[test]
fn dma_error_during_receive_leaves_ownership_and_already_written_bytes_intact() {
    let mut m = fixture(true);
    start(&mut m);
    // First four bytes writable at the end of the mapped page; the next
    // page's IOPTE is invalid. No descriptor completion may be fabricated.
    descriptor(&mut m, 0x8100, [0xffc, 0x8000, 0xff80, 0]);
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .ethernet
        .receive_frame(&frame()));
    pump(&mut m, 20);
    assert_eq!(m.bus.read32(0x8ffc).unwrap(), 0xffff_ffff);
    assert_eq!(m.bus.read32(0x9000).unwrap(), 0);
    assert_eq!(m.bus.read16(0x8102).unwrap(), 0x8000);
    assert_eq!(m.bus.read16(0x8106).unwrap(), 0);
    assert_eq!(status(&mut m) & 0x8830, 0x8800);
    assert_eq!(m.bus.read32(DMA).unwrap() & 3, 3);
}

#[test]
fn shifted_dma_base_uses_the_programmed_high_address_byte() {
    let mut m = fixture(true);
    m.bus.write32(DMA + 12, 0xfe00_0000).unwrap();
    // The existing ff000000 mapping is valid, but fe000000 is out of the
    // configured 16 MiB DVMA range. The MAC must report MERR, never IDON.
    pump(&mut m, 12);
    assert_eq!(status(&mut m) & 0x8900, 0x8800);
    assert_eq!(m.bus.devices().unwrap().ethernet.mac_address(), None);
}
