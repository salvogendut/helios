use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_PS, PSR_S};
use helios::devices::serial::Channel;
use helios::machine::{
    InstalledVideo, Machine, MachineProfile, ParkingError, SOLARIS_IDLE_NPC, SOLARIS_IDLE_PC,
    SOLARIS_IDLE_PSR,
};
use helios::mmu::{Width, PCR_EN};

const LOOP: [u8; 8] = [0x10, 0x80, 0, 0, 1, 0, 0, 0];

fn address_only_machine() -> Machine {
    let mut m = Machine::with_profile_processors_video(
        MachineProfile::Ss20,
        16 * 1024 * 1024,
        &LOOP,
        MachineProfile::Ss20.default_nvram(),
        1,
        InstalledVideo::None,
    )
    .unwrap();
    let words = [0x8188_6000u32, 0x80a0_0000u32]; // wr %g1,%psr; cmp %g0,%g0
    let code: Vec<_> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    m.load_image(0x1000, &code).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    m.cpu.write_reg(1, PSR_S | PSR_PS | PSR_ET);
    m.cpu.step(&mut m.bus).unwrap();
    m.cpu.step(&mut m.bus).unwrap();
    m.cpu.set_pc_npc(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC);
    assert_eq!(m.cpu.psr(), SOLARIS_IDLE_PSR);
    m
}

fn iteration(m: &mut Machine) {
    for expected in [
        0xf006fa24, 0xf006fa28, 0xf006fa2c, 0xf006fa34, 0xf006fa38, 0xf006fa3c, 0xf006fa40,
        0xf006fa54, 0xf006fa58, 0xf006fa5c, 0xf0071708, 0xf007170c, 0xf006fa60, 0xf006fa64,
    ] {
        assert_eq!(m.cpu.pc(), expected);
        assert!(matches!(
            m.cpu.step(&mut m.bus).unwrap(),
            Step::Executed { .. }
        ));
    }
    assert_eq!(m.cpu.pc(), SOLARIS_IDLE_PC);
    assert_eq!(m.cpu.npc(), SOLARIS_IDLE_NPC);
    assert_eq!(m.cpu.psr(), SOLARIS_IDLE_PSR);
}

fn idle_machine() -> Machine {
    let mut m = address_only_machine();
    for (pa, word) in [
        (0x1000, 0x201),
        (0x23c0, 0xee),
        (0x06fa24, 0xd00220ec),
        (0x06fa28, 0x80a22001),
        (0x06fa2c, 0x3280000f),
        (0x06fa30, 0x01000000),
        (0x06fa34, 0xd0062058),
        (0x06fa38, 0x80a22000),
        (0x06fa3c, 0x02800006),
        (0x06fa40, 0x213c097e),
        (0x06fa54, 0xe00421e8),
        (0x06fa58, 0x9fc40000),
        (0x06fa5c, 0x01000000),
        (0x071708, 0x81c3e008),
        (0x07170c, 0x01000000),
        (0x06fa60, 0x10bffff1),
        (0x06fa64, 0x113c097e),
        (0x25f8ec, 1),
        (0x26dea0, 0),
        (0x25f9e8, 0xf0071708),
    ] {
        m.bus.write32(pa, word).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.write_reg(8, 0xf025f800);
    m.cpu.write_reg(24, 0xf026de48);
    m.cpu.write_reg(16, 0xf0071708);
    m.cpu.write_reg(15, 0xf006fa58);
    for _ in 0..8 {
        iteration(&mut m);
    }
    m
}

#[test]
fn token_issues_only_at_the_validated_boundary() {
    let m = idle_machine();
    let before = m.diagnostic_snapshot();
    assert!(m.parking_token().is_ok());
    assert_eq!(
        before.differences(&m.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );

    let other = address_only_machine();
    assert!(matches!(
        other.parking_token(),
        Err(ParkingError::NotIdleBoundary)
    ));
}

#[test]
fn parked_advance_early_wakeup_is_single_use_and_unclipped() {
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    let before = m.diagnostic_snapshot();
    let advance = m.advance_parked(token, 1_000).unwrap();
    assert_eq!(advance.requested_ns, 1_000);
    assert_eq!(advance.advanced_ns, 1_000);
    assert!(!advance.clipped);
    assert!(!advance.service_required);
    assert_eq!(advance.next_ns, Some(1_000_000_000));
    let after = m.diagnostic_snapshot();
    assert_eq!(before.cpu, after.cpu);
    assert_eq!(before.bus.memory_digest, after.bus.memory_digest);
    assert_eq!(before.bus.dram_digest, after.bus.dram_digest);
}

#[test]
fn token_stales_after_cpu_memory_mapping_device_time_and_reset_changes() {
    // CPU register change.
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.cpu.write_reg(1, 0xdead_beef);
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    // Memory write through the bus.
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.bus.write32(0x8000, 0x1234_5678).unwrap();
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    // MMU mapping/control write (CPU/MMU snapshot).
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, 0x201)
        .unwrap();
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    // Device state change through the installed device block.
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.bus
        .devices_mut()
        .unwrap()
        .timers
        .write(true, 8, 5 << 9)
        .unwrap();
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    // Time advancement.
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.advance_ns(1);
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    // Whole-machine reset.
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.reset();
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    // A token from another machine is not accepted.
    let a = idle_machine();
    let token = a.parking_token().unwrap();
    let mut b = idle_machine();
    assert_eq!(b.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);
    assert!(a.parking_token().is_ok());
}

#[test]
fn parked_advance_stops_at_a_coincident_timer_deadline() {
    let mut m = idle_machine();
    m.bus.write32(m.bus.layout().system_timer, 4 << 9).unwrap();
    m.bus.write32(m.bus.layout().cpu_timer, 4 << 9).unwrap();
    let token = m.parking_token().unwrap();
    let advance = m.advance_parked(token, 10_000).unwrap();
    assert_eq!(advance.requested_ns, 10_000);
    assert_eq!(advance.advanced_ns, 1_500);
    assert!(advance.clipped);
    assert!(advance.service_required);
    assert_eq!(advance.next_ns, Some(1_500));
    assert!(m.bus.devices().unwrap().timers.system_irq());
    assert!(m.bus.devices().unwrap().timers.processor_irq());
}

#[test]
fn parked_advance_preserves_cpu_mmu_and_ram_at_an_event_boundary() {
    let mut m = idle_machine();
    m.bus.write32(m.bus.layout().system_timer, 4 << 9).unwrap();
    let before = m.diagnostic_snapshot();
    let token = m.parking_token().unwrap();
    let advance = m.advance_parked(token, 10_000).unwrap();
    assert_eq!(advance.advanced_ns, 1_500);
    let after = m.diagnostic_snapshot();
    assert_eq!(before.cpu, after.cpu);
    assert_eq!(before.bus.memory_digest, after.bus.memory_digest);
    assert_eq!(before.bus.dram_digest, after.bus.dram_digest);
}

#[test]
fn parked_token_is_stale_after_reset() {
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.reset();
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);
}

#[test]
fn admission_rejects_changed_code_loads_and_helper_before_issuance() {
    let mut changed_code = idle_machine();
    changed_code.bus.write32(0x06fa24, 0x8200_6001).unwrap();
    assert!(matches!(
        changed_code.parking_token(),
        Err(ParkingError::NotIdleBoundary)
    ));

    let mut first_exit = idle_machine();
    first_exit.bus.write32(0x25f8ec, 0).unwrap();
    assert!(matches!(
        first_exit.parking_token(),
        Err(ParkingError::NotIdleBoundary)
    ));

    let mut second_exit = idle_machine();
    second_exit.bus.write32(0x26dea0, 1).unwrap();
    assert!(matches!(
        second_exit.parking_token(),
        Err(ParkingError::NotIdleBoundary)
    ));

    let mut changed_helper = idle_machine();
    changed_helper.bus.write32(0x25f9e8, 0xf0071800).unwrap();
    changed_helper.bus.write32(0x071800, 0x8200_6001).unwrap();
    assert!(matches!(
        changed_helper.parking_token(),
        Err(ParkingError::NotIdleBoundary)
    ));

    let mut unmapped = idle_machine();
    unmapped
        .cpu
        .mmu
        .store(&mut unmapped.bus, 0, 4, Width::Word, 0)
        .unwrap();
    assert!(matches!(
        unmapped.parking_token(),
        Err(ParkingError::NotIdleBoundary)
    ));
}

#[test]
fn pending_interrupt_or_service_work_rejects_admission() {
    let mut irq = idle_machine();
    let l = irq.bus.layout();
    irq.bus.write32(l.system_interrupt + 8, u32::MAX).unwrap();
    irq.bus.write32(l.cpu_interrupt + 8, 1 << 28).unwrap();
    assert_eq!(irq.bus.interrupt_levels(), 1 << 12);
    assert_eq!(
        irq.parking_token().unwrap_err(),
        ParkingError::ServiceRequired
    );

    let mut serial = idle_machine();
    let l = serial.bus.layout();
    for (reg, value) in [(3, 0xc1), (1, 0x10), (9, 8)] {
        serial.bus.write8(l.serial + 4, reg).unwrap();
        serial.bus.write8(l.serial + 4, value).unwrap();
    }
    serial
        .bus
        .write32(l.system_interrupt + 8, (1 << 31) | (1 << 15))
        .unwrap();
    serial.bus.write32(l.scsi_dma, 0).unwrap();
    assert!(serial
        .bus
        .devices_mut()
        .unwrap()
        .serial
        .receive(Channel::A, b'X'));
    assert!(serial.bus.devices().unwrap().serial.irq());
    assert_eq!(serial.bus.interrupt_levels(), 0);
    assert_eq!(
        serial.parking_token().unwrap_err(),
        ParkingError::ServiceRequired
    );
}

#[test]
fn reached_esp_timeout_requires_service_even_if_deadline_disappears() {
    let mut m = idle_machine();
    let l = m.bus.layout();
    m.bus.write32(l.system_interrupt + 8, u32::MAX).unwrap();
    m.bus.write32(l.scsi_dma, 0x10).unwrap();
    for (off, value) in [
        (12, 0),
        (32, 7),
        (36, 2),
        (20, 1),
        (16, 3),
        (8, 0x80),
        (12, 0x42),
    ] {
        m.bus.write8(l.esp + off, value).unwrap();
    }
    m.service_scsi().unwrap();
    assert_eq!(m.timebase().next_ns, Some(409_600));
    let token = m.parking_token().unwrap();
    let result = m.advance_parked(token, 409_600).unwrap();
    assert_eq!(result.advanced_ns, 409_600);
    assert!(!result.clipped);
    assert!(m.bus.devices().unwrap().esp.irq());
    assert_eq!(m.bus.interrupt_levels(), 1 << 4);
    assert!(result.service_required);
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::ServiceRequired
    );
}
