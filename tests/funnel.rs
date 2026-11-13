use helios::devices::sx::{Sx, SX_BASE};

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
        reg(&mut sx, r, 0x8192_0304u32.wrapping_mul(r as u32));
    }
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    sx
}

// NetBSD sxreg.h rev 1.22 SX_FUNNEL_I/S; the encoded count is n - 1.
fn funnel(a: u32, b: u32, d: u32, n: u32, immediate: bool) -> u32 {
    0xc000_0000 | ((n - 1) << 24) | ((6 + u32::from(immediate)) << 21) | (a << 14) | (d << 7) | b
}

// Independent bit-stream oracle: concatenate source words in bit significance
// order, discard the leading shift bits, and collect n words. No per-word
// funnel implementation or host shift-by-32 is used to obtain the expectation.
fn stream_shift(source: &[u32], shift: usize, n: usize) -> Vec<u32> {
    let bits: Vec<u32> = source
        .iter()
        .flat_map(|word| (0..32).rev().map(move |bit| (word >> bit) & 1))
        .collect();
    bits[shift..shift + n * 32]
        .chunks_exact(32)
        .map(|chunk| chunk.iter().fold(0, |word, bit| (word << 1) | bit))
        .collect()
}

fn check(sx: &mut Sx, instruction: u32, a: usize, d: usize, shift: usize, n: usize) {
    let before = registers(sx);
    let mut expected = before.clone();
    expected[d..d + n].copy_from_slice(&stream_shift(&before[a..a + n + 1], shift, n));
    let vram = sx.vram_bytes().to_vec();
    sx.write(SX_BASE + 0x1020, instruction, u32::MAX)
        .expect("funnel shifts across adjacent source elements");
    assert_eq!(
        registers(sx),
        expected,
        "outputs and all neighboring registers"
    );
    assert_eq!(sx.vram_bytes(), vram);
}

#[test]
fn captured_immediate_shifts_nine_words_in_place() {
    let mut sx = fixture();
    for i in 0..10 {
        let first = (i * 4) as u8;
        reg(
            &mut sx,
            10 + i,
            u32::from_be_bytes([first, first + 1, first + 2, first + 3]),
        );
    }
    assert_eq!(funnel(10, 24, 10, 9, true), 0xc8e2_8518);
    check(&mut sx, 0xc8e2_8518, 10, 10, 24, 9);
    assert_eq!(sx.register_word(0x128), 0x0304_0506);
    assert_eq!(sx.register_word(0x148), 0x2324_2526);
}

#[test]
fn byte_alignment_shifts_cross_nonuniform_register_boundaries() {
    for shift in [8, 16, 24] {
        let mut sx = fixture();
        check(
            &mut sx,
            funnel(10, shift, 40, 9, true),
            10,
            40,
            shift as usize,
            9,
        );
    }
}

#[test]
fn zero_and_bit_edge_shifts_do_not_shift_a_host_word_by_32() {
    for shift in [0, 1, 31] {
        let mut sx = fixture();
        check(
            &mut sx,
            funnel(10, shift, 40, 4, true),
            10,
            40,
            shift as usize,
            4,
        );
    }
}

#[test]
fn overlapping_destination_uses_original_source_vector() {
    let mut sx = fixture();
    check(&mut sx, funnel(10, 24, 11, 9, true), 10, 11, 24, 9);
}

#[test]
fn scalar_shift_is_snapshotted_before_overwriting_its_register() {
    let mut sx = fixture();
    reg(&mut sx, 10, 24);
    check(&mut sx, funnel(40, 10, 10, 9, false), 40, 10, 24, 9);
}

#[test]
fn final_source_pair_can_read_through_r127() {
    let mut sx = fixture();
    check(&mut sx, funnel(118, 24, 40, 9, true), 118, 40, 24, 9);
}
