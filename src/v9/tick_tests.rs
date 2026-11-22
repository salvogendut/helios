use super::*;
use crate::ultra5::{
    BoardStep, BootFlash, ResetNvram, Ultra5ResetBoard, FLASH_SIZE, RESET_NVRAM_SIZE,
};

const READ_TICK: u32 = 0x8541_0000; // rd %tick,%g2
const READ_PR_TICK: u32 = 0x8751_0000; // rdpr %tick,%g3
const WRITE_TICK: u32 = 0x8991_2000; // wrpr %g4,0,%tick
const READ_COMPARE: u32 = 0x8545_c000; // rd %tick_cmpr,%g2
const WRITE_COMPARE: u32 = 0xaf81_2000; // wr %g4,0,%tick_cmpr
const NOP: u32 = 0x0100_0000;

fn program(words: &[u32]) -> V9Memory {
    let mut memory = V9Memory::new(4 * words.len());
    memory.load_words(0, words);
    memory
}

#[test]
fn counter_aliases_share_the_reset_value_and_guest_writes() {
    let reset = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    assert_eq!(
        (reset.tick, reset.tick_compare, reset.softint),
        (1 << 63, 1 << 63, 0)
    );
    let mut cpu = V9Cpu::new(0);
    cpu.pstate = PSTATE_PRIV;
    cpu.write_reg(4, 1000);
    let mut memory = program(&[READ_TICK, READ_PR_TICK, WRITE_TICK, READ_TICK]);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.read_reg(2), 1 << 63);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.read_reg(3), (1 << 63) + 44);
    cpu.step(&mut memory).unwrap();
    cpu.pstate = 0;
    cpu.step(&mut memory).unwrap();
    assert_eq!((cpu.read_reg(2), cpu.tick), (1044, 1088));
}

#[test]
fn rejected_clock_accesses_leave_the_counter_and_cpu_unchanged() {
    for (pstate, npt, word) in [
        (0, true, READ_TICK),
        (0, true, READ_TICK & !(31 << 25)),
        (0, false, READ_PR_TICK),
        (0, false, WRITE_TICK),
        (0, false, READ_COMPARE),
        (0, false, WRITE_COMPARE),
        (PSTATE_PRIV, false, READ_TICK | 1),
        (PSTATE_PRIV, false, READ_PR_TICK | 1),
        (PSTATE_PRIV, false, READ_COMPARE | 1),
        (PSTATE_PRIV, false, 0xaf81_0020),
    ] {
        let mut cpu = V9Cpu::new(0);
        cpu.pstate = pstate;
        cpu.tick = 12 | (u64::from(npt) << 63);
        cpu.tick_compare = 15;
        let before = cpu.clone();
        assert!(cpu.step(&mut program(&[word])).is_err(), "word={word:08x}");
        assert_eq!(cpu, before);
    }
}

#[test]
fn compare_latches_inside_a_step_at_the_endpoint_and_across_wrap() {
    for npt in [0, 1 << 63] {
        for (start, compare, count, pending) in [
            (100, 101, 144, true),
            (100, 144, 144, true),
            (100, 145, 144, false),
            ((1 << 63) - 5, 1, 39, true),
            ((1 << 63) - 5, (1 << 63) | 1, 39, false),
        ] {
            let mut cpu = V9Cpu::new(0);
            cpu.tick = npt | start;
            cpu.tick_compare = compare;
            cpu.step(&mut program(&[NOP])).unwrap();
            assert_eq!((cpu.tick, cpu.softint), (npt | count, u16::from(pending)));
        }
    }
}

#[test]
fn reprogramming_and_acknowledging_the_comparator_preserve_other_requests() {
    let mut cpu = V9Cpu::new(0);
    cpu.pstate = PSTATE_PRIV;
    cpu.tick = 100;
    cpu.tick_compare = 500;
    cpu.softint = 0x80;
    let mut memory = program(&[
        WRITE_TICK,
        WRITE_COMPARE,
        WRITE_COMPARE,
        READ_COMPARE,
        0xa980_2004, // wr %g0,4,%set_softint
        0xab80_2001, // wr %g0,1,%clear_softint
        0xad80_2002, // wr %g0,2,%softint
        0x8545_8000, // rd %softint,%g2
        WRITE_COMPARE,
        WRITE_TICK,
    ]);
    cpu.write_reg(4, 1000);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.softint, 0x80); // The counter jumped over 500.
    cpu.write_reg(4, cpu.tick);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.softint, 0x81);
    cpu.write_reg(4, (1 << 63) | 7);
    cpu.step(&mut memory).unwrap();
    cpu.step(&mut memory).unwrap();
    assert_eq!((cpu.read_reg(2), cpu.softint), ((1 << 63) | 7, 0x81));
    for expected in [0x85, 0x84, 2] {
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.softint, expected);
    }
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.read_reg(2), 2);
    cpu.write_reg(4, 4000);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.softint, 2);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.softint, 3); // Writing TICK itself can establish equality.
}

#[test]
fn interrupt_selection_respects_ie_pil_and_both_sources_of_level_14() {
    let mut cpu = V9Cpu::new(0);
    for (requests, pil, level) in [
        (1, 13, Some(14)),
        (1, 14, None),
        (1 << 14, 13, Some(14)),
        (1 | (1 << 15), 14, Some(15)),
        (1 << 15, 15, None),
        (1 << 3, 2, Some(3)),
        (1 << 3, 3, None),
    ] {
        cpu.softint = requests;
        cpu.pil = pil;
        for (pstate, expected) in [(PSTATE_PRIV, None), (PSTATE_PRIV | PSTATE_IE, level)] {
            cpu.pstate = pstate;
            match cpu.instruction_address() {
                Ok(_) => assert_eq!(expected, None),
                Err(CpuFault::Interrupt(actual)) => assert_eq!(Some(actual), expected),
                other => panic!("unexpected fetch result {other:?}"),
            }
        }
    }
}

#[test]
fn tick_interrupt_in_delay_slot_saves_both_pcs_and_retries_after_ack() {
    const RAM: u64 = 0x1000_0000;
    let mut board = Ultra5ResetBoard::new(
        BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
        ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
    );
    let mut cpu = V9Cpu::new(0x2020);
    cpu.pstate = PSTATE_PRIV;
    cpu.write_pr(6, 6).unwrap(); // WRPR PSTATE permits PRIV|IE
    cpu.lsu_control = LSU_IM;
    cpu.tba = 0x8000;
    cpu.pil = 3;
    cpu.tick = 0;
    cpu.tick_compare = 7;
    cpu.ccr = 0xa5;
    cpu.cwp = 3;
    cpu.write_reg(1, 0x45);
    for (slot, va, pa) in [(0, 0x2000, RAM), (1, 0x8000, RAM + 0x2000)] {
        cpu.instruction_tlb.stage_tag(va);
        cpu.instruction_tlb
            .write_data(slot, (1 << 63) | pa | 0x20, true);
    }
    // ba +32; add %g1,1,%g1. Handler clears SOFTINT[0], then retries.
    board
        .write_cacheable_xword(RAM + 0x20, 0x1080_0008_8200_6001)
        .unwrap();
    board
        .write_cacheable_xword(RAM + 0x29c0, 0xab80_2001_83f0_0000)
        .unwrap();
    board.step_mapped(&mut cpu).unwrap();
    assert_eq!(
        (cpu.pc, cpu.npc, cpu.tick, cpu.softint),
        (0x2024, 0x2040, 44, 1)
    );
    let before = cpu.clone();
    assert!(board
        .step_mapped(&mut cpu)
        .unwrap_err()
        .contains("interrupt"));
    assert_eq!(cpu, before);
    assert_eq!(
        board.step_outcome(&mut cpu).unwrap(),
        BoardStep::InterruptTaken {
            pc: 0x2024,
            level: 14,
            vector: 0x89c0
        }
    );
    let frame = cpu.traps[0];
    assert_eq!(
        (frame.tpc, frame.tnpc, frame.tt, frame.tstate),
        (0x2024, 0x2040, 0x4e, (0xa5 << 32) | (6 << 8) | 3)
    );
    assert_eq!(
        (
            cpu.tl,
            cpu.pil,
            cpu.tick,
            cpu.softint,
            cpu.pstate,
            cpu.read_reg(1)
        ),
        (1, 3, 88, 1, PSTATE_PRIV | PSTATE_PEF | PSTATE_AG, 0)
    );
    board.step_mapped(&mut cpu).unwrap();
    assert_eq!(cpu.softint, 0);
    board.step_mapped(&mut cpu).unwrap(); // retry
    assert_eq!(
        (cpu.pc, cpu.npc, cpu.pstate, cpu.read_reg(1)),
        (0x2024, 0x2040, 6, 0x45)
    );
    board.step_mapped(&mut cpu).unwrap();
    assert_eq!(
        (cpu.pc, cpu.npc, cpu.tick, cpu.read_reg(1)),
        (0x2040, 0x2044, 220, 0x46)
    );
}
