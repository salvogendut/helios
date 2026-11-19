#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::cpu::{Icc, Step};
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};
use std::num::NonZeroUsize;

const VA: u32 = 0x1234_5000;
const PA: u64 = 0x8000;
const CMP_G1_ONE: u32 = 0x80a0_6001; // subcc %g1, 1, %g0
const INC_G2: u32 = 0x8400_a001;

fn program(offset: u32, words: &[u32], input: u32, native: bool) -> Machine {
    let profile = MachineProfile::Ss20;
    let mut m =
        Machine::with_profile(profile, 16 << 20, &[1, 0, 0, 0], profile.default_nvram()).unwrap();
    for (address, value) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
    ] {
        m.bus.write32(address, value).unwrap();
    }
    for (index, &word) in words.iter().enumerate() {
        m.bus
            .write32(PA + u64::from(offset) + index as u64 * 4, word)
            .unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.set_pc_npc(VA + offset, VA + offset + 4);
    m.cpu.write_reg(1, input);
    // Deliberately seed Z opposite to the CMP result.
    m.cpu.set_icc(Icc {
        z: input != 1,
        ..Default::default()
    });
    m.set_native_execution_enabled(native);
    m.set_interpret_span_fallback(true);
    m
}

fn same_state(reference: &Machine, actual: &Machine) {
    let differences = reference
        .diagnostic_snapshot()
        .differences(&actual.diagnostic_snapshot());
    assert!(differences.is_empty(), "{differences:?}");
    #[cfg(feature = "perf-counters")]
    assert_eq!(
        reference.cpu.mmu.translation_counters(),
        actual.cpu.mmu.translation_counters()
    );
}

fn compare_prefix(reference: &mut Machine, actual: &mut Machine, mut remaining: usize) {
    while let Some(budget) = NonZeroUsize::new(remaining) {
        let result = actual.step_native_span(budget).unwrap();
        assert!(!result.is_empty() && result.len() <= remaining);
        for step in result.iter() {
            assert_eq!(step, reference.step().unwrap());
        }
        remaining -= result.len();
        same_state(reference, actual);
    }
}

#[test]
fn compare_branch_exit_with_fresh_icc_and_side_effecting_slot() {
    // All branches target the preceding CMP. The slot increments %g2.
    for (input, branch, expected_pc, expected_npc, executes_slot) in [
        (1, 0x12bf_ffff, VA + 8, VA + 12, true),   // BNE, untaken
        (2, 0x12bf_ffff, VA + 8, VA, true),        // BNE, taken
        (1, 0x32bf_ffff, VA + 12, VA + 16, false), // BNE,a, untaken
        (2, 0x32bf_ffff, VA + 8, VA, true),        // BNE,a, taken
        (1, 0x30bf_ffff, VA, VA + 4, false),       // BA,a
        (2, 0x20bf_ffff, VA + 12, VA + 16, false), // BN,a
    ] {
        let words = [CMP_G1_ONE, branch, INC_G2, 0x0100_0000];
        let mut reference = program(0, &words, input, false);
        let mut actual = program(0, &words, input, true);
        compare_prefix(&mut reference, &mut actual, 2);
        assert_eq!(
            (actual.cpu.pc(), actual.cpu.npc()),
            (expected_pc, expected_npc)
        );
        assert_eq!(actual.cpu.read_reg(2), 0);
        compare_prefix(&mut reference, &mut actual, 1);
        assert_eq!(actual.cpu.read_reg(2), u32::from(executes_slot));
    }
}

#[test]
fn page_edge_unmapped_slot_is_only_fetched_on_the_executed_path() {
    for (branch, skips_slot) in [(0x30bf_ffff, true), (0x10bf_ffff, false)] {
        let words = [CMP_G1_ONE, branch];
        let mut reference = program(0xff8, &words, 1, false);
        let mut actual = program(0xff8, &words, 1, true);
        compare_prefix(&mut reference, &mut actual, 2);
        if skips_slot {
            assert_eq!(actual.cpu.pc(), VA + 0xff8);
            compare_prefix(&mut reference, &mut actual, 1);
        } else {
            assert_eq!(
                (actual.cpu.pc(), actual.cpu.npc()),
                (VA + 0x1000, VA + 0xff8)
            );
            let fault = actual.step_native_span(NonZeroUsize::MIN).unwrap_err();
            assert_eq!(fault, reference.step().unwrap_err());
            assert_eq!((fault.pc, fault.npc), (VA + 0x1000, VA + 0xff8));
            same_state(&reference, &actual);
        }
    }
}

#[test]
fn native_cmp_bne_exit_fits_one_two_outcome_call() {
    let words = [CMP_G1_ONE, 0x12bf_ffff, INC_G2];
    let mut m = program(0, &words, 2, true);
    let before = m.native_execution_calls();
    let result = m.step_native_span(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result.native_instructions, 2);
    assert_eq!(m.native_execution_calls() - before, 1);
    assert_eq!(
        result.get(1),
        Some(Step::Executed {
            pc: VA + 4,
            npc: VA + 8,
            instruction: words[1]
        })
    );
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (VA + 8, VA));
}

fn with_traps(slot: u32, native: bool) -> Machine {
    use helios::cpu::{PSR_ET, PSR_S};
    let words = [
        0x8188_6000,
        0x0100_0000,
        0x0100_0000,
        0x0100_0000,
        CMP_G1_ONE,
        0x12bf_ffff,
        slot,
    ];
    let mut m = program(0, &words, PSR_ET | PSR_S, native);
    for _ in 0..4 {
        m.step().unwrap();
    }
    m.cpu.write_reg(1, 2);
    m
}

#[test]
fn interrupt_between_branch_and_slot_saves_the_slot_and_target() {
    let mut reference = with_traps(INC_G2, false);
    let mut actual = with_traps(INC_G2, true);
    compare_prefix(&mut reference, &mut actual, 2);
    assert_eq!((actual.cpu.pc(), actual.cpu.npc()), (VA + 24, VA + 16));
    for m in [&mut reference, &mut actual] {
        let layout = m.bus.layout();
        m.bus
            .write32(layout.system_interrupt + 8, u32::MAX)
            .unwrap();
        m.bus
            .write32(layout.cpu_interrupt + 8, 1 << (16 + 10))
            .unwrap();
    }
    let before = actual.native_execution_count();
    let out = actual
        .step_native_span(NonZeroUsize::new(16).unwrap())
        .unwrap();
    let entry = reference.step().unwrap();
    assert_eq!(out.get(0), Some(entry));
    assert!(
        matches!(entry, Step::InterruptTaken { level: 10, pc, npc, .. } if (pc,npc) == (VA+24,VA+16))
    );
    assert_eq!((out.len(), out.native_instructions), (1, 0));
    assert_eq!(actual.cpu.read_reg(2), 0);
    assert_eq!(actual.native_execution_count(), before);
    assert_eq!(
        (actual.cpu.read_reg(17), actual.cpu.read_reg(18)),
        (VA + 24, VA + 16)
    );
    same_state(&reference, &actual);
}

#[test]
fn fault_in_executed_slot_enters_guest_trap_at_the_exact_slot() {
    let mut reference = with_traps(0, false); // UNIMP
    let mut actual = with_traps(0, true);
    compare_prefix(&mut reference, &mut actual, 2);
    let out = actual
        .step_native_span(NonZeroUsize::new(16).unwrap())
        .unwrap();
    let entry = reference.step().unwrap();
    assert_eq!(out.get(0), Some(entry));
    assert!(matches!(entry, Step::TrapTaken {trap_type: 2, fault,..} if
        (fault.pc,fault.npc,fault.instruction)==(VA+24,VA+16,Some(0))));
    assert_eq!((out.len(), out.native_instructions), (1, 0));
    assert_eq!(
        (actual.cpu.read_reg(17), actual.cpu.read_reg(18)),
        (VA + 24, VA + 16)
    );
    same_state(&reference, &actual);
}

#[test]
fn slot_that_changes_icc_cannot_reverse_the_chosen_target() {
    // CMP %g1,1 gives NZ, the slot CMP %g0,0 then gives Z.
    let words = [CMP_G1_ONE, 0x12bf_ffff, 0x80a0_2000];
    let mut reference = program(0, &words, 2, false);
    let mut actual = program(0, &words, 2, true);
    compare_prefix(&mut reference, &mut actual, 3);
    assert!(actual.cpu.icc().z);
    assert_eq!((actual.cpu.pc(), actual.cpu.npc()), (VA, VA + 4));
}

#[test]
fn same_cached_span_relocates_at_two_virtual_addresses() {
    let words = [CMP_G1_ONE, 0x12bf_ffff];
    let mut a = program(0, &words, 2, false);
    let mut b = program(0, &words, 2, true);
    for m in [&mut a, &mut b] {
        for (i, word) in words.into_iter().enumerate() {
            m.bus.write32(PA + 0x100 + i as u64 * 4, word).unwrap();
        }
    }
    for offset in [0, 0x100] {
        for m in [&mut a, &mut b] {
            m.cpu.set_pc_npc(VA + offset, VA + offset + 4);
        }
        let before = b.native_execution_calls();
        compare_prefix(&mut a, &mut b, 2);
        assert_eq!(b.native_execution_calls() - before, 1);
        assert_eq!((b.cpu.pc(), b.cpu.npc()), (VA + offset + 8, VA + offset));
    }
}
