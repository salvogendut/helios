use helios::bus::Bus;
use helios::cpu::{FaultKind, Step, PSR_EF, PSR_ET, PSR_S};
use helios::fpu::{DeferredEntry, FpuState};
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 32;
const FSR_SEED: u32 = 0xc7b; // FCC=U, AEXC=3, stale CEXC=27
const FSR_BASE: u32 = 0x0008_0c60; // version=4, FCC=U, AEXC=3
const SENTINEL: u64 = 0xdec0_adde_a5a5_5a5a;

fn op(opf: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
    0x81a0_0000 | (rd << 25) | (rs1 << 14) | (opf << 5) | rs2
}

fn put(m: &mut Machine, address: u32, words: &[u32]) {
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    m.load_image(address, &bytes).unwrap();
}

fn setup(program: &[u32], fsr: u32) -> Machine {
    let mut m = Machine::bare_metal(4096, 0).unwrap();
    put(
        &mut m,
        0,
        &[0x8188_6000, NOP, NOP, NOP, 0xc108_a000, NOP, NOP, NOP],
    );
    put(&mut m, START, program);
    m.cpu.write_reg(1, PSR_S | PSR_EF | PSR_ET);
    m.cpu.write_reg(2, 0x200);
    m.bus.write32(0x200, fsr).unwrap();
    for _ in 0..8 {
        m.step().unwrap();
    }
    m
}

fn write(m: &mut Machine, double: bool, index: u8, bits: u64) {
    if double {
        m.cpu.fpu.write_reg(index, (bits >> 32) as u32);
        m.cpu.fpu.write_reg(index + 1, bits as u32);
    } else {
        m.cpu.fpu.write_reg(index, bits as u32);
    }
}

fn read(m: &Machine, double: bool, index: u8) -> u64 {
    let high = u64::from(m.cpu.fpu.read_reg(index));
    if double {
        (high << 32) | u64::from(m.cpu.fpu.read_reg(index + 1))
    } else {
        high
    }
}

fn masked(opf: u32, a: u64, b: u64, rd: u32, result: u64, flags: u32) {
    let double = opf & 3 == 2;
    let mut m = setup(&[op(opf, 8, 0, 2)], (rd << 30) | FSR_SEED);
    write(&mut m, double, 0, a);
    write(&mut m, double, 2, b);
    write(&mut m, true, 8, SENTINEL);
    assert!(matches!(
        m.step().unwrap(),
        Step::Executed { pc: START, .. }
    ));
    assert_eq!(read(&m, double, 8), result, "a={a:x} b={b:x} RD={rd}");
    if !double {
        assert_eq!(m.cpu.fpu.read_reg(9), SENTINEL as u32);
    }
    assert_eq!(
        m.cpu.fpu.fsr(),
        (rd << 30) | FSR_BASE | flags | (flags << 5)
    );
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
}

#[test]
fn exact_add_sub_read_all_sources_before_writing_any_destination_pair() {
    for (double, twenty, five, twenty_five, fifteen) in [
        (false, 0x41a0_0000, 0x40a0_0000, 0x41c8_0000, 0x4170_0000),
        (
            true,
            0x4034_0000_0000_0000,
            0x4014_0000_0000_0000,
            0x4039_0000_0000_0000,
            0x402e_0000_0000_0000,
        ),
    ] {
        for (base_opf, expected) in [(0x41, twenty_five), (0x45, fifteen)] {
            for rd in 0..32 {
                let opf = base_opf + u32::from(double);
                let instruction = op(opf, rd, u32::from(double), 2 | u32::from(double));
                let mut m = setup(&[instruction], FSR_SEED);
                for r in 0..32 {
                    m.cpu.fpu.write_reg(r, 0xfeed_0000 | u32::from(r));
                }
                write(&mut m, double, 0, twenty);
                write(&mut m, double, 2, five);
                let before: Vec<_> = (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect();
                m.step().unwrap();
                let base = if double { rd & !1 } else { rd } as u8;
                assert_eq!(read(&m, double, base), expected);
                for r in 0..32u8 {
                    if r != base && !(double && r == base + 1) {
                        assert_eq!(m.cpu.fpu.read_reg(r), before[usize::from(r)]);
                    }
                }
                assert_eq!(m.cpu.fpu.fsr(), FSR_BASE);
            }
        }
    }
}

#[test]
fn exact_cancellation_and_signed_zero_obey_rounding_direction() {
    for (double, sign, one) in [
        (false, 1 << 31, 0x3f80_0000),
        (true, 1 << 63, 0x3ff0_0000_0000_0000),
    ] {
        let add = 0x41 + u32::from(double);
        let sub = add + 4;
        for rd in 0..4 {
            let cancellation = if rd == 3 { sign } else { 0 };
            for (opf, a, b, expected) in [
                (add, 0, 0, 0),
                (add, sign, sign, sign),
                (add, 0, sign, cancellation),
                (add, sign, 0, cancellation),
                (sub, 0, sign, 0),
                (sub, sign, 0, sign),
                (sub, 0, 0, cancellation),
                (sub, sign, sign, cancellation),
                (add, one, sign | one, cancellation),
                (add, sign | one, one, cancellation),
                (sub, one, one, cancellation),
                (sub, sign | one, sign | one, cancellation),
            ] {
                masked(opf, a, b, rd, expected, 0);
            }
        }
    }
}

#[test]
fn halfway_sums_use_even_significand_and_distinguish_either_side_of_the_tie() {
    for (double, sign, one, half) in [
        (false, 1 << 31, 0x3f80_0000, 0x3380_0000), // half ulp at 1 = 2^-24
        (true, 1 << 63, 0x3ff0_0000_0000_0000, 0x3ca0_0000_0000_0000), // 2^-53
    ] {
        for subtract in [false, true] {
            let opf = 0x41 + u32::from(double) + 4 * u32::from(subtract);
            let b_sign = if subtract { sign } else { 0 };
            for rd in 0..4 {
                for (a, b, expected) in [
                    (one, half, [one, one, one + 1, one]),
                    (one + 1, half, [one + 2, one + 1, one + 2, one + 1]),
                    (one, half - 1, [one, one, one + 1, one]),
                    (one, half + 1, [one + 1, one, one + 1, one]),
                ] {
                    masked(opf, a, b | b_sign, rd, expected[rd as usize], 1);
                    let magnitude_rd = [0, 1, 3, 2][rd as usize];
                    masked(
                        opf,
                        a | sign,
                        b | (b_sign ^ sign),
                        rd,
                        sign | expected[magnitude_rd],
                        1,
                    );
                }
            }
        }
    }
}

#[test]
fn discarded_operands_keep_sticky_information_across_large_shifts_and_subtraction_borrows() {
    for (double, sign, fraction, bias, one) in [
        (false, 1 << 31, 23, 127, 0x3f80_0000),
        (true, 1 << 63, 52, 1023, 0x3ff0_0000_0000_0000),
    ] {
        let add = 0x41 + u32::from(double);
        let sub = add + 4;
        let mut small = vec![1];
        for gap in [fraction + 3, 63, 64, 65, 125] {
            small.push((bias - gap) << fraction);
        }
        for b in small {
            for rd in 0..4 {
                let up = [one, one, one + 1, one][rd as usize];
                let down = [one, one - 1, one, one - 1][rd as usize];
                masked(add, one, b, rd, up, 1);
                masked(add, b, one, rd, up, 1);
                masked(sub, one, b, rd, down, 1);
                masked(add, one, b | sign, rd, down, 1);
                let negative = [one, one - 1, one - 1, one][rd as usize];
                masked(sub, b, one, rd, sign | negative, 1);
            }
        }
    }
}

#[test]
fn cancellation_normalizes_exactly_and_subnormal_sums_are_exact() {
    for (double, sign, minimum, one, next_after_one_difference, below_one_difference) in [
        (
            false,
            1 << 31,
            0x0080_0000,
            0x3f80_0000,
            0x3400_0000,
            0x3380_0000,
        ),
        (
            true,
            1 << 63,
            0x0010_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0x3cb0_0000_0000_0000,
            0x3ca0_0000_0000_0000,
        ),
    ] {
        let add = 0x41 + u32::from(double);
        let sub = add + 4;
        for rd in 0..4 {
            for (a, b, expected) in [
                (one + 1, one, next_after_one_difference),
                (one, one - 1, below_one_difference),
                (minimum, minimum - 1, 1),
                (minimum + 1, minimum, 1),
                (minimum * 2 - 1, minimum, minimum - 1),
            ] {
                masked(sub, a, b, rd, expected, 0);
                masked(add, a, b | sign, rd, expected, 0);
                masked(sub, b, a, rd, expected | sign, 0);
            }
            masked(add, minimum - 1, 1, rd, minimum, 0);
            masked(add, 1, 1, rd, 2, 0);
            masked(sub, 1, sign | 1, rd, 2, 0);
        }
    }
}

#[test]
fn overflow_is_decided_after_rounding_including_just_above_maximum_finite() {
    for (double, sign, inf, half) in [
        (false, 1 << 31, 0x7f80_0000, 0x7300_0000), // half ulp at maximum = 2^103
        (true, 1 << 63, 0x7ff0_0000_0000_0000, 0x7c90_0000_0000_0000), // 2^970
    ] {
        let max = inf - 1;
        for subtract in [false, true] {
            let opf = 0x41 + u32::from(double) + 4 * u32::from(subtract);
            let b_sign = if subtract { sign } else { 0 };
            for rd in 0..4 {
                let tiny_expected = [max, max, inf, max][rd as usize];
                let half_expected = [inf, max, inf, max][rd as usize];
                for b in [1, half - 1] {
                    masked(
                        opf,
                        max,
                        b | b_sign,
                        rd,
                        tiny_expected,
                        if rd == 2 { 9 } else { 1 },
                    );
                }
                for b in [half, half + 1] {
                    masked(
                        opf,
                        max,
                        b | b_sign,
                        rd,
                        half_expected,
                        if rd == 0 || rd == 2 { 9 } else { 1 },
                    );
                }
                masked(opf, max, max | b_sign, rd, half_expected, 9);
                masked(
                    opf,
                    max | sign,
                    max | (b_sign ^ sign),
                    rd,
                    sign | [inf, max, max, inf][rd as usize],
                    9,
                );
            }
        }
    }
}

#[test]
fn infinities_and_nan_payloads_follow_operand_order_for_addition_and_subtraction() {
    for (double, sign, inf, quiet, meiko) in [
        (false, 1 << 31, 0x7f80_0000, 1 << 22, 0x7fff_0000),
        (
            true,
            1 << 63,
            0x7ff0_0000_0000_0000,
            1 << 51,
            0x7fff_e000_0000_0000,
        ),
    ] {
        let q1 = inf | quiet | 7;
        let q2 = inf | quiet | 17;
        for subtract in [false, true] {
            let opf = 0x41 + u32::from(double) + 4 * u32::from(subtract);
            let flip = if subtract { sign } else { 0 };
            for (a, b, expected, flags) in [
                (q1 | sign, q2, q1, 0),
                (q2, q1 | sign, q2, 0),
                (0, q2 | sign, q2, 0),
                (q1, inf | 1, meiko, 16),
                (inf | sign | 1, q2, meiko, 16),
                (inf, 0, inf, 0),
                (0, inf, inf | flip, 0),
                (inf, inf | flip, inf, 0),
                (inf, inf | (flip ^ sign), sign - 1, 16),
                (sign | inf, inf | flip, sign - 1, 16),
            ] {
                masked(opf, a, b, 0, expected, flags);
            }
        }
    }
}

#[test]
fn enabled_add_sub_exceptions_inhibit_results_and_exact_tiny_results_trap_only_on_uf() {
    for (double, sign, one, inf, minimum) in [
        (false, 1 << 31, 0x3f80_0000, 0x7f80_0000, 0x0080_0000),
        (
            true,
            1 << 63,
            0x3ff0_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x0010_0000_0000_0000,
        ),
    ] {
        for subtract in [false, true] {
            let opf = 0x41 + u32::from(double) + 4 * u32::from(subtract);
            let flip = if subtract { sign } else { 0 };
            let instruction = op(opf, 8, 0, 2);
            for (a, effective_b, mask, cause) in [
                (inf, sign | inf, 31, 16),
                (inf - 1, inf - 1, 31, 8),
                (inf - 1, inf - 1, 1, 1),
                (one, 1, 31, 1),
                (minimum, sign | (minimum - 1), 31, 4),
                (1, 0, 4, 4),
            ] {
                let mut m = setup(&[instruction], (mask << 23) | FSR_SEED);
                write(&mut m, double, 0, a);
                write(&mut m, double, 2, effective_b ^ flip);
                write(&mut m, true, 8, SENTINEL);
                let before: Vec<_> = (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect();
                assert!(matches!(
                    m.step().unwrap(),
                    Step::Executed { pc: START, .. }
                ));
                assert_eq!(
                    (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect::<Vec<_>>(),
                    before
                );
                assert_eq!(m.cpu.fpu.fsr(), (mask << 23) | FSR_BASE | 0x6000 | cause);
                assert_eq!(m.cpu.fpu.state(), FpuState::ExceptionPending);
                assert_eq!(
                    m.cpu.fpu.deferred_entry(),
                    Some(DeferredEntry {
                        pc: START,
                        instruction
                    })
                );
            }
            // Exact tiny results with only NXM enabled must complete, without NX or UF.
            let mut m = setup(&[instruction], (1 << 23) | FSR_SEED);
            write(&mut m, double, 0, 1);
            write(&mut m, double, 2, flip);
            m.step().unwrap();
            assert_eq!(read(&m, double, 8), 1);
            assert_eq!(m.cpu.fpu.fsr(), (1 << 23) | FSR_BASE);
        }
    }
}

#[test]
fn inexact_addition_defers_until_subtraction_and_queue_recovery_preserves_both_pcs() {
    let add = op(0x41, 8, 0, 2);
    let sub = op(0x45, 8, 0, 2);
    let mut m = setup(&[add, 0x8801_2001, sub], (1 << 23) | FSR_SEED);
    write(&mut m, false, 0, 0x3f80_0000);
    write(&mut m, false, 2, 1);
    write(&mut m, true, 8, SENTINEL);
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(4), 1);
    assert!(matches!(
        m.step().unwrap(),
        Step::TrapTaken {
            trap_type: 8,
            vector: 0x80,
            ..
        }
    ));
    assert_eq!(
        (m.cpu.read_reg(17), m.cpu.read_reg(18)),
        (START + 8, START + 12)
    );
    assert_eq!(read(&m, true, 8), SENTINEL);
    put(&mut m, 0x80, &[0xc130_a010, sub]); // STDFQ [g2+16]
    m.step().unwrap();
    assert_eq!(m.bus.read_doubleword(0x210).unwrap(), [START, add]);
    write(&mut m, false, 2, 0x3f00_0000);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.read_reg(8), 0x3f00_0000);
    assert_eq!(m.cpu.fpu.fsr(), (1 << 23) | FSR_BASE);
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
}

#[test]
fn disabled_add_sub_trap_and_quad_variants_remain_precise_host_stops() {
    for opf in [0x41, 0x42, 0x43, 0x45, 0x46, 0x47] {
        let instruction = op(opf, 8, 0, 2);
        let mut m = Machine::bare_metal(4096, 0).unwrap();
        put(&mut m, 0, &[0x8188_6000, NOP, NOP, NOP, instruction]);
        m.cpu.write_reg(1, PSR_S | PSR_ET);
        for _ in 0..4 {
            m.step().unwrap();
        }
        let before = m.cpu.fpu.clone();
        assert!(
            matches!(m.step().unwrap(), Step::TrapTaken { trap_type: 4, fault, .. }
            if fault.kind == FaultKind::FpDisabled)
        );
        assert_eq!(m.cpu.fpu, before);
    }
    for opf in [0x43, 0x47] {
        let mut m = setup(&[op(opf, 8, 0, 2)], FSR_SEED);
        let before = m.cpu.clone();
        assert_eq!(
            m.step().unwrap_err().kind,
            FaultKind::UnsupportedInstruction
        );
        assert_eq!(m.cpu, before);
    }
}
