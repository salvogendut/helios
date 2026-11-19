#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};

const ADD1: u32 = 0x8200_6001;

fn workstation(cpus: usize) -> Machine {
    let profile = MachineProfile::Ss20;
    Machine::with_profile_processors(
        profile,
        16 * 1024 * 1024,
        &ADD1.to_be_bytes(),
        profile.default_nvram(),
        cpus,
    )
    .unwrap()
}

fn mapped(next_pte: u32) -> Machine {
    let mut m = workstation(1);
    for (addr, value) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
        (0x4018, next_pte),
        (0x8ffc, ADD1),
        (0x9000, 0x8400_a002),
    ] {
        m.bus.write32(addr, value).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.set_pc_npc(0x1234_5ffc, 0x1234_6000);
    m
}

#[test]
fn translated_ss20_bridge_matches_full_state_after_each_step() {
    let mut reference = mapped(0x098e);
    let mut native = mapped(0x098e);
    native.set_native_execution_enabled(true);
    assert!(!native.cpu.mmu.boot_mode());
    for _ in 0..2 {
        assert_eq!(reference.step(), native.step());
        assert!(reference
            .diagnostic_snapshot()
            .differences(&native.diagnostic_snapshot())
            .is_empty());
    }
    assert_eq!(native.native_execution_count(), 2);
    assert_eq!(native.cpu.read_reg(1), 1);
    assert_eq!(native.cpu.read_reg(2), 2);
}

#[test]
fn translated_ss20_next_page_fault_is_precise() {
    let mut reference = mapped(0);
    let mut native = mapped(0);
    native.set_native_execution_enabled(true);
    assert!(matches!(native.step().unwrap(), Step::Executed { .. }));
    reference.step().unwrap();
    assert_eq!(native.native_execution_count(), 1);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&native.diagnostic_snapshot())
        .is_empty());
    assert_eq!(reference.step().unwrap_err(), native.step().unwrap_err());
    assert_eq!(native.native_execution_count(), 1);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&native.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn translated_ss20_permission_fault_executes_no_native_instruction() {
    let mut reference = mapped(0);
    let mut native = mapped(0);
    for machine in [&mut reference, &mut native] {
        machine.bus.write32(0x4014, 0x0882).unwrap();
    }
    native.set_native_execution_enabled(true);
    assert_eq!(reference.step().unwrap_err(), native.step().unwrap_err());
    assert_eq!(native.native_execution_count(), 0);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&native.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn dual_cpu_request_keeps_both_processors_interpreted() {
    let mut m = workstation(2);
    m.load_image(0x1000, &ADD1.to_be_bytes()).unwrap();
    m.load_image(0x2000, &ADD1.to_be_bytes()).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    assert!(m.set_processor_entry(2, 0x2000, 0x2004));
    assert!(m.set_processor_arbitration(2, true));
    m.set_native_execution_enabled(true);
    let first = m.step_scheduled().unwrap();
    let second = m.step_scheduled().unwrap();
    assert_eq!((first.processor, second.processor), (0, 2));
    assert_eq!(m.cpu.read_reg(1), 1);
    assert_eq!(m.secondary_cpu().unwrap().cpu.read_reg(1), 1);
    assert_eq!(
        m.native_execution_count(),
        0,
        "SMP is outside the native milestone"
    );
    assert_eq!(m.secondary_cpu().unwrap().cpu.native_execution_count(), 0);
    assert!(!m.native_execution_enabled());
}

#[test]
fn disabled_bridge_reports_no_native_work() {
    let mut m = workstation(1);
    m.cpu.set_pc_npc(0, 4);
    assert!(!m.native_execution_enabled());
    m.step().unwrap();
    assert_eq!(m.native_execution_count(), 0);
}
