#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::jit::NativeSpanStep;
use helios::machine::{CpuClockMode, Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};
use std::num::NonZeroUsize;

const VA: u32 = 0x1234_5000;
const ADD: u32 = 0x8200_6001;

fn mapped(profile: MachineProfile, native: bool, clock: CpuClockMode) -> Machine {
    let mut m = Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &ADD.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap();
    m.reset_with_clock_mode(clock);
    for (a, w) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
    ] {
        m.bus.write32(a, w).unwrap();
    }
    for i in 0..32 {
        m.bus.write32(0x8000 + i * 4, ADD).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.set_pc_npc(VA, VA + 4);
    m.set_native_execution_enabled(native);
    m
}

fn same(a: &Machine, b: &Machine) {
    assert!(
        !a.native_execution_enabled(),
        "oracle must be the interpreter"
    );
    let sa = a.diagnostic_snapshot();
    let sb = b.diagnostic_snapshot();
    assert!(sa == sb, "snapshot differences: {:?}", sa.differences(&sb));
    #[cfg(feature = "perf-counters")]
    assert_eq!(
        a.cpu.mmu.translation_counters(),
        b.cpu.mmu.translation_counters()
    );
}

fn compare(a: &mut Machine, b: &mut Machine, budget: usize) -> NativeSpanStep {
    let result = b
        .step_native_span(NonZeroUsize::new(budget).unwrap())
        .unwrap();
    assert!(!result.is_empty() && result.len() <= budget.min(16));
    for step in result.iter() {
        assert_eq!(step, a.step().unwrap());
    }
    same(a, b);
    result
}

#[test]
fn exact_outcomes_counters_and_clock_for_all_budgets() {
    for profile in [MachineProfile::Ss5, MachineProfile::Ss20] {
        for clock in [CpuClockMode::Fast, CpuClockMode::Hardware] {
            for budget in [1, 2, 3, 15, 16, 17] {
                let mut a = mapped(profile, false, clock);
                let mut b = mapped(profile, true, clock);
                let result = compare(&mut a, &mut b, budget);
                assert_eq!(result.len(), budget.min(16));
                assert_eq!(usize::from(result.native_instructions), budget.min(16));
                assert_eq!(b.native_execution_calls(), 1);
                assert_eq!(b.cpu.read_reg(1), budget.min(16) as u32);
                if budget == 17 {
                    assert_eq!(compare(&mut a, &mut b, 1).len(), 1);
                    assert_eq!(b.cpu.read_reg(1), 17);
                }
            }
        }
    }
}

#[test]
fn warmed_long_span_cannot_overrun_a_shorter_budget() {
    for budget in [1, 2, 3, 15] {
        let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
        let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
        assert_eq!(compare(&mut a, &mut b, 16).len(), 16);
        for m in [&mut a, &mut b] {
            m.advance_ns(0);
            m.cpu.set_pc_npc(VA, VA + 4);
        }
        assert_eq!(compare(&mut a, &mut b, budget).len(), budget);
        assert_eq!(b.cpu.read_reg(1), 16 + budget as u32);
    }
}

#[test]
fn unsupported_inside_the_candidate_span_stops_before_its_fault() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        m.bus.write32(0x800c, 0).unwrap();
    }
    let result = compare(&mut a, &mut b, 16);
    assert_eq!(result.len(), 3);
    assert_eq!(b.cpu.pc(), VA + 12);
    assert_eq!(b.native_execution_count(), 3);
    assert_eq!(
        a.step().unwrap_err(),
        b.step_native_span(NonZeroUsize::new(16).unwrap())
            .unwrap_err()
    );
    same(&a, &b);
    assert_eq!(b.native_execution_count(), 3);
}

#[test]
fn deferred_time_shortens_a_span_and_keeps_the_service_outcome() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for _ in 0..9 {
        compare(&mut a, &mut b, 1);
    }
    for m in [&mut a, &mut b] {
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert_eq!(b.bus.deferred_ns(), 900);
    assert_eq!(compare(&mut a, &mut b, 16).len(), 10);
    assert_eq!(b.bus.deferred_ns(), 1900);
    assert_eq!(compare(&mut a, &mut b, 16).len(), 1);
    assert_eq!(b.bus.deferred_ns(), 0);
    assert_eq!(compare(&mut a, &mut b, 16).len(), 16);
}

#[test]
fn page_edge_returns_only_the_prefix_before_the_next_fetch_fault() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        for offset in [0xff4, 0xff8, 0xffc] {
            m.bus.write32(0x8000 + offset, ADD).unwrap();
        }
        m.cpu.set_pc_npc(VA + 0xff4, VA + 0xff8);
    }
    assert_eq!(compare(&mut a, &mut b, 16).len(), 3);
    assert_eq!(b.cpu.pc(), VA + 0x1000);
    assert_eq!(
        a.step().unwrap_err(),
        b.step_native_span(NonZeroUsize::new(16).unwrap())
            .unwrap_err()
    );
    same(&a, &b);
}

#[test]
fn warmed_span_revalidates_late_words_after_host_code_writes() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    assert_eq!(compare(&mut a, &mut b, 16).len(), 16);
    for m in [&mut a, &mut b] {
        m.advance_ns(0);
        m.load_image(0x8030, &0x8200_6007u32.to_be_bytes()).unwrap();
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert_eq!(compare(&mut a, &mut b, 16).len(), 16);
    assert_eq!(b.cpu.read_reg(1), 38);
}
