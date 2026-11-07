//! Exact binary32/binary64 square root, rounded once to the destination.
//! SPARC V8 B.34/N.4 and microSPARC-II 4.3; see docs/fpu.md.

use super::ieee::{propagate_nan, NV, NX};
use std::cmp::Ordering;

/// Returns raw result bits and the applicable CEXC bits.
pub(super) fn square_root(a: u64, double: bool, rounding: u32) -> (u64, u32) {
    let (fraction_bits, bias) = if double { (52, 1023) } else { (23, 127) };
    let hidden = 1u64 << fraction_bits;
    let fraction = hidden - 1;
    let infinity = (2 * bias + 1) as u64 * hidden;
    let sign = (infinity | fraction) + 1;
    let magnitude = a & !sign;
    if let Some(result) = propagate_nan(a, a, double) {
        return result;
    }
    if a & sign != 0 {
        if magnitude == 0 {
            return (a, 0); // sqrt(-0) is -0.
        }
        // Invalid numeric input uses V8's positive all-fraction-ones default.
        return (infinity | fraction, NV);
    }
    if magnitude == 0 {
        return (0, 0);
    }
    if magnitude == infinity {
        return (infinity, 0);
    }

    // value = significand * 2^exponent. Subnormal inputs keep their full
    // integer significand and use the minimum exponent.
    let field = (magnitude >> fraction_bits) as i32;
    let significand = if field == 0 {
        magnitude & fraction
    } else {
        (magnitude & fraction) | hidden
    };
    let exponent = if field == 0 {
        1 - bias - fraction_bits
    } else {
        field - bias - fraction_bits
    };
    let value_exponent = exponent + (63 - significand.leading_zeros()) as i32;
    let result_exponent = value_exponent.div_euclid(2);
    let minimum_exponent = 1 - bias;
    let normal = result_exponent >= minimum_exponent;
    let grid_exp = if normal {
        result_exponent
    } else {
        minimum_exponent
    };
    let scale = grid_exp - fraction_bits;
    // Compare q^2 * 2^(2*scale) against significand * 2^exponent.
    let shift = exponent - 2 * scale;
    let limit = u128::from(if normal { 2 * hidden } else { hidden });
    let mut low = 0u128;
    let mut high = limit;
    while low < high {
        let middle = (low + high).div_ceil(2);
        if cmp_scaled(u128::from(significand), middle * middle, shift) != Ordering::Less {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let mut quotient = low;
    let exact = cmp_scaled(u128::from(significand), quotient * quotient, shift) == Ordering::Equal;
    let half = 2 * quotient + 1;
    let half_order = cmp_scaled(4 * u128::from(significand), half * half, shift);
    let increment = !exact
        && match rounding {
            0 => {
                half_order == Ordering::Greater
                    || (half_order == Ordering::Equal && quotient & 1 != 0)
            }
            1 => false,
            2 => true,
            3 => false,
            _ => unreachable!("two-bit rounding direction"),
        };
    quotient += u128::from(increment);
    let mut grid_exp = grid_exp;
    if normal && quotient == 2 * u128::from(hidden) {
        quotient >>= 1;
        grid_exp += 1;
    }
    if grid_exp > bias {
        return (
            if rounding == 1 {
                infinity - 1
            } else {
                infinity
            },
            NX | 8,
        );
    }
    let bits = if normal {
        (((grid_exp + bias) as u64) << fraction_bits) | (quotient as u64 & fraction)
    } else {
        quotient as u64
    };
    (bits, if exact { 0 } else { NX })
}

fn cmp_scaled(a: u128, b: u128, shift: i32) -> Ordering {
    if shift >= 0 {
        a.checked_shl(shift as u32)
            .map_or(Ordering::Greater, |v| v.cmp(&b))
    } else {
        b.checked_shl(shift.unsigned_abs())
            .map_or(Ordering::Less, |v| a.cmp(&v))
    }
}
