#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::cpu::{FaultKind, Step, PSR_ET, PSR_S};
use helios::devices::serial::Channel;
use helios::machine::{CpuClockMode, Machine, MachineProfile};
use helios::mmu::{FaultType, Width, PCR_EN};
use std::num::NonZeroUsize;

const VA: u32 = 0x1234_5000;
const LEAF: u64 = 0x4014;
const ADD: u32 = 0x8200_6001; // add %g1, 1, %g1
const ADDCC: u32 = 0x8480_6002; // addcc %g1, 2, %g2

fn mapped(profile: MachineProfile, native: bool, mode: CpuClockMode) -> Machine {
    let mut m = Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &ADD.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap();
    m.reset_with_clock_mode(mode);
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (LEAF, 0x088e),
        (0x8000, ADD),
        (0x8004, ADDCC),
    ] {
        m.bus.write32(address, word).unwrap();
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

fn equal(reference: &Machine, native: &Machine) {
    let a = reference.diagnostic_snapshot();
    let b = native.diagnostic_snapshot();
    assert_eq!(a.differences(&b), Vec::<&str>::new());
    assert_eq!(a, b);
    #[cfg(feature = "perf-counters")]
    assert_eq!(
        reference.cpu.mmu.translation_counters(),
        native.cpu.mmu.translation_counters()
    );
}

fn compare(reference: &mut Machine, native: &mut Machine, budget: usize) -> bool {
    let calls_before = native.native_execution_calls();
    let result = native
        .step_native_pair(NonZeroUsize::new(budget).unwrap())
        .unwrap();
    assert_eq!(reference.step().unwrap(), result.first);
    if let Some(second) = result.second {
        assert_eq!(reference.step().unwrap(), second);
        assert_eq!(result.native_instructions, 2);
        assert_eq!(native.native_execution_calls() - calls_before, 1);
    }
    equal(reference, native);
    result.second.is_some()
}

#[test]
fn translated_pair_is_one_native_call_with_complete_state_and_counter_parity() {
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        for clock in [CpuClockMode::Fast, CpuClockMode::Hardware] {
            let mut a = mapped(profile, false, clock);
            let mut b = mapped(profile, true, clock);
            assert!(compare(&mut a, &mut b, 2), "{profile:?} {clock:?}");
            assert_eq!((b.cpu.read_reg(1), b.cpu.read_reg(2)), (1, 3));
            assert_eq!(b.native_execution_count(), 2);
            assert_eq!((b.cpu.pc(), b.cpu.npc()), (VA + 8, VA + 12));
            // Exercise an already-compiled pair and both warm fetches.
            for m in [&mut a, &mut b] {
                m.cpu.set_pc_npc(VA, VA + 4);
            }
            assert!(compare(&mut a, &mut b, 2));
            assert_eq!((b.cpu.read_reg(1), b.cpu.read_reg(2)), (2, 4));
        }
    }
}

#[test]
fn one_remaining_instruction_does_not_run_cached_second_instruction() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    assert!(compare(&mut a, &mut b, 2));
    for m in [&mut a, &mut b] {
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert!(!compare(&mut a, &mut b, 1));
    assert_eq!((b.cpu.pc(), b.cpu.npc()), (VA + 4, VA + 8));
    assert_eq!(b.native_execution_count(), 3);
}

#[test]
fn warmed_permission_then_flush_has_one_precise_fault_and_no_replay() {
    for profile in [MachineProfile::Ss5, MachineProfile::Ss20] {
        let mut a = mapped(profile, false, CpuClockMode::Fast);
        let mut b = mapped(profile, true, CpuClockMode::Fast);
        assert!(compare(&mut a, &mut b, 2));
        for m in [&mut a, &mut b] {
            m.bus.write32(LEAF, 0x0882).unwrap();
            m.cpu.set_pc_npc(VA, VA + 4);
        }
        assert!(compare(&mut a, &mut b, 2));
        for m in [&mut a, &mut b] {
            m.cpu.mmu.store(&mut m.bus, VA, 3, Width::Word, 0).unwrap();
            m.cpu.set_pc_npc(VA, VA + 4);
        }
        let before = b.native_execution_count();
        let error = b
            .step_native_pair(NonZeroUsize::new(2).unwrap())
            .unwrap_err();
        assert_eq!(a.step().unwrap_err(), error);
        assert!(matches!(error.kind, FaultKind::InstructionMmu(fault)
            if fault.kind == FaultType::Protection && fault.address == VA));
        assert_eq!(b.cpu.mmu.fault_status(), 0x368);
        assert_eq!(b.native_execution_count(), before);
        equal(&a, &b);
        assert_eq!(
            b.step_native_pair(NonZeroUsize::new(2).unwrap())
                .unwrap_err(),
            error
        );
        assert_eq!(a.step().unwrap_err(), error);
        equal(&a, &b);
    }
}

#[test]
fn cold_permission_fault_preserves_registers_fault_count_and_device_state() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        m.bus.write32(LEAF, 0x0882).unwrap();
    }
    assert_eq!(
        a.step().unwrap_err(),
        b.step_native_pair(NonZeroUsize::new(2).unwrap())
            .unwrap_err()
    );
    assert_eq!(b.native_execution_count(), 0);
    assert_eq!(b.cpu.read_reg(1), 0);
    equal(&a, &b);
}

#[test]
fn next_page_fault_is_not_speculated_before_first_retirement() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        m.bus.write32(0x8ffc, ADD).unwrap();
        m.cpu.set_pc_npc(VA + 0xffc, VA + 0x1000);
    }
    assert!(!compare(&mut a, &mut b, 2));
    assert_eq!(b.cpu.read_reg(1), 1);
    assert_eq!(
        a.step().unwrap_err(),
        b.step_native_pair(NonZeroUsize::new(2).unwrap())
            .unwrap_err()
    );
    equal(&a, &b);
}

#[test]
fn raw_word_validation_catches_host_image_and_bus_stores() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    assert!(compare(&mut a, &mut b, 2));
    for m in [&mut a, &mut b] {
        m.load_image(0x8004, &0x8400_6011u32.to_be_bytes()).unwrap(); // add g1,17,g2
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert!(compare(&mut a, &mut b, 2));
    assert_eq!(b.cpu.read_reg(2), 19);
    for m in [&mut a, &mut b] {
        m.bus.write8(0x8007, 0x21).unwrap();
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert!(compare(&mut a, &mut b, 2));
    assert_eq!(b.cpu.read_reg(2), 36);
}

#[test]
fn remap_uses_fresh_physical_words_after_flush() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    assert!(compare(&mut a, &mut b, 2));
    for m in [&mut a, &mut b] {
        m.bus.write32(LEAF, 0x098e).unwrap();
        m.bus.write32(0x9000, 0x8200_6008).unwrap(); // add g1,8,g1
        m.bus.write32(0x9004, ADDCC).unwrap();
        m.cpu.mmu.store(&mut m.bus, VA, 3, Width::Word, 0).unwrap();
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert!(compare(&mut a, &mut b, 2));
    assert_eq!((b.cpu.read_reg(1), b.cpu.read_reg(2)), (9, 11));
}

#[test]
fn unsupported_second_instruction_has_no_early_fault_or_side_effect() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        m.bus.write32(0x8004, 0).unwrap();
    } // unimp
    assert!(!compare(&mut a, &mut b, 2));
    assert_eq!(b.cpu.read_reg(1), 1);
    assert_eq!(
        a.step().unwrap_err(),
        b.step_native_pair(NonZeroUsize::new(2).unwrap())
            .unwrap_err()
    );
    equal(&a, &b);
}

#[test]
fn branch_delay_and_deferred_service_boundaries_fall_back() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        m.cpu.set_pc_npc(VA, VA + 0x1000);
    }
    assert!(!compare(&mut a, &mut b, 2));
    // Bring the deferred clock to 1,800 ns without crossing a device service.
    for _ in 1..18 {
        for m in [&mut a, &mut b] {
            m.cpu.set_pc_npc(VA, VA + 4);
        }
        assert!(!compare(&mut a, &mut b, 1));
    }
    assert_eq!(b.bus.deferred_ns(), 1800);
    for m in [&mut a, &mut b] {
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert!(!compare(&mut a, &mut b, 2));
    assert_eq!(b.bus.deferred_ns(), 1900);
    for m in [&mut a, &mut b] {
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert!(!compare(&mut a, &mut b, 2));
    assert_eq!(b.bus.deferred_ns(), 0);
}

#[test]
fn reference_service_mode_and_disabled_native_never_batch() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    a.set_device_service_guards(false);
    b.set_device_service_guards(false);
    assert!(!compare(&mut a, &mut b, 2));
    b.set_native_execution_enabled(false);
    assert!(!compare(&mut a, &mut b, 2));
}

#[test]
fn result_retains_the_two_exact_outcomes() {
    let mut m = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    let result = m.step_native_pair(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!(
        result.first,
        Step::Executed {
            pc: VA,
            npc: VA + 4,
            instruction: ADD
        }
    );
    assert_eq!(
        result.second,
        Some(Step::Executed {
            pc: VA + 4,
            npc: VA + 8,
            instruction: ADDCC
        })
    );
    assert_eq!(result.native_instructions, 2);
}

fn enable_traps(m: &mut Machine) {
    for (address, word) in [
        (0x8010, 0x8188_6000), // wr %g1, 0, %psr
        (0x8014, 0x0100_0000),
        (0x8018, 0x0100_0000),
        (0x801c, 0x0100_0000),
    ] {
        m.bus.write32(address, word).unwrap();
    }
    m.cpu.write_reg(1, PSR_S | PSR_ET);
    m.cpu.set_pc_npc(VA + 0x10, VA + 0x14);
    for _ in 0..4 {
        m.cpu.step(&mut m.bus).unwrap();
    }
    m.cpu.write_reg(1, 0);
    m.cpu.set_pc_npc(VA, VA + 4);
}

#[test]
fn serial_irq_and_held_dma_reset_preserve_first_outcome() {
    for held_reset in [false, true] {
        let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
        let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
        for m in [&mut a, &mut b] {
            enable_traps(m);
            let layout = m.bus.layout();
            for (reg, value) in [(3, 0xc1), (1, 0x10), (9, 8)] {
                m.bus.write8(layout.serial + 4, reg).unwrap();
                m.bus.write8(layout.serial + 4, value).unwrap();
            }
            m.bus
                .write32(layout.system_interrupt + 8, (1 << 31) | (1 << 15))
                .unwrap();
            m.bus
                .write32(layout.scsi_dma, if held_reset { 0x80 } else { 0 })
                .unwrap();
            assert!(m
                .bus
                .devices_mut()
                .unwrap()
                .serial
                .receive(Channel::A, b'X'));
        }
        let result = b.step_native_pair(NonZeroUsize::new(2).unwrap()).unwrap();
        assert_eq!(result.first, a.step().unwrap());
        assert_eq!(result.second, None);
        if held_reset {
            assert!(matches!(result.first, Step::Executed { pc: VA, .. }));
            assert_eq!(result.native_instructions, 1);
        } else {
            assert!(matches!(
                result.first,
                Step::InterruptTaken { level: 12, .. }
            ));
            assert_eq!(result.native_instructions, 0);
        }
        equal(&a, &b);
    }
}

#[test]
fn timer_deadline_blocks_pair_before_deferred_quantum() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    for m in [&mut a, &mut b] {
        m.bus.write32(m.bus.system_timer_base(), 4 << 9).unwrap();
        m.advance_ns(1400);
        assert_eq!(m.bus.deferred_ns(), 0);
        assert_eq!(m.timebase().next_device_ns, Some(1500));
    }
    assert!(!compare(&mut a, &mut b, 2));
    assert_eq!(b.virtual_ns(), 1500);
}

#[test]
fn pending_reset_precedes_both_instructions_and_resets_counts() {
    let mut a = mapped(MachineProfile::Ss20, false, CpuClockMode::Fast);
    let mut b = mapped(MachineProfile::Ss20, true, CpuClockMode::Fast);
    assert!(compare(&mut a, &mut b, 2));
    for m in [&mut a, &mut b] {
        m.bus.write32(m.bus.layout().reset, 1).unwrap();
    }
    let result = b.step_native_pair(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!(result.first, a.step().unwrap());
    assert!(matches!(result.first, Step::SoftwareReset { .. }));
    assert!(result.second.is_none());
    assert_eq!(result.native_instructions, 0);
    assert_eq!(b.native_execution_count(), 0);
    assert_eq!(b.native_execution_calls(), 0);
    equal(&a, &b);
}

fn check_unguarded_span_policy(interpret_single: bool) {
    // Disabling the service guards forces one outcome per call. The selected
    // engine for that outcome must agree with its return and phase counters.
    for budget in [1, 16] {
        let mut reference = mapped(MachineProfile::Ss20, false, CpuClockMode::Hardware);
        let mut candidate = mapped(MachineProfile::Ss20, true, CpuClockMode::Hardware);
        reference.set_device_service_guards(false);
        candidate.set_device_service_guards(false);
        candidate.set_interpret_span_fallback(interpret_single);
        let calls = candidate.native_execution_calls();
        let native = candidate.native_execution_count();
        #[cfg(feature = "jit-profile")]
        let start = candidate.cpu.jit_phase();
        let span = candidate
            .step_native_span(NonZeroUsize::new(budget).unwrap())
            .unwrap();
        assert_eq!(span.len(), 1);
        assert_eq!(span.get(0), Some(reference.step().unwrap()));
        equal(&reference, &candidate);
        let expected = u64::from(!interpret_single);
        assert_eq!(
            (
                candidate.native_execution_calls() - calls,
                candidate.native_execution_count() - native,
                u64::from(span.native_instructions)
            ),
            (expected, expected, expected),
            "single instruction policy, executed count, and reported count"
        );
        #[cfg(feature = "jit-profile")]
        {
            let delta = candidate.cpu.jit_phase().since(&start).unwrap();
            assert_eq!(
                (
                    delta.native_instructions(),
                    delta.interpreted,
                    delta.fallbacks
                ),
                (expected, 1 - expected, 1 - expected)
            );
        }
    }
}

#[test]
fn unguarded_span_keeps_interpreted_single_word_policy() {
    check_unguarded_span_policy(true);
}

#[test]
fn unguarded_span_reports_native_single_word_bridge() {
    check_unguarded_span_policy(false);
}
