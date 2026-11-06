//! Integer binary32/binary64 addition and subtraction, with guard/round/sticky bits.
//! SPARC V8 B.34/N.3-N.6; see docs/fpu.md for the arithmetic contract.

use super::ieee::{overflow, propagate_nan, NV, NX, UF};

/// Right shift, retaining every discarded nonzero bit in the low sticky bit.
fn shift_right_jam(value: u64, distance: u32) -> u64 {
    match distance {
        0 => value,
        1..=63 => (value >> distance) | u64::from(value << (64 - distance) != 0),
        _ => u64::from(value != 0),
    }
}

pub(super) fn add(
    a: u64,
    b: u64,
    double: bool,
    subtract: bool,
    rounding: u32,
    trap_underflow: bool,
) -> (u64, u32) {
    if let Some(result) = propagate_nan(a, b, double) {
        return result;
    }
    let (fraction_bits, bias) = if double { (52, 1023) } else { (23, 127) };
    let hidden = 1u64 << fraction_bits;
    let infinity = (2 * bias + 1) << fraction_bits;
    let sign = (infinity | (hidden - 1)) + 1;
    let mut sign_a = a & sign;
    let mut sign_b = (b ^ if subtract { sign } else { 0 }) & sign;
    let mut a = a & !sign;
    let mut b = b & !sign;
    if a == infinity || b == infinity {
        if a == infinity && b == infinity && sign_a != sign_b {
            // Same V8-recommended numeric-invalid default as 0/0 and infinity/infinity.
            return (infinity | (hidden - 1), NV);
        }
        return (infinity | if a == infinity { sign_a } else { sign_b }, 0);
    }
    // Larger magnitude first: its sign is the result's, except exact cancellation.
    if a < b {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut sign_a, &mut sign_b);
    }
    let decode = |bits: u64| {
        let field = (bits >> fraction_bits) as u32;
        (
            (bits & (hidden - 1)) | if field != 0 { hidden } else { 0 },
            field.max(1),
        )
    };
    let (large, mut exponent) = decode(a);
    let (small, small_exponent) = decode(b);
    let large = large << 3;
    let small = shift_right_jam(small << 3, exponent - small_exponent);
    let mut sum = if sign_a == sign_b {
        large + small
    } else {
        large - small
    };
    if sum == 0 {
        // Equal-sign zeros retain their sign; exact cancellation is -0 only for RD=-infinity.
        return (
            if sign_a == sign_b {
                sign_a
            } else if rounding == 3 {
                sign
            } else {
                0
            },
            0,
        );
    }
    let normal = hidden << 3;
    if sum >= normal << 1 {
        sum = shift_right_jam(sum, 1);
        exponent += 1;
    } else if sum < normal {
        // Severe cancellation only occurs for close exponents, where alignment
        // was exact. With jammed bits cancellation needs at most one left shift;
        // the three low bits retain sufficient rounding information either way.
        let shift = (sum.leading_zeros() - normal.leading_zeros()).min(exponent - 1);
        sum <<= shift;
        exponent -= shift;
    }
    let tiny = sum < normal; // before rounding, with exponent clamped to 1
    let tail = sum & 7;
    let mut rounded = sum >> 3;
    let increment = match rounding {
        0 => tail > 4 || (tail == 4 && rounded & 1 != 0),
        1 => false,
        2 => tail != 0 && sign_a == 0,
        3 => tail != 0 && sign_a != 0,
        _ => unreachable!("two-bit rounding direction"),
    };
    rounded += u64::from(increment);
    if rounded == hidden << 1 {
        rounded >>= 1;
        exponent += 1;
    }
    if u64::from(exponent) > 2 * bias {
        return overflow(infinity, sign_a, rounding);
    }
    let bits = if rounded < hidden {
        rounded
    } else {
        (u64::from(exponent) << fraction_bits) | (rounded & (hidden - 1))
    };
    // Same-format add/sub tiny results are exact multiples of a minimum
    // subnormal; with UFM enabled even these exact nonzero results trap.
    let flags = if tail != 0 { NX } else { 0 }
        | if tiny && (tail != 0 || trap_underflow) {
            UF
        } else {
            0
        };
    (sign_a | bits, flags)
}
