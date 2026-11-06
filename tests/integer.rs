use helios::cpu::{FaultKind, Icc, Step, PSR_ET, PSR_S};
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 20;

fn alu(op3: u32, rd: u32, rs1: u32, operand: u32, immediate: bool) -> u32 {
    0x8000_0000
        | (rd << 25)
        | (op3 << 19)
        | (rs1 << 14)
        | if immediate {
            0x2000 | (operand & 0x1fff)
        } else {
            operand & 31
        }
}

fn setup(program: &[u32], y: u32, flags: u32, mode: u32) -> Machine {
    let mut m = Machine::bare_metal(4096, 0).unwrap();
    let words: Vec<_> = [
        alu(0x30, 0, 7, 0, true),
        alu(0x31, 0, 6, 0, true),
        NOP,
        NOP,
        NOP,
    ]
    .into_iter()
    .chain(program.iter().copied())
    .collect();
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_be_bytes).collect();
    m.load_image(0, &bytes).unwrap();
    m.cpu.write_reg(7, y);
    m.cpu.write_reg(6, mode | (flags << 20));
    for _ in 0..5 {
        m.step().unwrap();
    }
    m
}

fn flags(n: bool, z: bool, v: bool, c: bool) -> Icc {
    Icc { n, z, v, c }
}

#[test]
fn multiply_sets_y_and_low_word_flags_for_signed_and_unsigned_products() {
    let cases = [
        (0xffff_ffff, 0xffff_ffff, 0xffff_fffe_0000_0001u64, 1u64),
        (0x8000_0000, 2, 0x0000_0001_0000_0000, 0xffff_ffff_0000_0000),
        (
            0x10000,
            0x10000,
            0x0000_0001_0000_0000,
            0x0000_0001_0000_0000,
        ),
        (
            0x8000_0000,
            0x8000_0000,
            0x4000_0000_0000_0000,
            0x4000_0000_0000_0000,
        ),
        (0xffff_ffff, 2, 0x0000_0001_ffff_fffe, 0xffff_ffff_ffff_fffe),
        (0, 0xffff_ffff, 0, 0),
    ];
    for (a, b, unsigned, signed) in cases {
        for op3 in [0x0a, 0x0b, 0x1a, 0x1b] {
            for rd in [0, 1, 3] {
                // g0 still updates Y/ICC; rd may alias rs1
                let expected = if op3 & 1 == 0 { unsigned } else { signed };
                let mut m = setup(&[alu(op3, rd, 1, 2, false)], 0x1234, 15, PSR_S);
                m.cpu.write_reg(1, a);
                m.cpu.write_reg(2, b);
                m.step().unwrap();
                assert_eq!(m.cpu.y(), (expected >> 32) as u32);
                assert_eq!(
                    m.cpu.read_reg(rd as u8),
                    if rd == 0 { 0 } else { expected as u32 }
                );
                assert_eq!(
                    m.cpu.icc(),
                    if op3 & 0x10 != 0 {
                        flags(
                            expected as u32 & 0x8000_0000 != 0,
                            expected as u32 == 0,
                            false,
                            false,
                        )
                    } else {
                        flags(true, true, true, true)
                    }
                );
            }
        }
    }
}

#[test]
fn multiply_and_divide_sign_extend_immediates_before_unsigned_interpretation() {
    for (op3, expected, y) in [
        (0x0a, 0xffff_fffe, 1),
        (0x0b, 0xffff_fffe, u32::MAX),
        (0x0e, 0, 0),
        (0x0f, 0xffff_fffe, 0),
    ] {
        let mut m = setup(&[alu(op3, 3, 1, 0x1fff, true)], 0, 0, PSR_S);
        m.cpu.write_reg(1, 2);
        m.step().unwrap();
        assert_eq!((m.cpu.read_reg(3), m.cpu.y()), (expected, y));
    }
}

#[test]
fn unsigned_division_uses_y_and_saturates_quotient() {
    for (dividend, divisor, expected, overflow) in [
        (0x0000_0001_0000_0000u64, 2, 0x8000_0000, false),
        (0xffff_ffff_ffff_ffff, 1, 0xffff_ffff, true),
        (0xffff_ffff_ffff_ffff, 0xffff_ffff, 0xffff_ffff, true),
        (0x0000_0000_ffff_ffff, 1, 0xffff_ffff, false),
        (19, 3, 6, false),
        (0, 7, 0, false),
    ] {
        for op3 in [0x0e, 0x1e] {
            let mut m = setup(
                &[alu(op3, 1, 1, 2, false)],
                (dividend >> 32) as u32,
                15,
                PSR_S,
            );
            m.cpu.write_reg(1, dividend as u32);
            m.cpu.write_reg(2, divisor);
            m.step().unwrap();
            assert_eq!(m.cpu.read_reg(1), expected);
            assert_eq!(m.cpu.y(), (dividend >> 32) as u32);
            assert_eq!(
                m.cpu.icc(),
                if op3 == 0x1e {
                    flags(expected >> 31 != 0, expected == 0, overflow, false)
                } else {
                    flags(true, true, true, true)
                }
            );
        }
    }
}

#[test]
fn signed_division_rounds_toward_zero_and_uses_microsparc_overflow_method_b() {
    for (dividend, divisor, expected, overflow) in [
        (-19i64, 3i32, -6i32, false),
        (19, -3, -6, false),
        (-19, -3, 6, false),
        (i64::MIN, -1, i32::MAX, true),
        (i64::MIN, 1, i32::MIN, true),
        (i64::MAX, 1, i32::MAX, true),
        (-4_294_967_296, 2, i32::MIN, false),
        (-4_294_967_297, 2, i32::MIN, true), // truncates to MIN, but nonzero remainder
        (4_294_967_297, -2, i32::MIN, true),
        (-4_294_967_295, 2, -2_147_483_647, false),
        (4_294_967_295, 2, i32::MAX, false),
        (4_294_967_296, 2, i32::MAX, true),
        (-1, 2, 0, false),
        (0, -1, 0, false),
    ] {
        for op3 in [0x0f, 0x1f] {
            let y = (dividend as u64 >> 32) as u32;
            let mut m = setup(&[alu(op3, 2, 1, 2, false)], y, 15, PSR_S);
            m.cpu.write_reg(1, dividend as u32);
            m.cpu.write_reg(2, divisor as u32);
            m.step().unwrap();
            assert_eq!(m.cpu.read_reg(2), expected as u32, "{dividend}/{divisor}");
            assert_eq!(m.cpu.y(), y);
            assert_eq!(
                m.cpu.icc(),
                if op3 == 0x1f {
                    flags(expected < 0, expected == 0, overflow, false)
                } else {
                    flags(true, true, true, true)
                },
                "{dividend}/{divisor}"
            );
        }
    }
}

#[test]
fn divide_by_zero_preserves_destination_y_and_icc_in_delay_slot() {
    for op3 in [0x0e, 0x0f, 0x1e, 0x1f] {
        for rd in [0, 3] {
            let mut m = setup(
                &[0x1080_0008, alu(op3, rd, 1, 0, true)],
                0x1234_5678,
                15,
                PSR_S | PSR_ET,
            );
            m.cpu.write_reg(1, 99);
            m.cpu.write_reg(3, 0xabcd);
            m.step().unwrap();
            let Step::TrapTaken {
                fault,
                trap_type,
                vector,
            } = m.step().unwrap()
            else {
                panic!("expected trap");
            };
            assert_eq!(fault.kind, FaultKind::DivisionByZero);
            assert_eq!((trap_type, vector), (0x2a, 0x2a0));
            assert_eq!((fault.pc, fault.npc), (START + 4, START + 32));
            assert_eq!(
                (m.cpu.read_reg(17), m.cpu.read_reg(18)),
                (START + 4, START + 32)
            );
            assert_eq!(m.cpu.read_reg(3), 0xabcd);
            assert_eq!(m.cpu.y(), 0x1234_5678);
            assert_eq!(m.cpu.icc(), flags(true, true, true, true));
        }
    }
    let mut m = setup(&[alu(0x1f, 3, 1, 0, true)], 9, 15, PSR_S);
    let error = m.step().unwrap_err();
    assert_eq!(error.kind, FaultKind::DivisionByZero);
    assert_eq!(m.cpu.error_state(), Some(error));
    assert_eq!(m.cpu.pc(), START);
    assert_eq!(m.cpu.y(), 9);
}

#[test]
fn tagged_arithmetic_checks_both_tags_and_arithmetic_overflow() {
    for (op3, a, b, result, expected) in [
        (0x20, 4, 8, 12, flags(false, false, false, false)),
        (0x20, 1, 4, 5, flags(false, false, true, false)),
        (0x20, 4, 2, 6, flags(false, false, true, false)),
        (
            0x20,
            0x7fff_fffc,
            4,
            0x8000_0000,
            flags(true, false, true, false),
        ),
        (0x20, 0xffff_fffc, 4, 0, flags(false, true, false, true)),
        (0x21, 4, 8, 0xffff_fffc, flags(true, false, false, true)),
        (0x21, 2, 2, 0, flags(false, true, true, false)),
        (
            0x21,
            0x8000_0000,
            4,
            0x7fff_fffc,
            flags(false, false, true, false),
        ),
        (
            0x21,
            0xffff_fffc,
            4,
            0xffff_fff8,
            flags(true, false, false, false),
        ),
    ] {
        for trap in [false, true] {
            let mut m = setup(
                &[alu(op3 + if trap { 2 } else { 0 }, 3, 1, 2, false)],
                9,
                15,
                PSR_S | PSR_ET,
            );
            m.cpu.write_reg(1, a);
            m.cpu.write_reg(2, b);
            m.cpu.write_reg(3, 0x1234);
            let step = m.step().unwrap();
            if trap && expected.v {
                assert!(
                    matches!(step, Step::TrapTaken { trap_type: 0x0a, fault, .. } if fault.kind == FaultKind::TagOverflow)
                );
                assert_eq!(m.cpu.read_reg(3), 0x1234);
                assert_eq!(m.cpu.icc(), flags(true, true, true, true));
            } else {
                assert!(matches!(step, Step::Executed { .. }));
                assert_eq!(m.cpu.read_reg(3), result);
                assert_eq!(m.cpu.icc(), expected);
            }
            assert_eq!(m.cpu.y(), 9);
        }
    }
}

#[test]
fn multiply_step_uses_old_y_and_n_xor_v_and_updates_even_for_g0() {
    for (a, b, y, old_flags, result, new_y, expected) in [
        (
            3,
            4,
            1,
            0,
            5,
            0x8000_0000,
            flags(false, false, false, false),
        ),
        (
            3,
            4,
            2,
            0,
            1,
            0x8000_0001,
            flags(false, false, false, false),
        ),
        (0, 0, 0, 8, 0x8000_0000, 0, flags(true, false, false, false)),
        (0, 0, 0, 10, 0, 0, flags(false, true, false, false)), // N=V=1
        (
            0xffff_fffe,
            1,
            1,
            0,
            0x8000_0000,
            0,
            flags(true, false, true, false),
        ),
        (
            0xffff_ffff,
            1,
            1,
            8,
            0,
            0x8000_0000,
            flags(false, true, false, true),
        ),
        (0, 0x8000_0000, 1, 8, 0, 0, flags(false, true, true, true)),
    ] {
        for rd in [0, 1] {
            let mut m = setup(&[alu(0x24, rd, 1, 2, false)], y, old_flags, PSR_S);
            m.cpu.write_reg(1, a);
            m.cpu.write_reg(2, b);
            m.step().unwrap();
            assert_eq!(m.cpu.read_reg(rd as u8), if rd == 0 { 0 } else { result });
            assert_eq!(m.cpu.y(), new_y);
            assert_eq!(m.cpu.icc(), expected);
        }
    }
}

#[test]
fn multiply_step_sequence_produces_a_full_64_bit_product() {
    let mut program = vec![alu(0x24, 3, 3, 2, false); 32];
    program.push(alu(0x24, 3, 3, 0, false)); // final shift, no add
    for multiplier in [0, 1, 7, 0x1234567, 0x7fff_ffff] {
        for multiplicand in [i32::MIN, -123456, -1, 0, 123456, i32::MAX] {
            let mut m = setup(&program, multiplier, 0, PSR_S);
            m.cpu.write_reg(2, multiplicand as u32);
            for _ in 0..33 {
                m.step().unwrap();
            }
            let actual = (u64::from(m.cpu.read_reg(3)) << 32) | u64::from(m.cpu.y());
            assert_eq!(
                actual as i64,
                i64::from(multiplier) * i64::from(multiplicand)
            );
        }
    }
}

#[test]
fn flush_is_unprivileged_and_does_not_access_memory_or_change_flags() {
    let mut m = setup(&[alu(0x3b, 3, 1, 3, true)], 7, 15, PSR_ET);
    m.cpu.write_reg(1, 0xdead_0000); // unmapped, unaligned operand is harmless
    m.cpu.write_reg(3, 55);
    m.step().unwrap();
    assert_eq!(m.cpu.pc(), START + 4);
    assert_eq!(m.cpu.read_reg(3), 55);
    assert_eq!(m.cpu.y(), 7);
    assert_eq!(m.cpu.icc(), flags(true, true, true, true));
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}
