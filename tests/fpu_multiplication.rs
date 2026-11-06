use helios::bus::Bus;
use helios::cpu::{FaultKind, Step, PSR_EF, PSR_ET, PSR_S};
use helios::fpu::{DeferredEntry, FpuState};
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 32;
const FSR_SEED: u32 = 0xc7b; // FCC=U, AEXC=3, stale CEXC=27
const FSR_BASE: u32 = 0x0008_0c60; // version=4, FCC=U, AEXC=3
const SENTINEL: u64 = 0xdec0_adde_a5a5_5a5a;

fn mul(double: bool, rd: u32, rs1: u32, rs2: u32) -> u32 {
    0x81a0_0000 | (rd << 25) | (rs1 << 14) | (if double { 0x4a } else { 0x49 } << 5) | rs2
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
    let mut m = setup(&[mul(double, 8, 0, 2)], (rd << 30) | FSR_SEED);
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
fn exact_products_read_sources_before_writing_all_destination_registers_and_pairs() {
    for (double, five, twenty_five) in [
        (false, 0x40a0_0000, 0x41c8_0000),
        (true, 0x4014_0000_0000_0000, 0x4039_0000_0000_0000),
    ] {
        for rd in 0..32 {
            let instruction = mul(double, rd, u32::from(double), 2 | u32::from(double));
            let mut m = setup(&[instruction], FSR_SEED);
            for r in 0..32 {
                m.cpu.fpu.write_reg(r, 0xfeed_0000 | u32::from(r));
            }
            write(&mut m, double, 0, five);
            write(&mut m, double, 2, five);
            let before: Vec<_> = (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect();
            m.step().unwrap();
            let base = if double { rd & !1 } else { rd } as u8;
            assert_eq!(read(&m, double, base), twenty_five);
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
fn full_product_low_bits_control_rounding_for_both_result_signs() {
    for (double, sign, one) in [
        (false, 1 << 31, 0x3f80_0000),
        (true, 1 << 63, 0x3ff0_0000_0000_0000),
    ] {
        // (1+ulp)^2 = 1+2ulp+ulp^2; all product bits must survive until rounding.
        for (a, expected) in [
            (one + 1, [one + 2, one + 2, one + 3, one + 2]),
            (one - 1, [one - 2, one - 2, one - 1, one - 2]),
        ] {
            for rd in 0..4 {
                masked(double, a, a, rd, expected[rd as usize], 1);
                let magnitude_rd = [0, 1, 3, 2][rd as usize];
                masked(double, a | sign, a, rd, sign | expected[magnitude_rd], 1);
                masked(double, a, a | sign, rd, sign | expected[magnitude_rd], 1);
            }
        }
    }
}

#[test]
fn halfway_products_round_to_even_and_normalize_across_a_binade() {
    for (double, one, one_and_half, two_and_quarter) in [
        (false, 0x3f80_0000, 0x3fc0_0000, 0x4010_0000),
        (
            true,
            0x3ff0_0000_0000_0000,
            0x3ff8_0000_0000_0000,
            0x4002_0000_0000_0000,
        ),
    ] {
        for rd in 0..4 {
            let odd = [2, 1, 2, 1][rd as usize];
            let even = [4, 4, 5, 4][rd as usize];
            masked(double, one + 1, one_and_half, rd, one_and_half + odd, 1);
            masked(double, one + 3, one_and_half, rd, one_and_half + even, 1);
            masked(double, one_and_half, one_and_half, rd, two_and_quarter, 0);
        }
    }
}

#[test]
fn underflow_rounds_once_and_retains_tininess_when_rounding_up_to_normal() {
    for (double, minimum, one, half) in [
        (false, 0x0080_0000, 0x3f80_0000, 0x3f00_0000),
        (
            true,
            0x0010_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
        ),
    ] {
        for rd in 0..4 {
            masked(double, minimum, half, rd, minimum / 2, 0);
            masked(double, 1, half, rd, [0, 0, 1, 0][rd as usize], 5);
            masked(double, 3, half, rd, [2, 1, 2, 1][rd as usize], 5);
            let crossover = [minimum, minimum - 1, minimum, minimum - 1][rd as usize];
            masked(double, minimum * 2 - 1, half, rd, crossover, 5);
            masked(double, minimum, one - 1, rd, crossover, 5);
            masked(double, minimum, one + 1, rd, minimum + 1, 0);
        }
    }
}

#[test]
fn extreme_exponents_and_subnormal_operands_preserve_exact_and_directed_results() {
    for (double, sign, maximum, result) in [
        (false, 1 << 31, 0x7f7f_ffff, 0x34ff_ffff),
        (true, 1 << 63, 0x7fef_ffff_ffff_ffff, 0x3ccf_ffff_ffff_ffff),
    ] {
        for rd in 0..4 {
            masked(double, 1, maximum, rd, result, 0);
            masked(double, maximum, 1, rd, result, 0);
            masked(double, 1, 1, rd, [0, 0, 1, 0][rd as usize], 5);
            masked(double, sign | 1, 1, rd, sign | [0, 0, 0, 1][rd as usize], 5);
        }
    }
}

#[test]
fn overflow_flags_follow_unbounded_exponent_rounding_and_directed_result_selection() {
    for (double, sign, one, two, inf) in [
        (false, 1 << 31, 0x3f80_0000, 0x4000_0000, 0x7f80_0000),
        (
            true,
            1 << 63,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x7ff0_0000_0000_0000,
        ),
    ] {
        let max = inf - 1;
        for rd in 0..4 {
            let expected = [inf, max, inf, max][rd as usize];
            masked(double, max, two, rd, expected, 9);
            masked(
                double,
                max | sign,
                two,
                rd,
                sign | [inf, max, max, inf][rd as usize],
                9,
            );
            // (2-2ulp)*(1+ulp) = 2-2ulp^2: just below the next binade.
            masked(
                double,
                inf - 2,
                one + 1,
                rd,
                expected,
                if rd == 0 || rd == 2 { 9 } else { 1 },
            );
        }
    }
}

#[test]
fn zero_infinity_and_nan_products_follow_meiko_classification_without_divide_by_zero() {
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
        for sa in [0, sign] {
            for sb in [0, sign] {
                masked(double, sa, sb, 3, sa ^ sb, 0);
                masked(double, one | sa, sb, 3, sa ^ sb, 0);
                masked(double, sa, one | sb, 3, sa ^ sb, 0);
                masked(double, inf | sa, one | sb, 0, inf | (sa ^ sb), 0);
                masked(double, inf | sa, inf | sb, 0, inf | (sa ^ sb), 0);
                masked(double, inf | sa, sb, 0, sign - 1, 16);
                masked(double, sa, inf | sb, 0, sign - 1, 16);
            }
        }
        let q1 = inf | quiet | 7;
        let q2 = inf | quiet | 17;
        for (a, b, result, flags) in [
            (q1 | sign, q2, q1, 0),
            (q2, q1 | sign, q2, 0),
            (one, q2 | sign, q2, 0),
            (q1, 0, q1, 0),
            (inf | 1, q1, meiko, 16),
            (q1, sign | inf | 1, meiko, 16),
        ] {
            masked(double, a, b, 0, result, flags);
        }
    }
}

#[test]
fn enabled_exceptions_inhibit_all_registers_and_accrual_and_keep_only_the_enabled_cause() {
    for (double, one, two, half, inf, minimum) in [
        (
            false,
            0x3f80_0000,
            0x4000_0000,
            0x3f00_0000,
            0x7f80_0000,
            0x0080_0000,
        ),
        (
            true,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x3fe0_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x0010_0000_0000_0000,
        ),
    ] {
        let instruction = mul(double, 8, 0, 2);
        for (a, b, mask, cause) in [
            (inf, 0, 31, 16),
            (inf - 1, two, 31, 8),
            (inf - 1, two, 1, 1),
            (1, half, 31, 4),
            (1, half, 1, 1),
            (minimum, half, 4, 4),
            (one + 1, one + 1, 31, 1),
        ] {
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
        let mut m = setup(&[instruction], (1 << 23) | FSR_SEED);
        write(&mut m, double, 0, minimum);
        write(&mut m, double, 2, half);
        m.step().unwrap();
        assert_eq!(read(&m, double, 8), minimum / 2);
        assert_eq!(m.cpu.fpu.fsr(), (1 << 23) | FSR_BASE);
    }
}

#[test]
fn overflow_is_deferred_and_draining_the_queue_allows_an_exact_multiply_to_complete() {
    let instruction = mul(false, 8, 0, 2);
    let detector = mul(true, 10, 0, 2);
    let mut m = setup(&[instruction, 0x8801_2001, detector], (8 << 23) | FSR_SEED);
    write(&mut m, false, 0, 0x7f7f_ffff);
    write(&mut m, false, 2, 0x4000_0000);
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
    put(&mut m, 0x80, &[0xc130_a010, instruction]);
    m.step().unwrap();
    assert_eq!(m.bus.read_doubleword(0x210).unwrap(), [START, instruction]);
    write(&mut m, false, 0, 0x40a0_0000);
    write(&mut m, false, 2, 0x40a0_0000);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.read_reg(8), 0x41c8_0000);
    assert_eq!(m.cpu.fpu.fsr(), (8 << 23) | FSR_BASE);
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
}

#[test]
fn disabled_multiply_traps_and_quad_or_widening_variants_remain_precise_host_stops() {
    for opf in [0x49, 0x4a, 0x4b, 0x69, 0x6e] {
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
    for opf in [0x4b, 0x69, 0x6e] {
        let mut m = setup(&[0x91a0_0002 | (opf << 5)], FSR_SEED);
        let before = m.cpu.clone();
        assert_eq!(
            m.step().unwrap_err().kind,
            FaultKind::UnsupportedInstruction
        );
        assert_eq!(m.cpu, before);
    }
}
