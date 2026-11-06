//! Shared arithmetic rules from SPARC V8 and microSPARC-II figure 4.8.

pub(super) const NX: u32 = 1;
pub(super) const DZ: u32 = 2;
pub(super) const UF: u32 = 4;
pub(super) const OF: u32 = 8;
pub(super) const NV: u32 = 16;

pub(super) fn propagate_nan(a: u64, b: u64, double: bool) -> Option<(u64, u32)> {
    let (sign, infinity, quiet) = if double {
        (1 << 63, 0x7ff0_0000_0000_0000, 1 << 51)
    } else {
        (1 << 31, 0x7f80_0000, 1 << 22)
    };
    let a = a & !sign;
    let b = b & !sign;
    let nan_a = a > infinity;
    let nan_b = b > infinity;
    if !nan_a && !nan_b {
        return None;
    }
    if (nan_a && a & quiet == 0) || (nan_b && b & quiet == 0) {
        // Any SNaN yields Meiko's positive canonical quiet NaN when masked.
        Some((
            if double {
                0x7fff_e000_0000_0000
            } else {
                0x7fff_0000
            },
            NV,
        ))
    } else {
        // First quiet operand wins, and all NaN result signs are cleared.
        Some((if nan_a { a } else { b }, 0))
    }
}

pub(super) fn overflow(infinity: u64, result_sign: u64, rounding: u32) -> (u64, u32) {
    let to_infinity =
        rounding == 0 || (rounding == 2 && result_sign == 0) || (rounding == 3 && result_sign != 0);
    (
        result_sign | if to_infinity { infinity } else { infinity - 1 },
        OF | NX,
    )
}
