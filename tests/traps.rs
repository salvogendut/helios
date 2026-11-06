use helios::bus::Bus;
use helios::cpu::{Cpu, FaultKind, Step, PSR_EF, PSR_ET, PSR_PIL, PSR_PS, PSR_S, PSR_VERSION};
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 20;

fn op(op: u32, op3: u32, rd: u8, rs1: u8, operand: u32, immediate: bool) -> u32 {
    (op << 30)
        | (op3 << 19)
        | (u32::from(rd) << 25)
        | (u32::from(rs1) << 14)
        | (u32::from(immediate) << 13)
        | (operand & if immediate { 0x1fff } else { 31 })
}

fn alu(op3: u32, rd: u8, rs1: u8, immediate: i32) -> u32 {
    op(2, op3, rd, rs1, immediate as u32, true)
}

fn put(m: &mut Machine, address: u32, words: &[u32]) {
    m.load_image(
        address,
        &words
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
}

/// Set privileged state through guest instructions, not a host back door.
fn machine(program: &[u32], psr: u32) -> Machine {
    let mut m = Machine::bare_metal(0x4000, 0).unwrap();
    put(
        &mut m,
        0,
        &[alu(0x33, 0, 1, 0), alu(0x31, 0, 2, 0), NOP, NOP, NOP],
    );
    put(&mut m, START, program);
    m.cpu.write_reg(1, 0x1000);
    m.cpu.write_reg(2, psr);
    for _ in 0..5 {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
    m
}

fn assert_trap(m: &mut Machine, expected_type: u8) {
    let pc = m.cpu.pc();
    let npc = m.cpu.npc();
    let cwp = m.cpu.current_window();
    let psr = m.cpu.psr();
    let vector = (m.cpu.tbr() & 0xffff_f000) | (u32::from(expected_type) << 4);
    match m.step().unwrap() {
        Step::TrapTaken {
            fault,
            trap_type,
            vector: actual_vector,
        } => {
            assert_eq!((fault.pc, fault.npc), (pc, npc));
            assert_eq!((trap_type, actual_vector), (expected_type, vector));
        }
        other => panic!("expected a trap, got {other:?}"),
    }
    assert_eq!(
        (m.cpu.pc(), m.cpu.npc(), m.cpu.tbr()),
        (vector, vector + 4, vector)
    );
    assert_eq!(m.cpu.current_window(), (cwp + 7) % 8);
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (pc, npc));
    assert_eq!(
        m.cpu.psr() & (PSR_ET | PSR_S | PSR_PS),
        PSR_S | if psr & PSR_S != 0 { PSR_PS } else { 0 }
    );
    assert_eq!(m.cpu.psr() & 0x00f0_1f00, psr & 0x00f0_1f00);
    assert_eq!(m.cpu.error_state(), None);
}

#[test]
fn psr_masks_reserved_and_identity_bits_and_selects_window() {
    let mut m = machine(&[alu(0x29, 3, 0, 0)], 0xfff0_ffe7);
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(3), 0x04f0_1fe7);
    assert_eq!(m.cpu.psr(), 0x04f0_1fe7);
    assert_eq!(m.cpu.current_window(), 7);
    assert!(m.cpu.icc().n && m.cpu.icc().z && m.cpu.icc().v && m.cpu.icc().c);
    assert_eq!(m.cpu.psr() & (PSR_EF | PSR_PIL), PSR_EF | PSR_PIL);
}

#[test]
fn writes_xor_operands_and_wim_retains_the_eighth_window_bit() {
    let mut m = machine(
        &[
            op(2, 0x32, 0, 3, 4, false),
            NOP,
            NOP,
            NOP,
            alu(0x2a, 5, 0, 0),
            alu(0x30, 0, 3, -1),
            NOP,
            NOP,
            NOP,
            alu(0x28, 6, 0, 0),
        ],
        PSR_S | PSR_ET,
    );
    m.cpu.write_reg(3, 0xffff_ff00);
    m.cpu.write_reg(4, 0x80);
    for _ in 0..10 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.window_invalid_mask(), 0x80);
    assert_eq!(m.cpu.read_reg(5), 0x80);
    assert_eq!(m.cpu.y(), 0xff);
    assert_eq!(m.cpu.read_reg(6), 0xff);
}

#[test]
fn user_mode_can_access_y_and_execute_stbar() {
    let mut m = machine(
        &[
            alu(0x30, 0, 0, -1),
            NOP,
            NOP,
            NOP,
            alu(0x28, 3, 0, 0),
            0x8143_c000,
        ],
        PSR_ET,
    );
    for _ in 0..6 {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
    assert_eq!(m.cpu.read_reg(3), u32::MAX);
    assert_eq!(m.cpu.psr(), PSR_VERSION | PSR_ET);
}

#[test]
fn privileged_reads_to_g0_and_writes_trap_in_user_mode() {
    for instruction in [
        alu(0x29, 0, 0, 0),
        alu(0x2a, 0, 0, 0),
        alu(0x2b, 0, 0, 0),
        alu(0x31, 0, 0, 8),
        alu(0x32, 0, 0, 255),
        alu(0x33, 0, 0, -1),
    ] {
        let mut m = machine(&[instruction], PSR_ET);
        assert_trap(&mut m, 3);
        assert_eq!(m.cpu.window_invalid_mask(), 0);
        assert_eq!(m.cpu.tbr(), 0x1030);
    }
}

#[test]
fn invalid_psr_window_traps_without_partial_write() {
    let mut m = machine(&[alu(0x31, 0, 3, 0)], PSR_S | PSR_ET | PSR_EF | 0x0090_0000);
    m.cpu.write_reg(3, 8); // invalid CWP, and attempts to clear all other fields
    assert_trap(&mut m, 2);
    assert_eq!(m.cpu.psr() & (PSR_EF | 0x00f0_0000), PSR_EF | 0x0090_0000);
}

#[test]
fn unimp_is_guest_illegal_but_missing_implementation_is_a_host_stop() {
    let mut m = machine(&[0], PSR_S | PSR_ET);
    assert_trap(&mut m, 2);
    for instruction in [0x8070_0000, alu(0x28, 3, 16, 0), alu(0x30, 16, 0, 0)] {
        let mut m = machine(&[instruction], PSR_S | PSR_ET);
        let before = m.cpu.clone();
        assert_eq!(
            m.step().unwrap_err().kind,
            FaultKind::UnsupportedInstruction
        );
        assert_eq!(m.cpu, before);
        assert_eq!(m.cpu.error_state(), None);
    }
}

#[test]
fn tbr_write_changes_only_base_and_rd_reads_last_trap_type() {
    let mut m = machine(&[alu(0x3a, 8, 0, 7)], PSR_ET);
    assert_trap(&mut m, 0x87);
    m.cpu.write_reg(4, 0x2fff);
    put(
        &mut m,
        0x1870,
        &[alu(0x33, 0, 4, 0), NOP, NOP, NOP, alu(0x2b, 5, 0, 0)],
    );
    for _ in 0..5 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.tbr(), 0x2870);
    assert_eq!(m.cpu.read_reg(5), 0x2870);
}

#[test]
fn conditional_software_traps_use_icc_and_low_seven_sum_bits() {
    let cases: &[(u32, &[u32])] = &[
        (0, &[8, 9, 10, 11, 12, 13, 14, 15]),
        (0x0040_0000, &[1, 2, 4, 8, 11, 13, 14, 15]),
        (0x0090_0000, &[2, 3, 4, 5, 6, 8, 9, 15]),
        (0x0020_0000, &[2, 3, 7, 8, 9, 12, 13, 14]),
    ];
    for &(icc, conditions) in cases {
        for condition in 0..16 {
            let mut m = machine(&[alu(0x3a, condition as u8, 3, -1)], PSR_ET | icc);
            m.cpu.write_reg(3, 0x185);
            if conditions.contains(&condition) {
                assert_trap(&mut m, 0x84);
            } else {
                assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
                assert_eq!((m.cpu.pc(), m.cpu.npc()), (START + 4, START + 8));
            }
        }
    }
    let mut m = machine(&[op(2, 0x3a, 8, 3, 4, false)], PSR_ET);
    m.cpu.write_reg(3, u32::MAX);
    m.cpu.write_reg(4, 3);
    assert_trap(&mut m, 0x82);
}

#[test]
fn window_fault_traps_rotate_even_into_a_window_marked_invalid() {
    for (instruction, mask, tt) in [
        (alu(0x3c, 16, 0, 99), 0x80, 5),
        (alu(0x3d, 16, 0, 99), 2, 6),
    ] {
        let mut m = machine(
            &[alu(0x32, 0, 0, mask), NOP, NOP, NOP, instruction],
            PSR_S | PSR_ET,
        );
        m.cpu.write_reg(16, 42);
        for _ in 0..4 {
            m.step().unwrap();
        }
        assert_trap(&mut m, tt);
        assert_eq!(m.cpu.read_reg(16), 0); // failed SAVE/RESTORE did not write its result
        assert_eq!(m.cpu.window_invalid_mask(), mask as u8);
    }
}

#[test]
fn bus_and_alignment_faults_vector_without_load_or_store_side_effects() {
    for (op3, address, tt) in [
        (0, 0x8000, 0x29),
        (4, 0x8000, 0x29),
        (0, 0x101, 7),
        (6, 0x101, 7),
    ] {
        let mut m = machine(&[op(3, op3, 4, 3, 0, true)], PSR_ET);
        m.cpu.write_reg(3, address);
        m.cpu.write_reg(4, 0xdead_beef);
        m.bus.write32(0x100, 0x1234_5678).unwrap();
        assert_trap(&mut m, tt);
        assert_eq!(m.cpu.read_reg(4), 0xdead_beef);
        assert_eq!(m.bus.read32(0x100).unwrap(), 0x1234_5678);
    }
    let mut m = machine(&[op(3, 4, 4, 3, 0, true)], PSR_ET);
    m.bus.map_rom(0x8000, &[1, 2, 3, 4]).unwrap();
    m.cpu.write_reg(3, 0x8000);
    assert_trap(&mut m, 0x29);
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0x0102_0304);
}

#[test]
fn instruction_bus_error_has_no_instruction_word() {
    let mut m = machine(&[alu(0x38, 0, 3, 0), NOP], PSR_ET);
    m.cpu.write_reg(3, 0x8000);
    m.step().unwrap();
    m.step().unwrap();
    match m.step().unwrap() {
        Step::TrapTaken {
            fault,
            trap_type,
            vector,
        } => {
            assert_eq!(trap_type, 0x21);
            assert_eq!(vector, 0x1210);
            assert_eq!(fault.instruction, None);
            assert_eq!((fault.pc, fault.npc), (0x8000, 0x8004));
        }
        other => panic!("expected fetch trap, got {other:?}"),
    }
}

#[test]
fn rett_returns_to_user_mode_and_resumes_a_faulting_branch_delay_slot() {
    let branch = 0x1080_0000 | ((0x80 - START) / 4);
    let mut m = machine(&[branch, op(3, 0, 4, 3, 0, true)], PSR_ET);
    m.cpu.write_reg(3, 0x101);
    m.bus.write32(0x104, 0xdead_beef).unwrap();
    m.step().unwrap();
    assert_trap(&mut m, 7);
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (START + 4, 0x80));
    put(
        &mut m,
        0x1070,
        &[alu(0, 3, 3, 3), alu(0x38, 0, 17, 0), alu(0x39, 0, 18, 0)],
    );
    for _ in 0..3 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.psr() & (PSR_S | PSR_ET | PSR_PS), PSR_ET);
    assert_eq!(m.cpu.current_window(), 0);
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (START + 4, 0x80));
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(4), 0xdead_beef);
    assert_eq!(m.cpu.pc(), 0x80);
}

#[test]
fn rett_with_traps_enabled_selects_privilege_before_illegal_instruction() {
    for (psr, tt) in [(PSR_ET, 3), (PSR_S | PSR_ET, 2)] {
        let mut m = machine(&[alu(0x39, 0, 0, 1)], psr);
        assert_trap(&mut m, tt);
    }
}

#[test]
fn failed_rett_updates_tt_in_error_mode_with_defined_priority() {
    for (psr, wim, tt) in [(0, 2, 3), (PSR_S, 2, 6), (PSR_S, 0, 7)] {
        let mut m = machine(&[alu(0x39, 0, 0, 1)], psr);
        m.cpu.set_window_invalid_mask(wim);
        let before = m.cpu.clone();
        let fault = m.step().unwrap_err();
        assert_eq!(fault.kind.trap_type(), Some(tt));
        assert_eq!(m.cpu.error_state(), Some(fault));
        assert_eq!(m.cpu.psr(), before.psr());
        assert_eq!((m.cpu.pc(), m.cpu.npc()), (before.pc(), before.npc()));
        assert_eq!(m.cpu.tbr(), 0x1000 | (u32::from(tt) << 4));
        for r in 0..32 {
            assert_eq!(m.cpu.read_reg(r), before.read_reg(r));
        }
    }
}

#[test]
fn nested_fault_with_et_clear_latches_error_without_another_trap_entry() {
    let mut m = machine(&[alu(0x3a, 8, 0, 7)], PSR_S | PSR_ET);
    assert_trap(&mut m, 0x87);
    let before = m.cpu.clone();
    let fault = m.step().unwrap_err(); // zero/UNIMP at handler
    assert_eq!(fault.kind, FaultKind::IllegalInstruction);
    assert_eq!(m.cpu.error_state(), Some(fault));
    assert_eq!(m.cpu.tbr(), 0x1870); // original TT retained on non-RETT error
    assert_eq!(m.cpu.psr(), before.psr());
    for r in 0..32 {
        assert_eq!(m.cpu.read_reg(r), before.read_reg(r));
    }
    put(&mut m, 0x1870, &[NOP]);
    let stopped = m.cpu.clone();
    assert_eq!(m.step().unwrap_err(), fault);
    assert_eq!(m.cpu, stopped); // even repaired code cannot execute in error mode
    m.cpu = Cpu::new(0x1870);
    assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
}
