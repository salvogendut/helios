use helios::bus::Bus;
use helios::cpu::{FaultKind, Step, PSR_EF, PSR_ET, PSR_S};
use helios::fpu::{DeferredEntry, FpuState};
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 32;
const FSR_SEED: u32 = 0xc7b; // FCC=U, AEXC=3, stale CEXC=27
const FSR_BASE: u32 = 0x0008_0c60;

fn instruction(double: bool, rd: u32, rs2: u32) -> u32 {
    0x81a0_0000 | (rd << 25) | (if double { 0xc8 } else { 0xc4 } << 5) | rs2
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

#[test]
fn signed_integer_boundaries_convert_exactly_to_double_in_every_rounding_mode() {
    // Encodings derived from sign, binary exponent and fraction, not host casts.
    for (word, expected) in [
        (0, 0),
        (1, 0x3ff0_0000_0000_0000),
        (u32::MAX, 0xbff0_0000_0000_0000),
        (0x0100_0001, 0x4170_0000_1000_0000),
        (0x7fff_ffff, 0x41df_ffff_ffc0_0000),
        (0x8000_0000, 0xc1e0_0000_0000_0000),
        (0x8000_0001, 0xc1df_ffff_ffc0_0000),
    ] {
        for rounding in 0..4 {
            let mut m = setup(&[instruction(true, 8, 4)], (rounding << 30) | FSR_SEED);
            m.cpu.fpu.write_reg(4, word);
            m.step().unwrap();
            let result =
                (u64::from(m.cpu.fpu.read_reg(8)) << 32) | u64::from(m.cpu.fpu.read_reg(9));
            assert_eq!(result, expected);
            assert_eq!(m.cpu.fpu.fsr(), (rounding << 30) | FSR_BASE);
            assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
        }
    }
}

#[test]
fn single_integer_conversion_rounds_ties_and_binade_carry_with_signed_directions() {
    for (word, results, flags) in [
        (0, [0; 4], 0),
        (1, [0x3f80_0000; 4], 0),
        (u32::MAX, [0xbf80_0000; 4], 0),
        (0x00ff_ffff, [0x4b7f_ffff; 4], 0),
        (0x8000_0000, [0xcf00_0000; 4], 0),
        (
            0x0100_0001,
            [0x4b80_0000, 0x4b80_0000, 0x4b80_0001, 0x4b80_0000],
            1,
        ),
        (
            0x0100_0003,
            [0x4b80_0002, 0x4b80_0001, 0x4b80_0002, 0x4b80_0001],
            1,
        ),
        (
            0xfeff_ffff,
            [0xcb80_0000, 0xcb80_0000, 0xcb80_0000, 0xcb80_0001],
            1,
        ),
        (
            0xfeff_fffd,
            [0xcb80_0002, 0xcb80_0001, 0xcb80_0001, 0xcb80_0002],
            1,
        ),
        (
            0x7fff_ffff,
            [0x4f00_0000, 0x4eff_ffff, 0x4f00_0000, 0x4eff_ffff],
            1,
        ),
        (
            0x8000_0001,
            [0xcf00_0000, 0xceff_ffff, 0xceff_ffff, 0xcf00_0000],
            1,
        ),
    ] {
        for rounding in 0..4 {
            let mut m = setup(&[instruction(false, 8, 4)], (rounding << 30) | FSR_SEED);
            m.cpu.fpu.write_reg(4, word);
            m.cpu.fpu.write_reg(9, 0xdec0_adde);
            m.step().unwrap();
            assert_eq!(m.cpu.fpu.read_reg(8), results[rounding as usize]);
            assert_eq!(m.cpu.fpu.read_reg(9), 0xdec0_adde);
            assert_eq!(
                m.cpu.fpu.fsr(),
                (rounding << 30) | FSR_BASE | flags | (flags << 5)
            );
        }
    }
}

#[test]
fn conversion_reads_the_actual_word_before_overwriting_an_overlapping_pair() {
    for double in [false, true] {
        for rd in 0..32 {
            for rs2 in [rd, rd ^ 1] {
                let mut m = setup(&[instruction(double, rd, rs2)], FSR_SEED);
                for r in 0..32u8 {
                    m.cpu.fpu.write_reg(r, 0xfeed_0000 | u32::from(r));
                }
                m.cpu.fpu.write_reg(rs2 as u8, 3);
                let before: Vec<_> = (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect();
                m.step().unwrap();
                let base = if double { rd & !1 } else { rd } as u8;
                for r in 0..32u8 {
                    let expected = if r == base {
                        if double {
                            0x4008_0000
                        } else {
                            0x4040_0000
                        }
                    } else if double && r == base + 1 {
                        0
                    } else {
                        before[usize::from(r)]
                    };
                    assert_eq!(m.cpu.fpu.read_reg(r), expected);
                }
            }
        }
    }
}

#[test]
fn inexact_integer_conversion_defers_without_destination_or_accrual_changes() {
    let operation = instruction(false, 8, 4);
    let mut m = setup(
        &[operation, NOP, instruction(true, 10, 4)],
        (1 << 23) | FSR_SEED,
    );
    m.cpu.fpu.write_reg(4, 0x0100_0001);
    m.cpu.fpu.write_reg(8, 0xdec0_adde);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.read_reg(8), 0xdec0_adde);
    assert_eq!(m.cpu.fpu.fsr(), (1 << 23) | FSR_BASE | 0x6001);
    assert_eq!(m.cpu.fpu.state(), FpuState::ExceptionPending);
    assert_eq!(
        m.cpu.fpu.deferred_entry(),
        Some(DeferredEntry {
            pc: START,
            instruction: operation
        })
    );
    m.step().unwrap();
    assert!(matches!(
        m.step().unwrap(),
        Step::TrapTaken { trap_type: 8, .. }
    ));
    assert_eq!(
        (m.cpu.read_reg(17), m.cpu.read_reg(18)),
        (START + 8, START + 12)
    );
    put(&mut m, 0x80, &[0xc130_a010, instruction(true, 10, 4)]);
    m.step().unwrap();
    assert_eq!(m.bus.read_doubleword(0x210).unwrap(), [START, operation]);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.read_reg(10), 0x4170_0000);
    assert_eq!(m.cpu.fpu.read_reg(11), 0x1000_0000);
    assert_eq!(m.cpu.fpu.fsr(), (1 << 23) | FSR_BASE);
}

#[test]
fn unsupported_quad_conversion_is_retryable_and_disabled_fpu_takes_priority() {
    let quad = 0x81a0_1984; // fitoq %f4, %f0
    let mut m = setup(&[quad], FSR_SEED);
    let before = m.cpu.fpu.clone();
    assert_eq!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedInstruction
    );
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (START, START + 4));
    assert_eq!(m.cpu.fpu, before);
    let mut m = Machine::bare_metal(4096, 0).unwrap();
    put(
        &mut m,
        0,
        &[0x8188_6000, NOP, NOP, NOP, instruction(true, 8, 4)],
    );
    m.cpu.write_reg(1, PSR_S | PSR_ET);
    for _ in 0..4 {
        m.step().unwrap();
    }
    assert!(matches!(
        m.step().unwrap(),
        Step::TrapTaken { trap_type: 4, .. }
    ));
}

fn convert(opf: u32, bits: u64, rounding: u32, expected: u64, flags: u32) {
    let word = 0x91a0_0004 | (opf << 5); // destination f8, source f4 (or f4/f5)
    let mut m = setup(&[word], (rounding << 30) | FSR_SEED);
    if matches!(opf, 0xc6 | 0xd2) {
        m.cpu.fpu.write_reg(4, (bits >> 32) as u32);
        m.cpu.fpu.write_reg(5, bits as u32);
    } else {
        m.cpu.fpu.write_reg(4, bits as u32);
    }
    m.cpu.fpu.write_reg(9, 0xdec0_adde);
    m.step().unwrap();
    let result = if opf == 0xc9 {
        (u64::from(m.cpu.fpu.read_reg(8)) << 32) | u64::from(m.cpu.fpu.read_reg(9))
    } else {
        assert_eq!(m.cpu.fpu.read_reg(9), 0xdec0_adde);
        u64::from(m.cpu.fpu.read_reg(8))
    };
    assert_eq!(result, expected, "opf={opf:x} bits={bits:x} RD={rounding}");
    assert_eq!(
        m.cpu.fpu.fsr(),
        (rounding << 30) | FSR_BASE | flags | (flags << 5)
    );
}

#[test]
fn floats_truncate_toward_zero_independent_of_rd_and_check_range_after_truncation() {
    for rounding in 0..4 {
        for (opf, sign, one, one_and_half, two31, max, subnormal) in [
            (
                0xd1,
                1u64 << 31,
                0x3f80_0000,
                0x3fc0_0000,
                0x4f00_0000,
                0x7f7f_ffff,
                1,
            ),
            (
                0xd2,
                1u64 << 63,
                0x3ff0_0000_0000_0000,
                0x3ff8_0000_0000_0000,
                0x41e0_0000_0000_0000,
                0x7fef_ffff_ffff_ffff,
                1,
            ),
        ] {
            for s in [0, sign] {
                convert(opf, s, rounding, 0, 0);
                convert(opf, subnormal | s, rounding, 0, 1);
                convert(
                    opf,
                    one | s,
                    rounding,
                    if s == 0 { 1 } else { u64::from(u32::MAX) },
                    0,
                );
                convert(
                    opf,
                    one_and_half | s,
                    rounding,
                    if s == 0 { 1 } else { u64::from(u32::MAX) },
                    1,
                );
                convert(
                    opf,
                    max | s,
                    rounding,
                    if s == 0 { 0x7fff_ffff } else { 0x8000_0000 },
                    16,
                );
            }
            convert(opf, two31, rounding, 0x7fff_ffff, 16);
            convert(opf, two31 | sign, rounding, 0x8000_0000, 0);
        }
        convert(0xd1, 0x4eff_ffff, rounding, 0x7fff_ff80, 0);
        convert(0xd2, 0x41df_ffff_fff0_0000, rounding, 0x7fff_ffff, 1);
        convert(0xd2, 0xc1e0_0000_0018_0000, rounding, 0x8000_0000, 1);
        convert(0xd2, 0xc1e0_0000_0020_0000, rounding, 0x8000_0000, 16);
    }
}

#[test]
fn format_conversion_preserves_signed_zero_infinity_and_exact_subnormals() {
    for rounding in 0..4 {
        for (single, double) in [
            (0, 0),
            (1, 0x36a0_0000_0000_0000),
            (0x0080_0000, 0x3810_0000_0000_0000),
            (0x3fc0_0000, 0x3ff8_0000_0000_0000),
            (0x7f7f_ffff, 0x47ef_ffff_e000_0000),
            (0x7f80_0000, 0x7ff0_0000_0000_0000),
        ] {
            for negative in [false, true] {
                let s = single | (u64::from(negative) << 31);
                let d = double | (u64::from(negative) << 63);
                convert(0xc9, s, rounding, d, 0);
                convert(0xc6, d, rounding, s, 0);
            }
        }
    }
}

#[test]
fn narrowing_rounds_ties_and_tininess_before_rounding_and_overflow_after_rounding() {
    for (bits, results, flags) in [
        (
            0x3ff0_0000_1000_0000,
            [0x3f80_0000, 0x3f80_0000, 0x3f80_0001, 0x3f80_0000],
            [1; 4],
        ),
        (
            0x3ff0_0000_3000_0000,
            [0x3f80_0002, 0x3f80_0001, 0x3f80_0002, 0x3f80_0001],
            [1; 4],
        ),
        (1, [0, 0, 1, 0], [5; 4]),
        (0x3690_0000_0000_0000, [0, 0, 1, 0], [5; 4]),
        (0x36a8_0000_0000_0000, [2, 1, 2, 1], [5; 4]),
        (
            0x380f_ffff_e000_0000,
            [0x0080_0000, 0x007f_ffff, 0x0080_0000, 0x007f_ffff],
            [5; 4],
        ),
        (
            0x47ef_ffff_f000_0000,
            [0x7f80_0000, 0x7f7f_ffff, 0x7f80_0000, 0x7f7f_ffff],
            [9, 1, 9, 1],
        ),
        (
            0x47ef_ffff_efff_ffff,
            [0x7f7f_ffff, 0x7f7f_ffff, 0x7f80_0000, 0x7f7f_ffff],
            [1, 1, 9, 1],
        ),
        (
            0x7fef_ffff_ffff_ffff,
            [0x7f80_0000, 0x7f7f_ffff, 0x7f80_0000, 0x7f7f_ffff],
            [9; 4],
        ),
    ] {
        for rounding in 0..4 {
            convert(
                0xc6,
                bits,
                rounding,
                results[rounding as usize],
                flags[rounding as usize],
            );
            let direction = [0, 1, 3, 2][rounding as usize];
            convert(
                0xc6,
                bits | (1 << 63),
                rounding,
                results[direction] | (1 << 31),
                flags[direction],
            );
        }
    }
}

#[test]
fn meiko_nan_conversions_follow_the_chip_table_instead_of_generic_v8_defaults() {
    for (opf, inf, quiet, sign) in [
        (0xd1, 0x7f80_0000, 1 << 22, 1 << 31),
        (0xd2, 0x7ff0_0000_0000_0000, 1 << 51, 1 << 63),
    ] {
        convert(opf, inf, 0, 0x7fff_ffff, 16);
        convert(opf, inf | sign, 0, 0x8000_0000, 16);
        convert(opf, inf | quiet | 7, 0, 0x7fff_0000, 16);
        convert(opf, inf | quiet | 7 | sign, 0, 0x8000_0000, 16);
        convert(opf, inf | 7, 0, 0x7fff_ffff, 16);
        convert(opf, inf | 7 | sign, 0, 0x8000_0000, 16);
    }
    for sign in [0, 1u64 << 31] {
        convert(0xc9, 0x7fc0_0007 | sign, 0, 0x7ff8_0000_e000_0000, 0);
        convert(0xc9, 0x7f80_0007 | sign, 0, 0x7fff_e000_0000_0000, 16);
    }
    for sign in [0, 1u64 << 63] {
        convert(0xc6, 0x7ff8_0000_0000_0007 | sign, 0, 0x7fff_0000, 0);
        convert(0xc6, 0x7ff0_0000_0000_0007 | sign, 0, 0x7fff_0000, 16);
    }
}

#[test]
fn narrowing_and_to_integer_exceptions_inhibit_destinations_and_defer_the_enabled_cause() {
    for (opf, bits, mask, cause) in [
        (0xc6, 0x36a0_0000_0000_0000u64, 4, 4), // exact tiny, UFM enabled
        (0xc6, 0x3690_0000_0000_0000, 5, 4),
        (0xc6, 0x3690_0000_0000_0000, 1, 1),
        (0xc6, 0x7fef_ffff_ffff_ffff, 9, 8),
        (0xc6, 0x7fef_ffff_ffff_ffff, 1, 1),
        (0xc6, 0x7ff0_0000_0000_0007, 16, 16),
        (0xd2, 0x3ff8_0000_0000_0000, 1, 1),
        (0xd2, 0x7ff0_0000_0000_0000, 16, 16),
    ] {
        let word = 0x91a0_0005 | (opf << 5); // odd source aliases f4/f5
        let mut m = setup(&[word], (mask << 23) | FSR_SEED);
        m.cpu.fpu.write_reg(4, (bits >> 32) as u32);
        m.cpu.fpu.write_reg(5, bits as u32);
        m.cpu.fpu.write_reg(8, 0xdec0_adde);
        let before: Vec<_> = (0..32).map(|r| m.cpu.fpu.read_reg(r)).collect();
        m.step().unwrap();
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
                instruction: word
            })
        );
    }
}
