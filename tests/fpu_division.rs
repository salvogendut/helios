use helios::bus::Bus;
use helios::cpu::{FaultKind, Step, PSR_EF, PSR_ET, PSR_S};
use helios::fpu::{DeferredEntry, FpuState};
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 32;
const FSR_SEED: u32 = 0xc7b; // FCC=U, AEXC=3, stale CEXC=27
const FSR_BASE: u32 = 0x0008_0c60; // version=4, FCC=U, AEXC=3
const SENTINEL: u64 = 0xdec0_adde_a5a5_5a5a;

fn div(double: bool, rd: u32, rs1: u32, rs2: u32) -> u32 {
    0x81a0_0000 | (rd << 25) | (rs1 << 14) | (if double { 0x4e } else { 0x4d } << 5) | rs2
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

fn masked(double: bool, a: u64, b: u64, rd: u32, result: u64, flags: u32) {
    let mut m = setup(&[div(double, 8, 0, 2)], (rd << 30) | FSR_SEED);
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
fn exact_division_supports_f0_overlapping_sources_and_reserved_double_register_bits() {
    for (double, a, b, result) in [
        (false, 0x42c8_0000, 0x4080_0000, 0x41c8_0000), // OBP: 100 / 4 = 25
        (
            true,
            0x4059_0000_0000_0000,
            0x4010_0000_0000_0000,
            0x4039_0000_0000_0000,
        ),
    ] {
        for rd in 0..32 {
            let mut m = setup(
                &[div(double, rd, u32::from(double), 2 | u32::from(double))],
                FSR_SEED,
            );
            for r in 0..32 {
                m.cpu.fpu.write_reg(r, 0xfeed_0000 | u32::from(r));
            }
            write(&mut m, double, 0, a);
            write(&mut m, double, 2, b);
            let before: Vec<_> = (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect();
            m.step().unwrap();
            let base = if double { rd & !1 } else { rd } as u8;
            assert_eq!(read(&m, double, base), result);
            for r in 0..32u8 {
                if r != base && !(double && r == base + 1) {
                    assert_eq!(m.cpu.fpu.read_reg(r), before[usize::from(r)]);
                }
            }
            assert_eq!(m.cpu.fpu.fsr(), FSR_BASE);
        }
    }
}

#[test]
fn recurring_quotients_round_in_all_four_directions_for_both_signs() {
    for (double, sign, one, three, expected) in [
        (
            false,
            1 << 31,
            0x3f80_0000,
            0x4040_0000,
            [0x3eaa_aaab, 0x3eaa_aaaa, 0x3eaa_aaab, 0x3eaa_aaaa],
        ),
        (
            true,
            1 << 63,
            0x3ff0_0000_0000_0000,
            0x4008_0000_0000_0000,
            [
                0x3fd5_5555_5555_5555,
                0x3fd5_5555_5555_5555,
                0x3fd5_5555_5555_5556,
                0x3fd5_5555_5555_5555,
            ],
        ),
    ] {
        for rd in 0..4 {
            masked(double, one, three, rd, expected[rd as usize], 1);
            let magnitude_rd = [0, 1, 3, 2][rd as usize];
            masked(
                double,
                one | sign,
                three,
                rd,
                expected[magnitude_rd] | sign,
                1,
            );
            masked(
                double,
                one,
                three | sign,
                rd,
                expected[magnitude_rd] | sign,
                1,
            );
        }
    }
}

#[test]
fn gradual_underflow_rounds_ties_once_and_detects_tininess_before_rounding() {
    for (double, sign, minimum, two, max) in [
        (false, 1 << 31, 0x0080_0000, 0x4000_0000, 0x7f7f_ffff),
        (
            true,
            1 << 63,
            0x0010_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x7fef_ffff_ffff_ffff,
        ),
    ] {
        for rd in 0..4 {
            // Exact tiny values do not raise UF with UFM clear.
            masked(double, minimum, two, rd, minimum / 2, 0);
            masked(double, 1, two, rd, [0, 0, 1, 0][rd as usize], 5);
            masked(double, 3, two, rd, [2, 1, 2, 1][rd as usize], 5);
            // Halfway below minimum normal: RN rounds upward, but still UF|NX.
            masked(
                double,
                2 * minimum - 1,
                two,
                rd,
                [minimum, minimum - 1, minimum, minimum - 1][rd as usize],
                5,
            );
            // Extreme exponent difference must not overflow an integer shift.
            masked(double, 1, max, rd, [0, 0, 1, 0][rd as usize], 5);
            masked(
                double,
                sign | 1,
                max,
                rd,
                sign | [0, 0, 0, 1][rd as usize],
                5,
            );
        }
    }
}

#[test]
fn overflow_returns_signed_infinity_or_maximum_finite_by_rounding_direction() {
    for (double, sign, inf) in [
        (false, 1 << 31, 0x7f80_0000),
        (true, 1 << 63, 0x7ff0_0000_0000_0000),
    ] {
        for rd in 0..4 {
            masked(
                double,
                inf - 1,
                1,
                rd,
                [inf, inf - 1, inf, inf - 1][rd as usize],
                9,
            );
            masked(
                double,
                sign | (inf - 1),
                1,
                rd,
                sign | [inf, inf - 1, inf - 1, inf][rd as usize],
                9,
            );
        }
    }
}

#[test]
fn zeros_infinities_and_divide_by_zero_follow_ieee_classification() {
    for (double, sign, one, inf) in [
        (false, 1 << 31, 0x3f80_0000, 0x7f80_0000),
        (true, 1 << 63, 0x3ff0_0000_0000_0000, 0x7ff0_0000_0000_0000),
    ] {
        for a_sign in [0, sign] {
            for b_sign in [0, sign] {
                let result_sign = a_sign ^ b_sign;
                masked(double, one | a_sign, b_sign, 0, inf | result_sign, 2);
                masked(double, inf | a_sign, b_sign, 0, inf | result_sign, 0);
                masked(double, inf | a_sign, one | b_sign, 0, inf | result_sign, 0);
                masked(double, a_sign, one | b_sign, 0, result_sign, 0);
                masked(double, one | a_sign, inf | b_sign, 0, result_sign, 0);
                masked(double, a_sign, b_sign, 0, sign - 1, 16);
                masked(double, inf | a_sign, inf | b_sign, 0, sign - 1, 16);
            }
        }
    }
}

#[test]
fn meiko_nan_rules_clear_sign_prefer_first_quiet_operand_and_replace_signaling_nan() {
    for (double, sign, one, inf, quiet, meiko) in [
        (
            false,
            1 << 31,
            0x3f80_0000,
            0x7f80_0000,
            1 << 22,
            0x7fff_0000,
        ),
        (
            true,
            1 << 63,
            0x3ff0_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            1 << 51,
            0x7fff_e000_0000_0000,
        ),
    ] {
        let q1 = inf | quiet | 0x123;
        let q2 = inf | quiet | 0x456;
        let s = inf | 1;
        for (a, b, expected, flags) in [
            (q1 | sign, q2, q1, 0),
            (q2, q1 | sign, q2, 0),
            (one, q2 | sign, q2, 0),
            (q1 | sign, 0, q1, 0),
            (s, q1, meiko, 16),
            (q1, s | sign, meiko, 16),
            (one, s, meiko, 16),
            (s | sign, s, meiko, 16),
        ] {
            masked(double, a, b, 0, expected, flags);
        }
    }
}

#[test]
fn enabled_exceptions_preserve_destinations_and_accrual_and_select_only_enabled_cause() {
    for (double, one, two, three, inf, minimum) in [
        (
            false,
            0x3f80_0000,
            0x4000_0000,
            0x4040_0000,
            0x7f80_0000,
            0x0080_0000,
        ),
        (
            true,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x4008_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x0010_0000_0000_0000,
        ),
    ] {
        for (a, b, mask, cause) in [
            (0, 0, 31, 16),
            (one, 0, 31, 2),
            (inf - 1, 1, 31, 8),
            (inf - 1, 1, 1, 1),
            (1, two, 31, 4),
            (1, two, 1, 1),
            (one, three, 31, 1),
            (minimum, two, 4, 4),
        ] {
            let instruction = div(double, 8, 0, 2);
            let mut m = setup(&[instruction], (mask << 23) | FSR_SEED);
            write(&mut m, double, 0, a);
            write(&mut m, double, 2, b);
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
    }
}

#[test]
fn deferred_division_exception_allows_integer_work_and_can_be_drained_then_retried() {
    let instruction = div(false, 8, 0, 2);
    // add g4,1,g4; stfsr [g2+8] (detecting instruction)
    let mut m = setup(
        &[instruction, 0x8801_2001, 0xc128_a008],
        (2 << 23) | FSR_SEED,
    );
    write(&mut m, false, 0, 0x3f80_0000);
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
    assert_eq!(m.bus.read32(0x208).unwrap(), 0);
    assert_eq!(read(&m, true, 8), SENTINEL);
    // STDFQ [g2+16], then retry with a nonzero divisor. ET=0 is permitted.
    put(&mut m, 0x80, &[0xc130_a010, instruction]);
    m.step().unwrap();
    assert_eq!(m.bus.read_doubleword(0x210).unwrap(), [START, instruction]);
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
    write(&mut m, false, 2, 0x4000_0000);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.read_reg(8), 0x3f00_0000);
    assert_eq!(m.cpu.fpu.fsr(), (2 << 23) | FSR_BASE); // clear FTT/CEXC, retain TEM/AEXC
}

#[test]
fn successful_division_clears_current_flags_but_retains_prior_accrued_exceptions() {
    let instruction = div(false, 8, 0, 2);
    let mut m = setup(&[instruction, instruction], FSR_SEED);
    write(&mut m, false, 0, 0x7f7f_ffff);
    write(&mut m, false, 2, 1);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.fsr(), FSR_BASE | 9 | (9 << 5));
    write(&mut m, false, 0, 0x4000_0000);
    write(&mut m, false, 2, 0x4000_0000);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.read_reg(8), 0x3f80_0000);
    assert_eq!(m.cpu.fpu.fsr(), FSR_BASE | (9 << 5));
}

#[test]
fn disabled_fpu_traps_before_division_and_quad_division_remains_an_explicit_host_stop() {
    for opf in [0x4d, 0x4e, 0x4f] {
        let instruction = 0x91a0_0002 | (opf << 5);
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
    let quad = 0x91a0_0002 | (0x4f << 5);
    let mut m = setup(&[quad], FSR_SEED);
    let before = m.cpu.clone();
    assert_eq!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedInstruction
    );
    assert_eq!(m.cpu, before);
}
