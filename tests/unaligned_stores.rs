use helios::devices::sx::{Sx, SX_BASE, SX_MEM_BASE, VRAM_BYTES, VSIMM_VRAM_BASE};

fn fixture() -> Sx {
    let mut sx = Sx::new();
    sx.write(SX_BASE + 0x128, 0xaabb_ccdd, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x12c, 0x7788_99ee, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x10, u32::MAX, u32::MAX).unwrap();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    sx.write(VSIMM_VRAM_BASE, 0x1122_3344, u32::MAX).unwrap();
    sx.write(VSIMM_VRAM_BASE + 4, 0x5566_7788, u32::MAX)
        .unwrap();
    for offset in (VRAM_BYTES - 32..VRAM_BYTES).step_by(4) {
        sx.write(VSIMM_VRAM_BASE + offset as u64, 0x1122_3344, u32::MAX)
            .unwrap();
    }
    sx
}

fn unchanged(sx: &Sx, expected: &[u8]) {
    if let Some((offset, (actual, wanted))) = sx
        .vram_bytes()
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (actual, wanted))| actual != wanted)
    {
        panic!("VRAM changed at {offset:#x}: got {actual:#04x}, expected {wanted:#04x}");
    }
}

#[test]
fn unaligned_long_raw_store_crosses_word_boundary_and_preserves_guards() {
    let mut sx = fixture();
    let mut expected = sx.vram_bytes().to_vec();
    expected[2..6].copy_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
    // Store one plane-masked long from R10, displacement 2.
    sx.write(SX_MEM_BASE + 0xfc00_0000, 0x800e_c502, u32::MAX)
        .unwrap();
    unchanged(&sx, &expected);
}

#[test]
fn unaligned_long_bgr_store_preserves_x_when_crossing_word_boundary() {
    let mut sx = fixture();
    let mut expected = sx.vram_bytes().to_vec();
    expected[2] = 0xaa;
    expected[3] = 0xbb;
    // The X byte at raw offset 4 is write-protected in the BGR aperture.
    expected[5] = 0xdd;
    sx.write(SX_MEM_BASE + 0xfd00_0000, 0x800e_c502, u32::MAX)
        .unwrap();
    unchanged(&sx, &expected);
}

fn rejected_store(address: u64, word: u32) {
    let mut sx = fixture();
    let expected = sx.vram_bytes().to_vec();
    let registers: Vec<_> = (0..128).map(|r| sx.register_word(0x100 + r * 4)).collect();
    assert!(
        sx.write(SX_MEM_BASE + address, word, u32::MAX).is_err(),
        "a command extending past the decoded aperture must be rejected"
    );
    unchanged(&sx, &expected);
    for (r, value) in registers.into_iter().enumerate() {
        assert_eq!(sx.register_word(0x100 + r * 4), value, "R{r}");
    }
}

#[test]
fn invalid_tail_of_one_unaligned_long_store_does_not_mutate_vram() {
    // Effective PA 0xffffffff is valid for only its first byte. Subsequent
    // bytes are outside the aperture. The failure must preserve the first byte.
    // Unmasked store avoids a destination read rejecting the access first.
    rejected_store(0xffff_fff8, 0x8006_c507);
}

#[test]
fn invalid_tail_of_later_long_store_preserves_the_entire_vector() {
    // First long at PA 0xfffffffb is entirely valid; the second at 0xffffffff
    // crosses the end. Validate the complete command before writing either.
    rejected_store(0xffff_fff8, 0x808e_c503);
}

#[test]
fn direct_aperture_write_rejects_invalid_tail_before_mutating_vram() {
    let mut sx = fixture();
    let expected = sx.vram_bytes().to_vec();
    assert!(sx.write(0xffff_ffff, 0xaabb_ccdd, u32::MAX).is_err());
    unchanged(&sx, &expected);
}
