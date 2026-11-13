use helios::devices::sx::{Sx, SX_BASE, SX_MEM_BASE, VSIMM_VRAM_BASE, WIDTH};

// Sun's SX data-format table: four packed bytes per register. This is a
// synthetic nonuniform source, using the exact captured address and LDP word.
const WORDS: [u32; 8] = [
    0x0123_4567,
    0x89ab_cdef,
    0x1032_5476,
    0x98ba_dcfe,
    0x80ff_007f,
    0x55aa_33cc,
    0xdeaf_bead,
    0x7654_3210,
];
const PIXEL: usize = 100 * WIDTH + 100;
const COMMAND_ADDRESS: u64 = SX_MEM_BASE + 0xff01_c260;

fn reg(sx: &mut Sx, r: usize, value: u32) {
    sx.write(SX_BASE + 0x100 + r as u64 * 4, value, u32::MAX)
        .unwrap();
}
fn registers(sx: &Sx) -> Vec<u32> {
    (0..128).map(|r| sx.register_word(0x100 + r * 4)).collect()
}
fn fixture() -> Sx {
    let mut sx = Sx::new();
    for r in 1..128 {
        reg(&mut sx, r, 0xfeed_0000 | r as u32);
    }
    reg(&mut sx, 2, 0); // Loads and non-conditional stores must ignore enables.
    sx.write(SX_BASE + 0x10, u32::MAX, u32::MAX).unwrap();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    for i in 0..32 {
        let x = (WORDS[i / 4] >> (24 - 8 * (i % 4))) & 0xff;
        sx.write(
            VSIMM_VRAM_BASE + ((PIXEL + i) * 4) as u64,
            (x << 24) | 0x0012_3400 | i as u32,
            u32::MAX,
        )
        .unwrap();
    }
    sx
}

#[test]
fn captured_ldp_reads_32_x32_bytes_into_eight_registers() {
    let mut sx = fixture();
    sx.write(SX_BASE + 0x10, 0, u32::MAX).unwrap();
    let mut expected = registers(&sx);
    expected[42..50].copy_from_slice(&WORDS);
    let vram = sx.vram_bytes().to_vec();
    sx.write(COMMAND_ADDRESS, 0x8fd7_d504, u32::MAX)
        .expect("captured SX_LDP R42,32 bytes,displacement 4");
    assert_eq!(registers(&sx), expected);
    assert_eq!(sx.vram_bytes(), vram);
}

#[test]
fn full_packed_load_fits_in_r120_through_r127() {
    let mut sx = fixture();
    let mut expected = registers(&sx);
    expected[120..128].copy_from_slice(&WORDS);
    // Same 32-byte load, destination R120: eight registers, not 32 registers.
    let command = (0x8fd7_d504 & !(0x7f << 7)) | (120 << 7);
    sx.write(COMMAND_ADDRESS, command, u32::MAX).unwrap();
    assert_eq!(registers(&sx), expected);
}

#[test]
fn next_ropm_55_inverts_source_b_in_place() {
    let mut sx = fixture();
    for r in 10..19 {
        reg(&mut sx, r, 0);
    }
    for (i, value) in WORDS.into_iter().chain([0xaabb_ccdd]).enumerate() {
        reg(&mut sx, 42 + i, value);
    }
    // NetBSD cgfourteen.c uses 0x3c for ~A / A; 0x55 is ~B for either
    // mask-selected nibble. This matches the captured ROP register value.
    sx.write(SX_BASE + 0x14, 0x55, u32::MAX).unwrap();
    let mut expected = registers(&sx);
    for value in &mut expected[42..51] {
        *value = !*value;
    }
    let vram = sx.vram_bytes().to_vec();
    // Derived from the original DDX at the instruction after the failed load.
    sx.write(SX_BASE + 0x20, 0x9822_952a, u32::MAX)
        .expect("ROP_M A=R10,B=D=R42,count=9");
    assert_eq!(registers(&sx), expected);
    assert_eq!(sx.vram_bytes(), vram);
}

#[test]
fn next_packed_masked_store_does_not_require_r2_enables() {
    let mut sx = fixture();
    for (i, value) in WORDS.into_iter().enumerate() {
        reg(&mut sx, 42 + i, !value);
    }
    let before = registers(&sx);
    let mut expected = sx.vram_bytes().to_vec();
    for i in 0..32 {
        expected[(PIXEL + i) * 4] = !expected[(PIXEL + i) * 4];
    }
    // Derived packed STORE|MASK; R2 is zero, and all plane-mask bits are set.
    sx.write(COMMAND_ADDRESS, 0x8f8f_d504, u32::MAX)
        .expect("packed masked store must write even with R2 clear");
    assert_eq!(registers(&sx), before);
    assert_eq!(sx.vram_bytes(), expected);
}

#[test]
fn outline_load_rop_store_inverts_only_the_selected_x_plane_bytes() {
    let mut sx = fixture();
    for r in 10..19 {
        reg(&mut sx, r, 0);
    }
    sx.write(SX_BASE + 0x14, 0x55, u32::MAX).unwrap();
    let mut expected = sx.vram_bytes().to_vec();
    for i in 0..32 {
        expected[(PIXEL + i) * 4] = !expected[(PIXEL + i) * 4];
    }
    sx.write(COMMAND_ADDRESS, 0x8fd7_d504, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x20, 0x9822_952a, u32::MAX).unwrap();
    sx.write(COMMAND_ADDRESS, 0x8f8f_d504, u32::MAX).unwrap();
    assert_eq!(sx.vram_bytes(), expected);
}

#[test]
fn packed_load_with_a_late_invalid_target_preserves_registers_and_vram() {
    let mut sx = fixture();
    let regs = registers(&sx);
    let vram = sx.vram_bytes().to_vec();
    // Sixteen legal bytes followed by sixteen outside the decoded aperture.
    assert!(sx
        .write(SX_MEM_BASE + 0xffff_fff0, 0x8fd7_d500, u32::MAX)
        .is_err());
    assert_eq!(registers(&sx), regs);
    assert_eq!(sx.vram_bytes(), vram);
}
