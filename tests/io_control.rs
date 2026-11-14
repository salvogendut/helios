use helios::bus::{Bus, BusErrorKind, Initiator, MapError};
use helios::cpu::{FaultKind, Step};
use helios::devices::io_control::IoControl;
use helios::devices::nvram::Nvram;
use helios::machine::Machine;
use helios::mmu::{FaultType, MemoryError, Srmmu, Width, PCR_BM, PCR_EN, PCR_NF};
use helios::system_bus::{SystemBus, IO_CONTROL_BASE, RESET_BASE};
use helios::topology::ProcessorModule;

#[test]
fn io_control_identity_range_and_table_base_follow_microsparc_fields() {
    let mut io = IoControl::default();
    assert_eq!(io.read(0).unwrap(), 0x0400_0000); // IMPL=0, VER=4, translation disabled
    assert_eq!(io.read(4).unwrap(), 0);
    for range in 0..8 {
        io.write(0, 0xffff_ffe2 | range << 2).unwrap(); // reserved/version writes ignored
        assert_eq!(io.read(0).unwrap(), 0x0400_0000 | range << 2);
    }
    for (written, expected) in [
        (u32::MAX, 0x07ff_fc00),
        (0xf800_03ff, 0),
        (0x0123_4567, 0x0123_4400),
    ] {
        io.write(4, written).unwrap();
        assert_eq!(io.read(4).unwrap(), expected);
    }
    io.write(0, u32::MAX).unwrap();
    assert_eq!(io.read(0).unwrap(), 0x0400_001d);
    io.write(0, 0).unwrap();
    assert_eq!(io.read(0).unwrap(), 0x0400_0000);
}

#[test]
fn arbitration_has_six_independent_bits_and_an_immutable_mid() {
    let mut io = IoControl::default();
    assert_eq!(io.read(0x2000).unwrap(), 8);
    for master in 0..6 {
        io.write(0x2000, 1 << (16 + master)).unwrap();
        assert_eq!(io.read(0x2000).unwrap(), 8 | 1 << (16 + master));
    }
    // The sixth master at bit 21 is SCSI/Ethernet, not a reserved bit.
    io.write(0x2000, u32::MAX).unwrap();
    assert_eq!(io.read(0x2000).unwrap(), 0x003f_0008);
    io.write(0x2000, 0xffc0_ffff).unwrap();
    assert_eq!(io.read(0x2000).unwrap(), 8);
}

#[test]
fn five_slot_registers_are_independent_and_shared_slot_four_cannot_bypass() {
    let mut io = IoControl::default();
    for slot in 0..5u64 {
        let offset = 0x1010 + slot * 4;
        assert_eq!(io.read(offset).unwrap(), 0);
        io.write(offset, u32::MAX).unwrap();
        assert_eq!(
            io.read(offset).unwrap(),
            if slot == 4 { 2 } else { 0x0001_0003 }
        );
        for later in slot + 1..5 {
            assert_eq!(io.read(0x1010 + later * 4).unwrap(), 0);
        }
    }
    for (offset, value) in [
        (0x1010, 1),
        (0x1014, 2),
        (0x1018, 0x10000),
        (0x101c, 0),
        (0x1020, 0x10001),
    ] {
        io.write(offset, value).unwrap();
    }
    assert_eq!(io.read(0x1010).unwrap(), 1);
    assert_eq!(io.read(0x1014).unwrap(), 2);
    assert_eq!(io.read(0x1018).unwrap(), 0x10000);
    assert_eq!(io.read(0x101c).unwrap(), 0);
    assert_eq!(io.read(0x1020).unwrap(), 0);
}

#[test]
fn unimplemented_facilities_stop_without_fabricating_success_or_changing_configuration() {
    let mut io = IoControl::default();
    io.write(0x2000, 0x0020_0000).unwrap();
    io.write(0, 0x1c).unwrap();
    let before = io.clone();
    for offset in [
        0x3000, 0x3004, 0x3008, 0x300c, 0x3010, 0x3014, 0x4000, 0x6000, 0x7000,
    ] {
        assert!(io.read(offset).unwrap_err().is_host_stop(), "{offset:#x}");
        assert!(
            io.write(offset, 0).unwrap_err().is_host_stop(),
            "{offset:#x}"
        );
        assert_eq!(io, before);
    }
    // No generic register RAM; the SS-20/Axil MBus arbiter-enable register is
    // implemented separately from the microSPARC-II configuration.
    for offset in [8, 0x1024, 0x2004, 0x3020, 0x7ffc] {
        assert_eq!(io.read(offset), Err(BusErrorKind::Unmapped));
        assert_eq!(io.write(offset, 123), Err(BusErrorKind::Unmapped));
        assert_eq!(io, before);
    }
    io.write(0x1008, 0x003f_0000).unwrap();
    assert_eq!(io.read(0x1008).unwrap(), 0x001f_0001);
}

#[test]
fn physical_aperture_rejects_wrong_widths_atomics_and_overlapping_mappings() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    bus.write32(IO_CONTROL_BASE + 0x2000, 0x003f_0000).unwrap();
    let before = bus.devices().unwrap().io_control.clone();
    for error in [
        bus.read8(IO_CONTROL_BASE).unwrap_err(),
        bus.read16(IO_CONTROL_BASE).unwrap_err(),
        bus.read_doubleword(IO_CONTROL_BASE).unwrap_err(),
        bus.write8(IO_CONTROL_BASE + 3, 1).unwrap_err(),
        bus.write16(IO_CONTROL_BASE + 2, 1).unwrap_err(),
        bus.write_doubleword(IO_CONTROL_BASE, [0, 0]).unwrap_err(),
        bus.write32_masked(IO_CONTROL_BASE + 0x2000, 0, 0x00ff_0000)
            .unwrap_err(),
        bus.swap32(IO_CONTROL_BASE + 0x2000, 0).unwrap_err(),
        bus.ldstub8(IO_CONTROL_BASE + 0x2000).unwrap_err(),
        bus.load_image(IO_CONTROL_BASE, &[0; 8]).unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop());
    }
    assert_eq!(bus.devices().unwrap().io_control, before);
    assert_eq!(
        bus.read32(IO_CONTROL_BASE + 1).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(
        bus.read32(IO_CONTROL_BASE + 0x8000).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    for address in [
        IO_CONTROL_BASE,
        IO_CONTROL_BASE + 0x2000,
        IO_CONTROL_BASE + 0x7ffc,
    ] {
        assert_eq!(bus.map_ram(address, 4), Err(MapError::Overlap));
        assert_eq!(bus.map_rom(address, &[0; 4]), Err(MapError::Overlap));
        let mut existing = SystemBus::default();
        existing.map_ram(address, 4).unwrap();
        assert_eq!(
            existing.install_sun4m(Nvram::default()),
            Err(MapError::Overlap)
        );
    }
}

#[test]
fn reset_prom_programs_sbus_controls_through_physical_bypass_and_reset_clears_them() {
    let words = [
        0xc0a0_4400u32, // sta g0, [g1] ASI20: disable arbitration
        0xc480_4400,    // lda [g1] ASI20, g2
        0xc6a0_4400,    // sta g3, [g1] ASI20: enable all masters
        0xc880_4400,    // lda [g1] ASI20, g4
        0xc6a1_4400,    // sta g3, [g5] ASI20: shared slot config
        0xcc81_4400,    // lda [g5] ASI20, g6
        0xd0a1_c400,    // sta o0, [g7] ASI20: range/reserved bits, EN=0
        0xd281_c400,    // lda [g7] ASI20, o1
    ];
    let prom: Vec<u8> = words.into_iter().flat_map(u32::to_be_bytes).collect();
    for software in [false, true] {
        let mut m = Machine::sun4m(8192, &prom).unwrap();
        m.cpu.write_reg(1, (IO_CONTROL_BASE + 0x2000) as u32);
        m.cpu.write_reg(3, u32::MAX);
        m.cpu.write_reg(5, (IO_CONTROL_BASE + 0x1020) as u32);
        m.cpu.write_reg(7, IO_CONTROL_BASE as u32);
        m.cpu.write_reg(8, !1);
        for _ in 0..8 {
            assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
        }
        assert_eq!(m.cpu.read_reg(2), 8);
        assert_eq!(m.cpu.read_reg(4), 0x003f_0008);
        assert_eq!(m.cpu.read_reg(6), 2);
        assert_eq!(m.cpu.read_reg(9), 0x0400_001c);
        m.bus.write32(IO_CONTROL_BASE + 4, 0x123_4400).unwrap();
        m.bus.write32(0x100, 0xabcd_1234).unwrap();
        if software {
            m.bus.write32(RESET_BASE, 1).unwrap();
            assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
        } else {
            m.reset();
        }
        assert_eq!(m.bus.devices().unwrap().io_control, IoControl::default());
        assert_eq!(m.bus.read32(IO_CONTROL_BASE).unwrap(), 0x0400_0000);
        assert_eq!(m.bus.read32(IO_CONTROL_BASE + 0x2000).unwrap(), 8);
        assert_eq!(m.bus.read32(0x100).unwrap(), 0xabcd_1234);
    }
}

#[test]
fn unsupported_performance_register_is_retryable_even_with_cpu_no_fault_mode() {
    let mut m = Machine::sun4m(8192, &0xc4a0_4400u32.to_be_bytes()).unwrap();
    m.cpu.write_reg(1, (IO_CONTROL_BASE + 0x3000) as u32);
    m.cpu.write_reg(2, 1);
    m.bus.write32(IO_CONTROL_BASE, 0x1c).unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_BM | PCR_NF)
        .unwrap();
    let before = m.bus.devices().unwrap().io_control.clone();
    let timers = m.bus.devices().unwrap().timers.clone();
    let fault = m.step().unwrap_err();
    assert!(
        matches!(fault.kind, FaultKind::UnsupportedMemory(MemoryError::Bus(error)) if error.kind.is_host_stop())
    );
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0, 4));
    assert!(m.cpu.error_state().is_none());
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert_eq!(m.cpu.mmu.fault_address(), 0);
    assert_eq!(m.bus.devices().unwrap().io_control, before);
    assert_eq!(m.bus.devices().unwrap().timers, timers);
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0); // no watchdog event
    m.cpu.write_reg(1, IO_CONTROL_BASE as u32); // retry at a supported register
    m.cpu.write_reg(2, 0x11);
    assert!(matches!(m.step().unwrap(), Step::Executed { pc: 0, .. }));
    assert_eq!(m.bus.read32(IO_CONTROL_BASE).unwrap(), 0x0400_0011);
}

#[test]
fn translated_read_only_control_mapping_does_not_allow_arbitration_writes() {
    let mut m = Machine::sun4m(0x5000, &0x0100_0000u32.to_be_bytes()).unwrap();
    for (address, word) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4008, 0x0100_0202),
    ] {
        // VA 0x2000 -> PA 0x10002000, ACC=0 read-only
        m.bus.write32(address, word).unwrap();
    }
    let mut mmu = Srmmu::default();
    mmu.store(&mut m.bus, 0x100, 4, Width::Word, 0x100).unwrap();
    mmu.store(&mut m.bus, 0, 4, Width::Word, PCR_EN).unwrap();
    assert_eq!(mmu.load(&mut m.bus, 0x2000, 10, Width::Word).unwrap(), 8);
    assert!(
        matches!(mmu.store(&mut m.bus, 0x2000, 10, Width::Word, u32::MAX),
        Err(MemoryError::Translation(fault)) if fault.kind == FaultType::Protection)
    );
    assert_eq!(m.bus.read32(IO_CONTROL_BASE + 0x2000).unwrap(), 8);
}

#[test]
fn chip_revision_is_immutable_and_address_compare_fields_replace_and_reset() {
    let mut io = IoControl::default();
    assert_eq!(io.read(0x3018).unwrap(), 0x3000_0000);
    assert_eq!(io.read(0x301c).unwrap(), 0);
    io.write(0x3018, u32::MAX).unwrap();
    io.write(0x301c, 0x8765_4321).unwrap();
    assert_eq!(io.read(0x3018).unwrap(), 0x3000_07ff);
    assert_eq!(io.read(0x301c).unwrap(), 0x8765_4321);
    // Each compare field can be cleared, including previously set bits.
    for bit in 0..11 {
        io.write(0x3018, 1 << bit).unwrap();
        assert_eq!(io.read(0x3018).unwrap(), 0x3000_0000 | 1 << bit);
    }
    io.write(0x3018, 0xffff_f800).unwrap();
    assert_eq!(io.read(0x3018).unwrap(), 0x3000_0000);
    // Configuring an address does not enable external scan/JTAG triggers;
    // trigger control remains an explicit unsupported facility.
    let before = io.clone();
    assert!(io.write(0x300c, 1).unwrap_err().is_host_stop());
    assert_eq!(io, before);
    io.reset();
    assert_eq!(io.read(0x3018).unwrap(), 0x3000_0000);
    assert_eq!(io.read(0x301c).unwrap(), 0);
}

#[test]
fn standalone_aer_and_mid_registers_follow_sun4m_and_the_initiator() {
    let mut io = IoControl::new(0x1300_0000);
    // Sun-4M 5.1.2 POR: bit 0 reads one, EN_P1/MID-9 set, MID-A/B clear.
    assert_eq!(io.read(0x1008).unwrap(), 0x0000_0003);
    let cpu0 = ProcessorModule::from_processor(0).unwrap();
    let cpu2 = ProcessorModule::from_processor(2).unwrap();
    assert_eq!(io.read_for(0x2000, Initiator::Cpu(cpu0)).unwrap(), 8);
    assert_eq!(io.read_for(0x2000, Initiator::Cpu(cpu2)).unwrap(), 0xa);
    assert_eq!(
        io.read_for(0x2000, Initiator::Dma { master: 5 }).unwrap(),
        0
    );
    assert_eq!(io.read_for(0x2000, Initiator::Host).unwrap(), 0);
    // MID is read-only; writes must not disturb the AER SBus/processor bits.
    io.write(0x2000, u32::MAX).unwrap();
    assert_eq!(io.read_for(0x2000, Initiator::Cpu(cpu2)).unwrap(), 0xa);
    assert_eq!(io.read(0x1008).unwrap(), 0x0000_0003);

    // Only AER bits 0:3, 16:20 and 31 are defined; bit 0 reads one.
    io.write(0x1008, u32::MAX).unwrap();
    assert_eq!(io.read(0x1008).unwrap(), 0x801f_000f);
    io.write(0x1008, 0x7fff_fffe).unwrap();
    assert_eq!(io.read(0x1008).unwrap(), 0x001f_000f);
    // MID-9/MID-B are absent processor slots; enabling them leaves the AER
    // bits observable but creates no CPU in the board presence query.
    assert!(io.processor_arbitration_enabled(1));
    assert!(io.processor_arbitration_enabled(3));
    assert!(io.processor_arbitration_enabled(2));
    io.write(0x1008, 0).unwrap();
    assert_eq!(io.read(0x1008).unwrap(), 1);
    assert!(!io.processor_arbitration_enabled(2));
    assert!(io.set_processor_arbitration(2, true));
    assert!(io.processor_arbitration_enabled(2));
    assert!(io.set_processor_arbitration(2, false));
    assert!(!io.processor_arbitration_enabled(2));
    assert!(!io.set_processor_arbitration(0, false));
    assert!(!io.set_processor_arbitration(4, true));
}

#[test]
fn integrated_ss5_aer_keeps_its_reset_and_has_no_processor_eligibility() {
    let mut io = IoControl::default();
    assert_eq!(io.read(0x1008).unwrap(), 1);
    io.write(0x1008, u32::MAX).unwrap();
    assert_eq!(io.read(0x1008).unwrap(), 0x801f_000f);
    assert!(io.processor_arbitration_enabled(0));
    assert!(!io.processor_arbitration_enabled(1));
    assert!(!io.processor_arbitration_enabled(2));
    assert!(!io.set_processor_arbitration(2, true));
    // The integrated +0x2000 register is fixed MID 8 plus SBAE, never the
    // caller's MID, and writes replace only the six arbitration enables.
    assert_eq!(io.read(0x2000).unwrap(), 8);
    let cpu2 = ProcessorModule::from_processor(2).unwrap();
    assert_eq!(io.read_for(0x2000, Initiator::Cpu(cpu2)).unwrap(), 8);
    io.write(0x2000, u32::MAX).unwrap();
    assert_eq!(io.read(0x2000).unwrap(), 0x003f_0008);
}
