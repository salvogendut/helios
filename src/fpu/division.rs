//! Exact quotient/remainder division for binary32/binary64. No host FP state.
//! SPARC V8 B.34/N.4-N.6 and microSPARC-II 4.3; see docs/fpu.md.

pub(super) const NX: u32 = 1;
pub(super) const DZ: u32 = 2;
pub(super) const UF: u32 = 4;
pub(super) const OF: u32 = 8;
pub(super) const NV: u32 = 16;

/// Returns raw result bits and the five CEXC bits. Exact tiny results signal
/// underflow only when UFM is enabled (V8 N.5/N.6).
pub(super) fn divide(
    a: u64,
    b: u64,
    double: bool,
    rounding: u32,
    trap_underflow: bool,
) -> (u64, u32) {
    let (fraction_bits, bias) = if double { (52, 1023) } else { (23, 127) };
    let hidden = 1u64 << fraction_bits;
    let fraction = hidden - 1;
    let infinity = (2 * bias + 1) as u64 * hidden;
    let sign = (infinity | fraction) + 1;
    let result_sign = (a ^ b) & sign;
    let magnitude_a = a & !sign;
    let magnitude_b = b & !sign;
    let nan_a = magnitude_a > infinity;
    let nan_b = magnitude_b > infinity;
    if nan_a || nan_b {
        let quiet = hidden >> 1;
        if (nan_a && a & quiet == 0) || (nan_b && b & quiet == 0) {
            // Meiko replaces any SNaN with its canonical positive quiet NaN.
            return (
                if double {
                    0x7fff_e000_0000_0000
                } else {
                    0x7fff_0000
                },
                NV,
            );
        }
        // Figure 4.8 prefers rs1 when both are quiet; clear the sign.
        return (if nan_a { magnitude_a } else { magnitude_b }, 0);
    }
    if (magnitude_a == 0 && magnitude_b == 0)
        || (magnitude_a == infinity && magnitude_b == infinity)
    {
        // V8 N.4's positive all-fraction-ones default for invalid numeric inputs.
        return (infinity | fraction, NV);
    }
    if magnitude_a == infinity {
        return (result_sign | infinity, 0); // infinity / zero is not finite / zero
    }
    if magnitude_b == infinity {
        return (result_sign, 0);
    }
    if magnitude_b == 0 {
        return (result_sign | infinity, DZ);
    }
    if magnitude_a == 0 {
        return (result_sign, 0);
    }

    // Normalize even subnormal operands to a p-bit significand, with value
    // significand * 2^(exponent - fraction_bits).
    let normalize = |bits: u64| {
        let field = (bits >> fraction_bits) as i32;
        let mut significand = bits & fraction;
        let mut exponent = if field == 0 { 1 - bias } else { field - bias };
        if field != 0 {
            significand |= hidden;
        }
        let shift = significand.leading_zeros() - (63 - fraction_bits);
        significand <<= shift;
        exponent -= shift as i32;
        (u128::from(significand), exponent)
    };
    let (mut numerator, exponent_a) = normalize(magnitude_a);
    let (denominator, exponent_b) = normalize(magnitude_b);
    let mut exponent = exponent_a - exponent_b;
    if numerator < denominator {
        numerator <<= 1;
        exponent -= 1;
    }
    // numerator/denominator is in [1,2). Round directly onto the destination
    // grid: never round once normally and then again for a subnormal result.
    let minimum = 1 - bias;
    let tiny = exponent < minimum;
    let shift = fraction_bits as i32 + (exponent - minimum).min(0);
    let (mut quotient, inexact, half_order) = if shift < -1 {
        // Positive magnitude below half the minimum subnormal. Avoid a huge
        // denominator shift; directed rounding can still produce one ulp.
        (0, true, std::cmp::Ordering::Less)
    } else {
        let (n, d) = if shift < 0 {
            (numerator, denominator << 1)
        } else {
            (numerator << shift, denominator)
        };
        // At most 106 significant bits for binary64; all intermediates fit u128.
        let remainder = n % d;
        (n / d, remainder != 0, (remainder * 2).cmp(&d))
    };
    let increment = inexact
        && match rounding {
            0 => half_order.is_gt() || (half_order.is_eq() && quotient & 1 != 0),
            1 => false,
            2 => result_sign == 0,
            3 => result_sign != 0,
            _ => unreachable!("two-bit rounding direction"),
        };
    quotient += u128::from(increment);
    if !tiny && quotient == u128::from(hidden) * 2 {
        quotient >>= 1;
        exponent += 1;
    }
    if exponent > bias {
        // Overflow is detected after rounding with an unbounded exponent.
        let to_infinity = rounding == 0
            || (rounding == 2 && result_sign == 0)
            || (rounding == 3 && result_sign != 0);
        return (
            result_sign | if to_infinity { infinity } else { infinity - 1 },
            OF | NX,
        );
    }
    let bits = if tiny {
        // Includes rounding up to minimum normal (encoded as hidden).
        quotient as u64
    } else {
        ((exponent + bias) as u64 * hidden) | (quotient as u64 & fraction)
    };
    let flags = if inexact { NX } else { 0 }
        | if tiny && (inexact || trap_underflow) {
            UF
        } else {
            0
        };
    (result_sign | bits, flags)
}
