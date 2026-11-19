#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{Machine, SchedulerError};
use helios::mmu::{Width, PCR_EN};

fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn bare(words: &[u32]) -> Machine {
    let mut m = Machine::bare_metal(0x10000, 0).unwrap();
    m.load_image(0, &image(words)).unwrap();
    m
}

fn steps(m: &mut Machine, count: usize) {
    for _ in 0..count {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
}

fn mapped_at_page_end(next_pte: u32) -> Machine {
    let mut m = bare(&[]);
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
        (0x4018, next_pte),
        (0x8ffc, 0x8200_6001), // add %g1, 1, %g1
        (0x9000, 0x8400_a002), // add %g2, 2, %g2
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
    m.cpu.set_pc_npc(0x1234_5ffc, 0x1234_6000);
    m
}

fn fault_status(m: &mut Machine) -> u32 {
    m.cpu.mmu.load(&mut m.bus, 0x1300, 4, Width::Word).unwrap()
}

#[test]
fn control_straight_line_bare_state_matches() {
    let words = [0x8200_6001, 0x8400_6007, 0x86a0_a003];
    let mut reference = bare(&words);
    let mut candidate = bare(&words);
    steps(&mut reference, 3);
    assert_eq!(run_bridge(&mut candidate, 3).unwrap().executed, 3);
    assert_eq!(reference.cpu.read_reg(1), 1);
    assert_eq!(reference.cpu.read_reg(2), 8);
    assert_eq!(reference.cpu.read_reg(3), 5);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

const BRANCH_PROGRAM: [u32; 5] = [
    0x1080_0004, // ba 0x10
    0x8200_6001, // delay: add %g1, 1, %g1
    0x8400_a063, // skipped: add %g2, 99, %g2
    0x0100_0000,
    0x8600_e007, // target: add %g3, 7, %g3
];

#[test]
fn control_branch_in_one_call_matches() {
    let mut reference = bare(&BRANCH_PROGRAM);
    let mut candidate = bare(&BRANCH_PROGRAM);
    steps(&mut reference, 3);
    assert_eq!(run_bridge(&mut candidate, 3).unwrap().executed, 3);
    assert_eq!(candidate.cpu.read_reg(2), 0);
    assert_eq!(candidate.cpu.read_reg(3), 7);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn split_run_preserves_branch_delay_target() {
    let mut reference = bare(&BRANCH_PROGRAM);
    let mut candidate = bare(&BRANCH_PROGRAM);
    steps(&mut reference, 3);
    assert_eq!(run_bridge(&mut candidate, 1).unwrap().executed, 1);
    assert_eq!((candidate.cpu.pc(), candidate.cpu.npc()), (4, 16));
    assert_eq!(run_bridge(&mut candidate, 2).unwrap().executed, 2);
    assert_eq!(candidate.cpu.read_reg(1), 1);
    assert_eq!(
        candidate.cpu.read_reg(2),
        0,
        "branch-skipped instruction must not execute"
    );
    assert_eq!(candidate.cpu.read_reg(3), 7);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn next_page_fault_is_not_observed_before_its_instruction() {
    let mut reference = mapped_at_page_end(0);
    let mut candidate = mapped_at_page_end(0);
    steps(&mut reference, 1);
    assert_eq!(run_bridge(&mut candidate, 1).unwrap().executed, 1);
    assert_eq!(candidate.cpu.read_reg(1), 1);
    assert_eq!(
        (candidate.cpu.pc(), candidate.cpu.npc()),
        (0x1234_6000, 0x1234_6004)
    );
    let reference_status = fault_status(&mut reference);
    let candidate_status = fault_status(&mut candidate);
    assert_eq!(reference_status, 0);
    assert_eq!(
        candidate_status, reference_status,
        "unexecuted next-page fetch polluted SFSR"
    );
}

#[test]
fn next_page_pte_is_not_referenced_before_its_instruction() {
    let mut reference = mapped_at_page_end(0x098e);
    let mut candidate = mapped_at_page_end(0x098e);
    steps(&mut reference, 1);
    assert_eq!(run_bridge(&mut candidate, 1).unwrap().executed, 1);
    assert_eq!(reference.bus.read32(0x4018).unwrap(), 0x098e);
    assert_eq!(
        candidate.bus.read32(0x4018).unwrap(),
        0x098e,
        "compilation must not set R in an unexecuted page's PTE"
    );
}

#[test]
fn first_fetch_permission_fault_matches_interpreter_capture() {
    let build = || {
        let mut m = mapped_at_page_end(0);
        m.bus.write32(0x4014, 0x0882).unwrap(); // no execute permission
        m
    };
    let mut reference = build();
    let mut candidate = build();
    let expected = reference.step().unwrap_err();
    let actual = run_bridge(&mut candidate, 1).unwrap_err();
    assert!(matches!(actual, SchedulerError::Cpu { processor: 0, fault } if fault == expected));
    assert_eq!(
        fault_status(&mut candidate),
        fault_status(&mut reference),
        "one guest fault must not become two MMU fault captures"
    );
}

#[test]
fn latched_cpu_fault_is_returned_on_next_run() {
    let mut reference = bare(&[0]); // UNIMP with ET=0 latches error mode
    let mut candidate = bare(&[0]);
    assert_eq!(reference.step().unwrap_err(), candidate.step().unwrap_err());
    let expected = reference.step().unwrap_err();
    let actual = run_bridge(&mut candidate, 1);
    assert!(
        matches!(actual, Err(SchedulerError::Cpu { processor: 0, fault }) if fault == expected),
        "latched fault must be returned, got {actual:?}"
    );
}

#[test]
fn pending_software_reset_is_serviced_and_reported() {
    let build = || {
        let mut m = Machine::sun4m(0x10000, &image(&[0x0100_0000])).unwrap();
        m.bus.devices_mut().unwrap().reset.write(1);
        m
    };
    let mut reference = build();
    let mut candidate = build();
    let expected = reference.step().unwrap();
    assert!(matches!(
        expected,
        Step::SoftwareReset {
            instruction: None,
            ..
        }
    ));
    let stats = run_bridge(&mut candidate, 1).unwrap();
    assert_eq!(stats.executed, 0);
    assert_eq!(
        stats.stopped,
        Some(expected),
        "pending reset must not silently return success"
    );
    assert!(!candidate.bus.devices().unwrap().reset.requested());
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

fn run_bridge(m: &mut Machine, budget: u64) -> Result<helios::jit::NativeRunStats, SchedulerError> {
    m.set_native_execution_enabled(true);
    let before = m.native_execution_count();
    let mut stats = helios::jit::NativeRunStats::default();
    while stats.executed < budget {
        match m.step() {
            Ok(Step::Executed { .. }) => stats.executed += 1,
            Ok(step) => {
                stats.stopped = Some(step);
                break;
            }
            Err(fault) => {
                return Err(SchedulerError::Cpu {
                    processor: 0,
                    fault,
                })
            }
        }
    }
    stats.native_instructions = m.native_execution_count().wrapping_sub(before);
    Ok(stats)
}
