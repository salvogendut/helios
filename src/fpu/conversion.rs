//! Integer and binary32/binary64 conversions, SPARC V8 B.34/N.4-N.7 and
//! microSPARC-II figure 4.9. No host floating-point state.

use super::ieee::{overflow, NV, NX, UF};

pub(super) fn from_integer(word: u32, double: bool, rounding: u32) -> (u64, u32) {
    let value = i64::from(word as i32);
    round_scaled(value.unsigned_abs(), 0, value < 0, double, rounding, false)
}

pub(super) fn to_integer(bits: u64, double: bool) -> (u64, u32) {
    let (fraction_bits, bias, sign_bit) = format(double);
    let hidden = 1u64 << fraction_bits;
    let fraction = hidden - 1;
    let negative = bits >> sign_bit != 0;
    let magnitude = bits & ((1 << sign_bit) - 1);
    let infinity = (2 * bias + 1) as u64 * hidden;
    let limit = if negative { 0x8000_0000 } else { 0x7fff_ffff };
    if magnitude >= infinity {
        // Meiko's positive quiet NaN produces the 32-bit ME_NaN word even
        // for FDTOI. Other invalid inputs saturate according to operand sign.
        let positive_quiet_nan = !negative && magnitude > infinity && bits & (hidden >> 1) != 0;
        return (
            if positive_quiet_nan {
                0x7fff_0000
            } else {
                limit
            },
            NV,
        );
    }
    let field = (magnitude >> fraction_bits) as i32;
    if field - bias > 31 {
        return (limit, NV);
    }
    let significand = (bits & fraction) | if field != 0 { hidden } else { 0 };
    let shift = fraction_bits + bias - field.max(1);
    let (integer, inexact) = if shift <= 0 {
        (significand << -shift, false)
    } else if shift >= 64 {
        (0, significand != 0)
    } else {
        (
            significand >> shift,
            significand & ((1u64 << shift) - 1) != 0,
        )
    };
    // Range is checked after truncating: -2147483648.75 is valid and inexact.
    if integer > limit {
        return (limit, NV);
    }
    let word = if negative {
        0u32.wrapping_sub(integer as u32)
    } else {
        integer as u32
    };
    (u64::from(word), if inexact { NX } else { 0 })
}

pub(super) fn between_formats(
    bits: u64,
    double_result: bool,
    rounding: u32,
    trap_underflow: bool,
) -> (u64, u32) {
    let (source_fraction, source_bias, source_sign) = format(!double_result);
    let (result_fraction, result_bias, result_sign) = format(double_result);
    let hidden = 1u64 << source_fraction;
    let fraction = bits & (hidden - 1);
    let magnitude = bits & ((1 << source_sign) - 1);
    let infinity = (2 * source_bias + 1) as u64 * hidden;
    let result_infinity = ((2 * result_bias + 1) as u64) << result_fraction;
    let negative = bits >> source_sign != 0;
    if magnitude > infinity {
        let signaling = fraction & (hidden >> 1) == 0;
        // Figure 4.9: FSTOD preserves quiet payloads; FDTOS canonicalizes all
        // NaNs. Signaling operands canonicalize in both directions and raise NV.
        let nan = if !double_result {
            0x7fff_0000
        } else if signaling {
            0x7fff_e000_0000_0000
        } else {
            result_infinity | (fraction << (result_fraction - source_fraction))
        };
        return (nan, if signaling { NV } else { 0 });
    }
    if magnitude == infinity {
        return ((u64::from(negative) << result_sign) | result_infinity, 0);
    }
    let field = (magnitude >> source_fraction) as i32;
    let significand = fraction | if field != 0 { hidden } else { 0 };
    round_scaled(
        significand,
        field.max(1) - source_bias - source_fraction,
        negative,
        double_result,
        rounding,
        trap_underflow,
    )
}

fn format(double: bool) -> (i32, i32, u32) {
    if double {
        (52, 1023, 63)
    } else {
        (23, 127, 31)
    }
}

/// Round the exact value magnitude * 2^power once onto the destination grid.
fn round_scaled(
    magnitude: u64,
    power: i32,
    negative: bool,
    double: bool,
    rounding: u32,
    trap_underflow: bool,
) -> (u64, u32) {
    let (fraction_bits, bias, sign_bit) = format(double);
    let sign = u64::from(negative) << sign_bit;
    if magnitude == 0 {
        return (sign, 0);
    }
    let hidden = 1u64 << fraction_bits;
    let mut exponent = (63 - magnitude.leading_zeros()) as i32 + power;
    let minimum = 1 - bias;
    let tiny = exponent < minimum;
    let shift = exponent.max(minimum) - fraction_bits - power;
    let (mut significand, inexact, half_order) = if shift <= 0 {
        (magnitude << -shift, false, std::cmp::Ordering::Less)
    } else if shift >= 64 {
        // The input has at most 53 bits, so is strictly below half a unit.
        (0, true, std::cmp::Ordering::Less)
    } else {
        let quotient = magnitude >> shift;
        let remainder = magnitude & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        (quotient, remainder != 0, remainder.cmp(&half))
    };
    let increment = inexact
        && match rounding {
            0 => half_order.is_gt() || (half_order.is_eq() && significand & 1 != 0),
            1 => false,
            2 => !negative,
            3 => negative,
            _ => unreachable!("two-bit rounding direction"),
        };
    significand += u64::from(increment);
    if !tiny && significand == hidden * 2 {
        significand >>= 1;
        exponent += 1;
    }
    if exponent > bias {
        return overflow((2 * bias + 1) as u64 * hidden, sign, rounding);
    }
    let bits = if tiny {
        significand
    } else {
        ((exponent + bias) as u64 * hidden) | (significand & (hidden - 1))
    };
    let flags = if inexact { NX } else { 0 }
        | if tiny && (inexact || trap_underflow) {
            UF
        } else {
            0
        };
    (sign | bits, flags)
}
