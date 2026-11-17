use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_PS, PSR_S};
use helios::devices::serial::Channel;
use helios::machine::{
    InstalledVideo, Machine, MachineProfile, SOLARIS_IDLE_NPC, SOLARIS_IDLE_PC, SOLARIS_IDLE_PSR,
};
use helios::mmu::{Width, PCR_EN};

const LOOP: [u8; 8] = [0x10, 0x80, 0, 0, 1, 0, 0, 0];

// This deliberately reproduces the submitted tests' admission fixture. It has
// no instruction or data mapping at the supposedly recognized Solaris PC.
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
    let words = [0x8188_6000u32, 0x80a0_0000u32];
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

// Reconstruct the actual captured loop and its three load dependencies. An L1
// 16 MiB mapping translates f0xxxxxx to physical 00xxxxxx, with R/M pre-set.
// Eight complete warmup iterations make the MMU diagnostic view steady too.
pub fn idle_machine() -> Machine {
    let mut m = address_only_machine();
    for (pa, word) in [
        (0x1000, 0x201),
        (0x23c0, 0xee),
        (0x06fa24, 0xd00220ec),
        (0x06fa28, 0x80a22001),
        (0x06fa2c, 0x3280000f),
        (0x06fa30, 0x01000000), // annulled on the captured path
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

#[test]
fn mapped_fixture_really_executes_the_complete_idle_loop() {
    let mut m = idle_machine();
    let before = m.diagnostic_snapshot();
    iteration(&mut m);
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
    let token = m.parking_token().expect("known idle control");
    assert!(m.validate_parking_token(&token));
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
    let result = m.advance_parked(token, 1_000).unwrap();
    assert_eq!(result.advanced_ns, 1_000);
    assert!(!result.service_required);
}

#[test]
fn matching_pc_psr_without_executable_loop_is_rejected() {
    let m = address_only_machine();
    assert!(
        m.parking_token().is_err(),
        "matching PC/PSR alone is not idle proof"
    );
}

#[test]
fn changed_code_before_issuance_is_rejected() {
    let mut m = idle_machine();
    m.bus.write32(0x06fa24, 0x82006001).unwrap(); // add g1,1,g1
    assert!(
        m.parking_token().is_err(),
        "would suppress a real register write"
    );
}

#[test]
fn first_load_exit_condition_before_issuance_is_rejected() {
    let mut m = idle_machine();
    m.bus.write32(0x25f8ec, 0).unwrap();
    assert!(
        m.parking_token().is_err(),
        "first branch will leave the idle path"
    );
}

#[test]
fn second_load_exit_condition_before_issuance_is_rejected() {
    let mut m = idle_machine();
    m.bus.write32(0x26dea0, 1).unwrap();
    assert!(
        m.parking_token().is_err(),
        "scheduler work is already pending"
    );
}

#[test]
fn changed_indirect_helper_before_issuance_is_rejected() {
    let mut m = idle_machine();
    m.bus.write32(0x25f9e8, 0xf0071800).unwrap();
    m.bus.write32(0x071800, 0x82006001).unwrap();
    assert!(
        m.parking_token().is_err(),
        "idle callback now points to active code"
    );
}

#[test]
fn already_deliverable_software_interrupt_prevents_advancement() {
    let mut m = idle_machine();
    let l = m.bus.layout();
    m.bus.write32(l.system_interrupt + 8, u32::MAX).unwrap();
    m.bus.write32(l.cpu_interrupt + 8, 1 << 28).unwrap(); // software PIL 12
    assert_eq!(m.bus.interrupt_levels(), 1 << 12);
    let Ok(token) = m.parking_token() else {
        return;
    };
    let result = m.advance_parked(token, 10_000).unwrap();
    assert_eq!(
        result.advanced_ns, 0,
        "PIL 12 must be serviced at the current boundary"
    );
    assert!(result.service_required);
}

#[test]
fn pending_serial_source_prevents_advancement_before_irq_sync() {
    let mut m = idle_machine();
    let l = m.bus.layout();
    for (reg, val) in [(3, 0xc1), (1, 0x10), (9, 8)] {
        m.bus.write8(l.serial + 4, reg).unwrap();
        m.bus.write8(l.serial + 4, val).unwrap();
    }
    m.bus
        .write32(l.system_interrupt + 8, (1 << 31) | (1 << 15))
        .unwrap();
    m.bus.write32(l.scsi_dma, 0).unwrap();
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .serial
        .receive(Channel::A, b'X'));
    assert!(m.bus.devices().unwrap().serial.irq());
    assert_eq!(m.bus.interrupt_levels(), 0); // normal pre-step service will sync it
    let Ok(token) = m.parking_token() else {
        return;
    };
    let result = m.advance_parked(token, 10_000).unwrap();
    assert_eq!(
        result.advanced_ns, 0,
        "normal pre-step service must run first"
    );
    assert!(result.service_required);
}

#[test]
fn exact_esp_timeout_return_requires_service_even_when_deadline_disappears() {
    let mut m = idle_machine();
    let l = m.bus.layout();
    m.bus.write32(l.system_interrupt + 8, u32::MAX).unwrap();
    m.bus.write32(l.scsi_dma, 0x10).unwrap(); // DMA2 interrupt enable
    for (off, val) in [
        (12, 0),
        (32, 7),
        (36, 2),
        (20, 1),
        (16, 3),
        (8, 0x80),
        (12, 0x42),
    ] {
        m.bus.write8(l.esp + off, val).unwrap();
    }
    m.service_scsi().unwrap();
    assert_eq!(m.timebase().next_ns, Some(409_600));
    assert!(!m.bus.devices().unwrap().esp.irq());
    let token = m.parking_token().unwrap();
    let result = m.advance_parked(token, 409_600).unwrap();
    assert_eq!(result.advanced_ns, 409_600);
    assert!(!result.clipped);
    assert!(m.bus.devices().unwrap().esp.irq());
    assert_eq!(m.bus.interrupt_levels(), 1 << 4);
    assert!(
        result.service_required,
        "reached timeout now needs an interrupt; result={result:?}"
    );
}

#[test]
fn changed_known_dependencies_after_issuance_stale_token() {
    for (pa, value) in [
        (0x06fa24, 0x82006001),
        (0x25f8ec, 0),
        (0x26dea0, 1),
        (0x25f9e8, 0xf0071800),
    ] {
        let mut m = idle_machine();
        let token = m.parking_token().unwrap();
        m.bus.write32(pa, value).unwrap();
        assert!(!m.validate_parking_token(&token));
    }
}
