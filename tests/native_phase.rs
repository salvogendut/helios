#![cfg(feature = "jit-profile")]
use helios::{
    bus::Bus,
    cpu::{Step, PSR_ET, PSR_S},
    machine::{Machine, MachineProfile},
    mmu::{Width, PCR_EN},
};
use std::num::NonZeroUsize;
const VA: u32 = 0x12345000;
const ADD: u32 = 0x82006001;

fn mapped() -> Machine {
    let p = MachineProfile::Ss20;
    let mut m = Machine::with_profile(p, 16 << 20, &[1, 0, 0, 0], p.default_nvram()).unwrap();
    for (a, w) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
    ] {
        m.bus.write32(a, w).unwrap();
    }
    for i in 0..16 {
        m.bus.write32(0x8000 + 4 * i, ADD).unwrap();
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
    m.set_native_execution_enabled(true);
    m
}

#[test]
fn warmed_span_and_word_cache_report_per_phase_counts() {
    let mut m = mapped();
    let base = m.cpu.jit_phase();
    for _ in 0..2 {
        m.cpu.set_pc_npc(VA, VA + 4);
        m.advance_ns(0);
        m.step_native_span(NonZeroUsize::new(16).unwrap()).unwrap();
    }
    let midpoint = m.cpu.jit_phase();
    let span = midpoint.since(&base).unwrap();
    assert_eq!(
        (
            span.native_instructions(),
            span.calls_by_length[15],
            span.spans.compiled,
            span.spans.misses,
            span.spans.hits
        ),
        (32, 2, 1, 1, 1)
    );
    for _ in 0..2 {
        m.cpu.set_pc_npc(VA, VA + 4);
        m.step_native_span(NonZeroUsize::MIN).unwrap();
    }
    let word = m.cpu.jit_phase().since(&midpoint).unwrap();
    assert_eq!(
        (
            word.native_instructions(),
            word.calls_by_length[0],
            word.words.compiled,
            word.words.misses,
            word.words.hits
        ),
        (2, 2, 1, 1, 1)
    );
    assert_eq!((word.interpreted, word.spans.hits), (0, 0));
    assert!(base.since(&midpoint).is_none());
}

#[test]
fn cached_unsupported_then_interpreted_policy_and_interpreter_retirements() {
    let mut m = mapped();
    m.bus.write32(0x8004, 0x10800000).unwrap();
    m.bus.write32(0x8008, 0xc8008000).unwrap(); // ld [%g2], %g4
    m.cpu.write_reg(2, VA);
    let start = m.cpu.jit_phase();
    for _ in 0..2 {
        m.cpu.set_pc_npc(VA + 4, VA + 8);
        m.step_native_span(NonZeroUsize::MIN).unwrap();
    }
    m.set_interpret_span_fallback(true);
    m.cpu.set_pc_npc(VA, VA + 4);
    for _ in 0..3 {
        m.step_native_span(NonZeroUsize::MIN).unwrap();
    }
    m.set_native_execution_enabled(false);
    m.cpu.set_pc_npc(VA, VA + 4);
    m.step().unwrap();
    let c = m.cpu.jit_phase().since(&start).unwrap();
    assert_eq!(m.cpu.read_reg(4), ADD);
    assert_eq!(
        (
            c.interpreted,
            c.fallbacks,
            c.control_fallbacks,
            c.memory_fallbacks
        ),
        (6, 5, 3, 1)
    );
    assert_eq!((c.words.misses, c.words.compiled, c.words.hits), (1, 0, 1));
}

#[test]
fn traps_interrupts_and_error_returns_have_separate_units() {
    let mut m = mapped();
    m.bus.write32(0x8000, 0x81886000).unwrap();
    m.cpu.write_reg(1, PSR_S | PSR_ET);
    for _ in 0..4 {
        m.cpu.step(&mut m.bus).unwrap();
    }
    let start = m.cpu.jit_phase();
    assert!(matches!(
        m.cpu.step_with_interrupts(&mut m.bus, 1 << 12).unwrap(),
        Step::InterruptTaken { level: 12, .. }
    ));
    m.cpu.set_pc_npc(VA, VA + 4);
    m.bus.write32(0x8000, 0).unwrap();
    assert!(m.step_native_span(NonZeroUsize::MIN).is_err());
    let counts = m.cpu.jit_phase().since(&start).unwrap();
    assert_eq!(
        (
            counts.interpreted,
            counts.traps,
            counts.interrupts,
            counts.cpu_errors
        ),
        (0, 0, 1, 1)
    );
    let mut m = mapped();
    m.bus.write32(0x8000, 0x81886000).unwrap();
    m.cpu.write_reg(1, PSR_S | PSR_ET);
    for _ in 0..4 {
        m.cpu.step(&mut m.bus).unwrap();
    }
    let start = m.cpu.jit_phase();
    m.bus.write32(0x8010, 0).unwrap();
    assert!(matches!(
        m.step_native_span(NonZeroUsize::MIN).unwrap().get(0),
        Some(Step::TrapTaken { .. })
    ));
    assert_eq!(m.cpu.jit_phase().since(&start).unwrap().traps, 1);
}

#[test]
fn epochs_change_for_all_cpu0_resets_and_replacement() {
    let mut m = mapped();
    let initial = m.cpu.jit_phase();
    assert!(mapped().cpu.jit_phase().since(&initial).is_none());
    m.reset();
    let power = m.cpu.jit_phase();
    assert!(power.since(&initial).is_none());
    assert!(m.reset_processor_module(0));
    let module = m.cpu.jit_phase();
    assert!(module.since(&power).is_none());
    m.bus.write32(m.bus.layout().reset, 1).unwrap();
    assert!(matches!(m.step(), Ok(Step::SoftwareReset { .. })));
    assert!(m.cpu.jit_phase().since(&module).is_none());
}

#[test]
fn word_cache_capacity_counts_entries_evicted() {
    let mut m = mapped();
    let base = m.cpu.jit_phase();
    for imm in 0..=4096 {
        m.bus.write32(0x8000, 0x03000000 | imm).unwrap();
        m.cpu.set_pc_npc(VA, VA + 4);
        m.step_native_span(NonZeroUsize::MIN).unwrap();
    }
    let stats = m.cpu.jit_phase().since(&base).unwrap();
    assert_eq!(
        (
            stats.words.misses,
            stats.words.compiled,
            stats.words.evicted_entries
        ),
        (4097, 4097, 4096)
    );
}
