#![cfg(all(feature = "jit-native", unix, target_arch = "x86_64"))]

use helios_jit_native::{compile, compile_span, State, MAX_BLOCK_INSNS};

const NOP: u32 = 0x0100_0000;
const BA_A: u32 = 0x3080_0004;
const INC_G2: u32 = 0x8400_a001;

#[test]
fn interior_annulled_branch_is_rejected_by_the_span_compiler() {
    if let Some(block) = compile_span(&[NOP, BA_A, INC_G2]) {
        let mut state = State::default();
        block.execute(&mut state);
        panic!(
            "accepted interior BA,a; annulled g2 became {}",
            state.regs[2]
        );
    }
}
#[test]
fn compiler_enforces_terminal_only_span_contract() {
    assert!(compile(&[NOP, BA_A]).is_none());
    assert!(compile_span(&[BA_A, BA_A]).is_none());
    assert!(compile_span(&[NOP; MAX_BLOCK_INSNS + 1]).is_none());
}

#[test]
fn emitted_exit_selects_pc_pair_for_every_icc_and_condition() {
    use helios_jit_native::BranchExit;
    for displacement in [0, 4, 0x007f_fffc, 0xff80_0000, 0xffff_fffc_u32] {
        for condition in 0..16u32 {
            for annul in [false, true] {
                let branch = 0x0080_0000
                    | (condition << 25)
                    | (u32::from(annul) << 29)
                    | ((displacement >> 2) & 0x3f_ffff);
                let block = compile_span(&[NOP, branch]).unwrap();
                for bits in 0..16u32 {
                    let (n, z, v, c) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
                    let less = n != v;
                    let taken = [
                        false,
                        z,
                        z || less,
                        less,
                        c || z,
                        c,
                        n,
                        v,
                        true,
                        !z,
                        !z && !less,
                        !less,
                        !c && !z,
                        !c,
                        !n,
                        !v,
                    ][condition as usize];
                    let dest = if taken { displacement } else { 8 };
                    let (pc_delta, npc_delta) = if annul && (!taken || condition == 8) {
                        (dest, dest.wrapping_add(4))
                    } else {
                        (4, dest)
                    };
                    let mut state = State {
                        icc: bits,
                        ..State::default()
                    };
                    assert_eq!(
                        block.execute(&mut state),
                        Some(BranchExit {
                            pc_delta,
                            npc_delta
                        }),
                        "condition={condition} annul={annul} ICC={bits} disp={displacement:x}"
                    );
                    assert_eq!(state.icc, bits);
                }
            }
        }
    }
}

#[test]
fn emitted_exit_uses_icc_written_inside_its_own_call() {
    use helios_jit_native::BranchExit;
    let block = compile_span(&[0x80a0_6001, 0x12bf_ffff]).unwrap();
    for (g1, bits, npc_delta) in [(1, 0, 8), (2, 2, u32::MAX - 3)] {
        let mut state = State {
            icc: bits,
            ..State::default()
        };
        state.regs[1] = g1;
        assert_eq!(
            block.execute(&mut state),
            Some(BranchExit {
                pc_delta: 4,
                npc_delta
            })
        );
    }
}

#[test]
fn single_word_and_straight_line_contracts() {
    assert!(compile(&[BA_A]).is_none());
    assert!(compile_span(&[BA_A]).is_none());
    assert!(compile_span(&[]).is_none());
    let mut state = State::default();
    assert_eq!(compile(&[INC_G2]).unwrap().execute(&mut state), None);
    assert_eq!(state.regs[2], 1);
}

#[test]
fn slot_compiler_accepts_exactly_one_alu_slot_after_a_supported_branch() {
    use helios_jit_native::compile_span_with_slot;
    let branch = 0x12bf_ffff;
    for invalid in [
        &[][..],
        &[branch, INC_G2],
        &[NOP, NOP, INC_G2],
        &[NOP, BA_A, INC_G2],
        &[NOP, 0x2080_0004, INC_G2],
        &[NOP, branch, BA_A],
        &[NOP, branch, 0xc200_8000],
        &[NOP, branch, INC_G2, NOP],
        &[NOP; MAX_BLOCK_INSNS + 1],
    ] {
        assert!(compile_span_with_slot(invalid).is_none(), "{invalid:x?}");
    }
    let words = [NOP, branch, INC_G2];
    assert!(compile_span(&words).is_none());
    assert!(compile(&words).is_none());
    assert!(compile_span_with_slot(&words).unwrap().has_delay_slot());
}

#[test]
fn native_slot_changes_icc_only_when_executed_and_returns_the_pre_slot_exit() {
    use helios_jit_native::{compile_span_with_slot, BranchExit};
    // Read Z for BE,a, then have the slot overwrite both Z and its destination.
    let words = [NOP, 0x22bf_ffff, 0x8a80_2001]; // ADDcc %g0,1,%g5
    let block = compile_span_with_slot(&words).unwrap();
    for (bits, expected_exit, expected_icc, expected_reg) in [
        (
            2,
            BranchExit {
                pc_delta: 4,
                npc_delta: 0xffff_fffc,
            },
            0,
            1,
        ),
        (
            9,
            BranchExit {
                pc_delta: 8,
                npc_delta: 12,
            },
            9,
            0xdead_beef,
        ),
    ] {
        let mut s = State {
            icc: bits,
            ..State::default()
        };
        s.regs[5] = 0xdead_beef;
        assert_eq!(block.execute(&mut s), Some(expected_exit));
        assert_eq!((s.icc, s.regs[5]), (expected_icc, expected_reg));
    }
}
