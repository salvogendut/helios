#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};
use std::num::NonZeroUsize;

const VA: u32 = 0x1234_5000;
const LEAF: u64 = 0x4014;
const ADD: u32 = 0x8200_6001;

fn mapped(words: &[u32]) -> Machine {
    let mut m = Machine::with_profile(
        MachineProfile::Ss20,
        16 * 1024 * 1024,
        &ADD.to_be_bytes(),
        MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (LEAF, 0x088e),
    ] {
        m.bus.write32(address, word).unwrap();
    }
    for (index, word) in words.iter().enumerate() {
        m.bus.write32(0x8000 + (index as u64) * 4, *word).unwrap();
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

fn run_span(budget: usize) -> (Machine, usize, u64) {
    let words = vec![ADD; 16];
    let mut expected = mapped(&words);
    let mut actual = mapped(&words);
    for _ in 0..budget.min(16) {
        assert!(matches!(expected.step().unwrap(), Step::Executed { .. }));
    }
    let calls = actual.native_execution_calls();
    let result = actual
        .step_native_span(NonZeroUsize::new(budget).unwrap())
        .unwrap();
    for (index, step) in result.iter().enumerate() {
        assert!(matches!(step, Step::Executed { .. }), "{index}");
    }
    assert_eq!(actual.native_execution_calls() - calls, 1);
    assert_eq!(
        expected
            .diagnostic_snapshot()
            .differences(&actual.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
    (actual, result.len(), u64::from(result.native_instructions))
}

#[test]
fn spans_of_two_three_fifteen_and_sixteen_match_interpreters() {
    for budget in [2, 3, 15, 16] {
        let (machine, outcomes, native) = run_span(budget);
        assert_eq!(outcomes, budget);
        assert_eq!(native, budget as u64);
        assert_eq!(machine.cpu.read_reg(1), budget as u32);
    }
}

#[test]
fn single_instruction_budget_uses_bridge_only() {
    let (machine, outcomes, native) = run_span(1);
    assert_eq!(outcomes, 1);
    assert_eq!(native, 1);
    assert_eq!(machine.cpu.read_reg(1), 1);
}

#[test]
fn unsupported_suffix_shortens_the_span() {
    let mut words = vec![ADD; 16];
    words.push(0x1080_0000); // ba
    let mut reference = mapped(&words);
    for _ in 0..16 {
        assert!(matches!(reference.step().unwrap(), Step::Executed { .. }));
    }
    let mut actual = mapped(&words);
    let calls = actual.native_execution_calls();
    let result = actual
        .step_native_span(NonZeroUsize::new(16).unwrap())
        .unwrap();
    assert_eq!(result.len(), 16);
    assert_eq!(u64::from(result.native_instructions), 16);
    assert_eq!(actual.native_execution_calls() - calls, 1);
    assert_eq!(
        reference
            .diagnostic_snapshot()
            .differences(&actual.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}
