use helios::devices::sx::{Sx, SX_BASE, SX_MEM_BASE, VSIMM_VRAM_BASE};

fn reg(sx: &mut Sx, r: usize, value: u32) {
    sx.write(SX_BASE + 0x100 + (r as u64) * 4, value, u32::MAX)
        .unwrap();
}

#[test]
fn captured_two_byte_vertical_outline_preserves_packed_tail_lanes() {
    let mut sx = Sx::new();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x10, u32::MAX, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x14, 0x55, u32::MAX).unwrap();
    reg(&mut sx, 2, 0);
    reg(&mut sx, 10, 0);
    reg(&mut sx, 11, 0);
    // Exact captured vertical-outline address at pixels (592,443)..(593,443).
    // Nonuniform X values make byte placement visible; B/G/R must survive.
    let pixel = 0x7cbd0usize;
    for (i, value) in [0x0122_3344, 0x89aa_bbcc, 0xdead_beef]
        .into_iter()
        .enumerate()
    {
        sx.write(VSIMM_VRAM_BASE + ((pixel + i) * 4) as u64, value, u32::MAX)
            .unwrap();
    }
    let mut expected = sx.vram_bytes().to_vec();
    expected[pixel * 4] = 0xfe;
    expected[(pixel + 1) * 4] = 0x76;
    sx.write(SX_MEM_BASE + 0xff07_cbd0, 0x80d7_d500, u32::MAX)
        .unwrap();
    sx.write(SX_BASE + 0x20, 0x9122_952a, u32::MAX).unwrap();
    sx.write(SX_MEM_BASE + 0xff07_cbd0, 0x808f_d500, u32::MAX)
        .unwrap();
    assert_eq!(
        sx.vram_word(pixel * 4),
        0xfe22_3344,
        "invert the first X byte"
    );
    assert_eq!(
        sx.vram_word((pixel + 1) * 4),
        0x76aa_bbcc,
        "invert the second X byte"
    );
    assert_eq!(sx.vram_bytes(), expected);
}

#[test]
fn ropm_uses_the_mask_selected_upper_truth_table_as_well_as_the_lower() {
    let mut sx = Sx::new();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x14, 0x3c, u32::MAX).unwrap();
    reg(&mut sx, 10, 0x1234_5678);
    reg(&mut sx, 42, 0xdead_beef);
    reg(&mut sx, 2, u32::MAX);
    // Sun's mask-controlled ROP, corroborated by NetBSD cg14_invert():
    // 0x3c is ~A / A for mask set / clear. Here only one register is used.
    sx.write(SX_BASE + 0x20, 0x9022_952a, u32::MAX).unwrap();
    assert_eq!(sx.register_word(0x100 + 42 * 4), 0xedcb_a987);
    reg(&mut sx, 2, 0);
    sx.write(SX_BASE + 0x20, 0x9022_952a, u32::MAX).unwrap();
    assert_eq!(sx.register_word(0x100 + 42 * 4), 0x1234_5678);
}
