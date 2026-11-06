//! Exact integer products for binary32/binary64, rounded once to the destination.
//! SPARC V8 B.34/N.3-N.6 and microSPARC-II 4.1/4.3; see docs/fpu.md.

use super::ieee::{overflow, propagate_nan, NV, NX, UF};

pub(super) fn multiply(
    a: u64,
    b: u64,
    double: bool,
    rounding: u32,
    trap_underflow: bool,
) -> (u64, u32) {
    if let Some(result) = propagate_nan(a, b, double) {
        return result;
    }
    let (fraction_bits, bias) = if double { (52, 1023) } else { (23, 127) };
    let hidden = 1u64 << fraction_bits;
    let fraction = hidden - 1;
    let infinity = (2 * bias + 1) as u64 * hidden;
    let sign = (infinity | fraction) + 1;
    let result_sign = (a ^ b) & sign;
    let a = a & !sign;
    let b = b & !sign;
    if (a == 0 && b == infinity) || (a == infinity && b == 0) {
        // V8-recommended numeric-invalid default, as for division and addition.
        return (infinity | fraction, NV);
    }
    if a == infinity || b == infinity {
        return (result_sign | infinity, 0);
    }
    if a == 0 || b == 0 {
        return (result_sign, 0);
    }
    // Value = integer significand * 2^power, including subnormal operands.
    let decode = |bits: u64| {
        let field = (bits >> fraction_bits) as i32;
        let significand = (bits & fraction) | if field != 0 { hidden } else { 0 };
        (u128::from(significand), field.max(1) - bias - fraction_bits)
    };
    let (significand_a, power_a) = decode(a);
    let (significand_b, power_b) = decode(b);
    let product = significand_a * significand_b; // at most 106 bits, with no discarded tail
    let power = power_a + power_b;
    let mut exponent = power + (127 - product.leading_zeros()) as i32;
    let minimum = 1 - bias;
    let tiny = exponent < minimum;
    // Choose the normal/subnormal grid before rounding to avoid double rounding.
    let shift = exponent.max(minimum) - fraction_bits - power;
    let (mut rounded, inexact, half_order) = if shift <= 0 {
        (product << -shift, false, std::cmp::Ordering::Less)
    } else if shift >= 128 {
        // product has <=106 bits, hence is strictly below half this grid spacing.
        (0, true, std::cmp::Ordering::Less)
    } else {
        let remainder = product & ((1u128 << shift) - 1);
        (
            product >> shift,
            remainder != 0,
            remainder.cmp(&(1u128 << (shift - 1))),
        )
    };
    let increment = inexact
        && match rounding {
            0 => half_order.is_gt() || (half_order.is_eq() && rounded & 1 != 0),
            1 => false,
            2 => result_sign == 0,
            3 => result_sign != 0,
            _ => unreachable!("two-bit rounding direction"),
        };
    rounded += u128::from(increment);
    if !tiny && rounded == u128::from(hidden) * 2 {
        rounded >>= 1;
        exponent += 1;
    }
    if exponent > bias {
        return overflow(infinity, result_sign, rounding);
    }
    let bits = if tiny {
        rounded as u64 // includes rounding up to minimum normal
    } else {
        ((exponent + bias) as u64 * hidden) | (rounded as u64 & fraction)
    };
    let flags = if inexact { NX } else { 0 }
        | if tiny && (inexact || trap_underflow) {
            UF
        } else {
            0
        };
    (result_sign | bits, flags)
}
