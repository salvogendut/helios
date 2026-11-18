#![cfg(feature = "jit")]

use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{Machine, SchedulerError};

fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn bare(words: &[u32]) -> Machine {
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(0, &image(words)).unwrap();
    machine
}

fn step_budget(machine: &mut Machine, budget: u64) {
    for _ in 0..budget {
        assert!(matches!(machine.step().unwrap(), Step::Executed { .. }));
    }
}

fn run_jit_budget(machine: &mut Machine, budget: u64) {
    let stats = machine.run_jit(budget).unwrap();
    assert_eq!(stats.executed, budget, "{stats:?}");
    assert_eq!(stats.stopped, None, "{stats:?}");
}

#[test]
fn supported_sethi_and_alu_match_expected_results() {
    let words = [
        0x0300_0123, // sethi %hi(0x48c00), %g1
        0x8210_6000, // or %g1, 0, %g1
        0x8400_6001, // add %g1, 1, %g2
        0x8600_200f, // add %g3, 15, %g3
        0x88a0_6001, // subcc %g1, 1, %g4
        0x0100_0000, // nop
    ];
    let mut interpreter = bare(&words);
    let mut jit = bare(&words);
    step_budget(&mut interpreter, 6);
    run_jit_budget(&mut jit, 6);
    assert_eq!(jit.cpu.read_reg(1), 0x48c00);
    assert_eq!(jit.cpu.read_reg(2), 0x48c01);
    assert_eq!(jit.cpu.read_reg(3), 15);
    assert_eq!(jit.cpu.read_reg(4), 0x48bff);
    assert_eq!(interpreter.cpu.read_reg(2), jit.cpu.read_reg(2));
    assert_eq!(interpreter.cpu.icc(), jit.cpu.icc());
    assert_eq!(
        (interpreter.cpu.pc(), interpreter.cpu.npc()),
        (jit.cpu.pc(), jit.cpu.npc())
    );
    assert_eq!(interpreter.cpu.psr(), jit.cpu.psr());
}

#[test]
fn differential_snapshot_matches_for_supported_block() {
    let words = [
        0x0300_0001, // sethi 0x40000, %g1
        0x8400_6001, // add %g1, 1, %g2
        0x8600_8002, // add %g2, %g2, %g3
        0x8800_c003, // add %g3, %g3, %g4
        0x8a00_a005, // add %g4, 5, %g5
        0x8c10_6000, // or %g1, 0, %g6
        0x8e00_e007, // add %g6, 7, %g7
    ];
    let mut interpreter = bare(&words);
    let mut jit = bare(&words);
    step_budget(&mut interpreter, 7);
    run_jit_budget(&mut jit, 7);
    assert_eq!(
        interpreter
            .diagnostic_snapshot()
            .differences(&jit.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

#[test]
fn unsupported_branch_falls_back_and_preserves_state() {
    let words = [
        0x0300_0002, // sethi
        0x8400_6001, // add %g1, 1, %g2
        0x1080_0003, // ba target
        0x8600_6003, // add %g3, 3, %g3
        0x0100_0000, // nop
        0x8800_6004, // target: add %g4, 4, %g4
    ];
    let mut interpreter = bare(&words);
    let mut jit = bare(&words);
    step_budget(&mut interpreter, 5);
    run_jit_budget(&mut jit, 5);
    assert_eq!(jit.cpu.read_reg(4), 0x804);
    assert_eq!((jit.cpu.pc(), jit.cpu.npc()), (24, 28));
    assert_eq!(
        interpreter
            .diagnostic_snapshot()
            .differences(&jit.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

fn sethi(rd: u32, value: u32) -> u32 {
    0x0100_0000 | (rd << 25) | ((value >> 10) & 0x3f_ffff)
}

fn or_imm(rd: u32, rs1: u32, value: u32) -> u32 {
    0x8000_0000 | (rd << 25) | (2 << 19) | (rs1 << 14) | 0x2000 | (value & 0x1fff)
}

#[test]
fn guest_store_invalidates_cached_code_before_target_execution() {
    let words = [
        sethi(1, 0),
        or_imm(1, 1, 0x28),
        sethi(2, 0x8600_200a),
        or_imm(2, 2, 0x0a),
        0xc420_6000, // st %g2, [%g1]
        0x1080_0005, // ba target (PC 0x28)
        0x0100_0000, // delay slot
        0x0100_0000, // target is overwritten by the store before execution
    ];
    let mut interpreter = bare(&words);
    let mut jit = bare(&words);
    step_budget(&mut interpreter, 8);
    run_jit_budget(&mut jit, 8);
    assert_eq!(interpreter.cpu.read_reg(3), 10);
    assert_eq!(jit.cpu.read_reg(3), 10);
    assert_eq!(
        interpreter
            .diagnostic_snapshot()
            .differences(&jit.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

fn mapped_machine(physical: u64, pte: u32, words: &[u32]) -> Machine {
    use helios::mmu::{Width, PCR_EN};
    const VA: u32 = 0x1234_5000;
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(physical as u32, &image(words)).unwrap();
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, pte),
    ] {
        machine.bus.write32(address, word).unwrap();
    }
    machine
        .cpu
        .mmu
        .store(&mut machine.bus, 1 << 8, 4, Width::Word, 0x100)
        .unwrap();
    machine
        .cpu
        .mmu
        .store(&mut machine.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    machine.cpu.set_pc_npc(VA, VA + 4);
    machine
}

#[test]
fn mapped_fetch_uses_the_current_translation() {
    for (physical, pte, base) in [(0x8000, 0x0000_088e, 0), (0x9000, 0x0000_098e, 0x1000)] {
        let words = [sethi(1, 0x1000 + base), 0x8400_6001, 0x8600_6002];
        let mut machine = mapped_machine(physical, pte, &words);
        let stats = machine.run_jit(3).unwrap();
        assert_eq!(stats.executed, 3, "{stats:?}");
        assert_eq!(stats.stopped, None, "{stats:?}");
        assert_eq!(machine.cpu.read_reg(2), base + 0x1001);
        assert_eq!(machine.cpu.read_reg(3), base + 0x1002);
    }
}

#[test]
fn removed_fetch_permission_falls_back_to_the_interpreter_fault() {
    let words = [0x0300_0004, 0x8400_6001];
    let mut machine = mapped_machine(0x8000, 0x0000_0882, &words);
    let error = machine.run_jit(2).unwrap_err();
    assert!(error.to_string().contains("InstructionMmu"), "{error}");
}
#[test]
fn device_machine_uses_interpreter_fallback() {
    use helios::machine::MachineProfile;
    let words = [0x0300_0003, 0x8400_6001];
    let build = || {
        let mut machine = Machine::with_profile(
            MachineProfile::Ss5,
            16 * 1024 * 1024,
            &image(&words),
            MachineProfile::Ss5.default_nvram(),
        )
        .unwrap();
        machine.cpu.set_pc_npc(0, 4);
        machine
    };
    let mut interpreter = build();
    let mut jit = build();
    step_budget(&mut interpreter, 2);
    let stats = jit
        .run_jit(2)
        .map_err(|error: SchedulerError| error.to_string());
    let stats = stats.unwrap();
    assert_eq!(stats.executed, 2);
    assert_eq!(stats.compiled_blocks, 0);
    assert_eq!(interpreter.cpu.read_reg(2), jit.cpu.read_reg(2));
}
