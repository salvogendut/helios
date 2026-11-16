use helios::bus::BusErrorKind;
use helios::devices::sx::{Sx, SX_BASE};

fn put(sx: &mut Sx, reg: usize, value: u32) {
    sx.write(SX_BASE + 0x100 + reg as u64 * 4, value, u32::MAX)
        .unwrap();
}

fn registers(sx: &Sx) -> Vec<u32> {
    (0..128).map(|r| sx.register_word(0x100 + 4 * r)).collect()
}

fn fixture(mask: u32, rop: u32) -> Sx {
    let mut sx = Sx::new();
    for reg in 0..128 {
        put(&mut sx, reg, 0xface_0000 | reg as u32);
    }
    put(&mut sx, 2, mask);
    sx.write(SX_BASE + 0x14, rop, u32::MAX).unwrap();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    sx
}

// NetBSD sxreg.h 1.22: counts encode length minus one, not byte length.
fn command(op: u32, a: u32, b: u32, dest: u32, length: u32) -> u32 {
    0x9000_0000 | ((length - 1) << 24) | (op << 21) | (a << 14) | (dest << 7) | b
}

#[test]
fn captured_netbsd_cursor_ropl_inverts_three_words_only() {
    let mut sx = fixture(u32::MAX, 0x3c);
    for (r, v) in [0x0011_2233, 0x4455_6677, 0x8899_aabb]
        .into_iter()
        .enumerate()
    {
        put(&mut sx, 8 + r, v);
    }
    let mut expected = registers(&sx);
    expected[40..43].copy_from_slice(&[0xffee_ddcc, 0xbbaa_9988, 0x7766_5544]);
    let vram = sx.vram_bytes().to_vec();
    // Captured at PC f002b7d8, SPARC ST word d622400a, IQ PA f80000020.
    sx.write(SX_BASE + 0x20, 0x9242_1408, u32::MAX).unwrap();
    assert_eq!(registers(&sx), expected);
    assert_eq!(sx.vram_bytes(), vram);
}

#[test]
fn netbsd_cursor_ropb_masks_bytes_from_the_most_significant_bit() {
    for (mask, result) in [
        (0x0000_0000, 0x1234_5678),
        (0x8000_0000, 0xed34_5678),
        (0x7000_0000, 0x12cb_a987),
        (0xa000_0000, 0xed34_a978),
        (0x0fff_ffff, 0x1234_5678),
        (0xffff_ffff, 0xedcb_a987),
    ] {
        let mut sx = fixture(mask, 0x3c);
        put(&mut sx, 8, 0x1234_5678);
        let mut expected = registers(&sx);
        expected[40] = result;
        sx.write(SX_BASE + 0x20, 0x9002_1408, u32::MAX).unwrap();
        assert_eq!(registers(&sx), expected, "mask {mask:08x}");
    }
}

#[test]
fn ropb_consumes_all_32_mask_bits_across_eight_registers() {
    let mut sx = fixture(0x8000_0001, 0x3c);
    for reg in 8..16 {
        put(&mut sx, reg, 0x1234_5678);
    }
    let mut expected = registers(&sx);
    expected[40..48].fill(0x1234_5678);
    expected[40] = 0xed34_5678;
    expected[47] = 0x1234_5687;
    sx.write(SX_BASE + 0x20, command(0, 8, 8, 40, 8), u32::MAX)
        .unwrap();
    assert_eq!(registers(&sx), expected);
}

#[test]
fn ropl_preserves_the_original_mask_when_destination_includes_r2() {
    let mut sx = fixture(0xa000_0000, 0x3c);
    put(&mut sx, 8, 0x0102_0304);
    put(&mut sx, 9, 0x1122_3344);
    put(&mut sx, 10, 0x5566_7788);
    let mut expected = registers(&sx);
    expected[2..5].copy_from_slice(&[0xfefd_fcfb, 0x1122_3344, 0xaa99_8877]);
    sx.write(SX_BASE + 0x1020, command(2, 8, 8, 2, 3), u32::MAX)
        .unwrap();
    assert_eq!(registers(&sx), expected);
}

// Independent Boolean forms for the sixteen two-input truth tables.
fn truth_tables(a: u32, b: u32) -> [u32; 16] {
    [
        0,
        !(a | b),
        !a & b,
        !a,
        a & !b,
        !b,
        a ^ b,
        !(a & b),
        a & b,
        !(a ^ b),
        b,
        !a | b,
        a,
        a | !b,
        a | b,
        u32::MAX,
    ]
}

#[test]
fn all_rop_modes_use_both_truth_table_nibbles_with_overlap_snapshots() {
    let a = [0x1122_3344, 0x0123_4567, 0x89ab_cdef];
    let b = [0x5555_5555, 0xcccc_cccc, 0x0f0f_0f0f];
    for op in 0..=2 {
        for upper in 0..16 {
            for lower in 0..16 {
                let mask = 0xa582_61e7;
                let mut sx = fixture(mask, (upper << 4) | lower);
                for i in 0..3 {
                    put(&mut sx, 8 + i, a[i]);
                    put(&mut sx, 24 + i, b[i]);
                }
                let mut expected = registers(&sx);
                // Per-byte/per-word selectors derived independently from the
                // fixed MASK pattern, while ROP_M repeats its bit pattern.
                let selects = match op {
                    0 => [0xff00_ff00, 0x00ff_00ff, 0xff00_0000],
                    1 => [mask; 3],
                    2 => [u32::MAX, 0, u32::MAX],
                    _ => unreachable!(),
                };
                for i in 0..3 {
                    let tables = truth_tables(a[i], b[i]);
                    expected[9 + i] = (tables[upper as usize] & selects[i])
                        | (tables[lower as usize] & !selects[i]);
                }
                sx.write(SX_BASE + 0x20, command(op, 8, 24, 9, 3), u32::MAX)
                    .unwrap();
                assert_eq!(registers(&sx), expected, "op {op} rop {upper:x}{lower:x}");
            }
        }
    }
}

#[test]
fn rop_failures_validate_all_ranges_before_mutation_and_queue_pairs_roll_back() {
    let mut sx = fixture(0x8000_0001, 0x3c);
    let before = registers(&sx);
    let vram = sx.vram_bytes().to_vec();
    for op in 0..=2 {
        for (a, b, d) in [(127, 8, 40), (8, 127, 40), (8, 8, 127)] {
            assert_eq!(
                sx.write(SX_BASE + 0x20, command(op, a, b, d, 2), u32::MAX),
                Err(BusErrorKind::Unsupported("SX ROP register range"))
            );
            assert_eq!(registers(&sx), before);
        }
    }
    assert_eq!(
        sx.write(SX_BASE + 0x20, command(0, 8, 8, 40, 9), u32::MAX),
        Err(BusErrorKind::Unsupported("SX byte ROP mask span"))
    );
    assert_eq!(registers(&sx), before);
    assert!(sx
        .write_doubleword(SX_BASE + 0x1020, [0x9242_1408, command(0, 8, 8, 127, 2)],)
        .is_err());
    assert_eq!(registers(&sx), before);
    assert_eq!(sx.vram_bytes(), vram);
}

#[test]
fn synchronous_queue_status_survives_control_writes_commands_and_reset() {
    let mut sx = fixture(u32::MAX, 0x3c);
    let status = |sx: &mut Sx| sx.read(SX_BASE, 4).unwrap();
    // NetBSD sx_attach enables page bounds and GO without setting MT.
    sx.write(SX_BASE, 0x1400, u32::MAX).unwrap();
    assert_eq!(status(&mut sx), 0x5400);
    assert_eq!(sx.read(SX_BASE + 0x1000, 4).unwrap(), 0x5400);
    for _ in 0..10 {
        // sx_wait polls MT after several instructions, including cursor ROPs.
        sx.write(SX_BASE + 0x20, 0x9242_1408, u32::MAX).unwrap();
        assert_eq!(status(&mut sx), 0x5400);
    }
    // Writing the hardware busy bit cannot fabricate a pending instruction.
    sx.write(SX_BASE, 0x9400, u32::MAX).unwrap();
    assert_eq!(status(&mut sx), 0x5400);
    sx.write(SX_BASE + 0x34, 0, u32::MAX).unwrap();
    assert_eq!(status(&mut sx), 0x5400);
    sx.write(SX_BASE, 0, u32::MAX).unwrap();
    assert_eq!(status(&mut sx), 0x4000);
    let before = registers(&sx);
    sx.write(SX_BASE + 0x20, 0x9242_1408, u32::MAX).unwrap();
    assert_eq!(registers(&sx), before); // GO is still a writable control.
    assert_eq!(status(&mut sx), 0x4000);
    sx.write(SX_BASE + 0x30, 0, u32::MAX).unwrap();
    assert_eq!(status(&mut sx), 0x4000);
    sx.reset();
    assert_eq!(status(&mut sx), 0x4000);
}
