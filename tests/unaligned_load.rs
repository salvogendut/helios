use helios::devices::sx::{Sx, SX_BASE, SX_MEM_BASE, VSIMM_VRAM_BASE};

const PIXEL: usize = 0x1c278;
const X_COMMAND: u64 = SX_MEM_BASE + 0xff01_c278;

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
        reg(&mut sx, r, 0x1122_3300 | r as u32);
    }
    for r in 10..19 {
        reg(&mut sx, r, 0);
    }
    sx.write(SX_BASE + 0x10, u32::MAX, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x14, 0x55, u32::MAX).unwrap();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    for i in 0..64 {
        let x = ((i * 7 + 1) & 0xff) as u32;
        let b = ((i * 13 + 0x42) & 0xff) as u32;
        sx.write(
            VSIMM_VRAM_BASE + (PIXEL + i) as u64 * 4,
            (x << 24) | (b << 16) | 0x0000_a5c3,
            u32::MAX,
        )
        .unwrap();
    }
    sx
}

fn same_vram(sx: &Sx, expected: &[u8], context: &str) {
    if let Some((offset, (actual, wanted))) = sx
        .vram_bytes()
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (actual, wanted))| actual != wanted)
    {
        panic!("{context}: VRAM byte {offset:#x}: got {actual:#04x}, expected {wanted:#04x}");
    }
}

// Exact words/PA from accepted-run3 trace: move1 RELEASE (lane 1),
// move2 RELEASE (lane 3), and move3 RELEASE (displacement 6, lane 2).
fn outline(sx: &mut Sx, displacement: u32) {
    sx.write(X_COMMAND, 0x8fd7_d500 | displacement, u32::MAX)
        .unwrap();
    sx.write(SX_BASE + 0x1020, 0x9822_952a, u32::MAX).unwrap();
    sx.write(X_COMMAND, 0x8f8f_d500 | displacement, u32::MAX)
        .unwrap();
}

#[test]
fn captured_unaligned_outline_inverts_only_requested_x_bytes() {
    for displacement in [1, 3, 6] {
        let mut sx = fixture();
        let mut expected = sx.vram_bytes().to_vec();
        for i in 0..32 {
            expected[(PIXEL + displacement as usize + i) * 4] ^= 0xff;
        }
        outline(&mut sx, displacement);
        same_vram(
            &sx,
            &expected,
            &format!("captured outline displacement {displacement}"),
        );
    }
}

#[test]
fn drawing_then_erasing_unaligned_outline_restores_every_pixel() {
    let mut sx = fixture();
    let expected = sx.vram_bytes().to_vec();
    outline(&mut sx, 3);
    outline(&mut sx, 3);
    same_vram(&sx, &expected, "draw then erase at captured lane 3");
}

#[test]
fn aligned_outline_remains_a_passing_control() {
    let mut sx = fixture();
    let expected = sx.vram_bytes().to_vec();
    outline(&mut sx, 0);
    outline(&mut sx, 0);
    same_vram(&sx, &expected, "aligned control");
}

#[test]
fn unaligned_32_byte_load_uses_nine_registers_and_preserves_requested_byte_lanes() {
    for first in [42, 119] {
        let mut sx = fixture();
        let before = registers(&sx);
        let vram = sx.vram_bytes().to_vec();
        let word = (0x8fd7_d501 & !(127 << 7)) | ((first as u32) << 7);
        sx.write(X_COMMAND, word, u32::MAX).unwrap();
        for i in 0..32 {
            let position = 1 + i;
            let word = sx.register_word(0x100 + (first + position / 4) * 4);
            let actual = ((word >> (24 - (position % 4) * 8)) & 0xff) as u8;
            assert_eq!(
                actual,
                vram[(PIXEL + 1 + i) * 4],
                "source byte {i} must occupy lane {}",
                position % 4
            );
        }
        let after = registers(&sx);
        for r in 0..128 {
            if !(first..first + 9).contains(&r) {
                assert_eq!(after[r], before[r], "neighbor R{r}");
            }
        }
        // No expectation is imposed on unused prefix/suffix bytes inside
        // the nine affected registers; their hardware padding is not established.
        same_vram(&sx, &vram, "packed load must not change VRAM");
    }
}

#[test]
fn unaligned_load_that_needs_r128_fails_before_mutating_registers() {
    let mut sx = fixture();
    let before = registers(&sx);
    let vram = sx.vram_bytes().to_vec();
    let command = (0x8fd7_d501 & !(127 << 7)) | (120 << 7);
    assert!(
        sx.write(X_COMMAND, command, u32::MAX).is_err(),
        "R120 plus nine packed words exceeds the supported register range"
    );
    assert_eq!(registers(&sx), before);
    same_vram(&sx, &vram, "rejected packed load");
}

// Original sl_rect_copyrl_8: same source/destination lane uses load/store
// directly; positive lane difference loads R10 and funnels by 8*diff;
// negative difference loads R11, funnels R10 by 32+8*diff. These are
// synthetic operand combinations exercising that original DDX algorithm.
#[test]
fn ddx_copy_preserves_bytes_for_all_source_and_destination_alignments() {
    for source_lane in 0..4usize {
        for dest_lane in 0..4usize {
            for count in [1, 3, 14, 31, 32usize] {
                let mut sx = fixture();
                let source = PIXEL + source_lane;
                let dest = PIXEL + 8 + dest_lane;
                let mut expected = sx.vram_bytes().to_vec();
                for i in 0..count {
                    expected[(dest + i) * 4 + 1] = sx.vram_byte((source + i) * 4 + 1);
                }
                let difference = source_lane as i32 - dest_lane as i32;
                let load_reg = if difference < 0 { 11 } else { 10 };
                let load = 0x8057_c000
                    | (((count - 1) as u32) << 23)
                    | (load_reg << 7)
                    | (source as u32 & 7);
                sx.write(
                    SX_MEM_BASE + 0xff40_0000 + (source as u64 & !7),
                    load,
                    u32::MAX,
                )
                .unwrap();
                if difference != 0 {
                    let shift = if difference < 0 {
                        32 + difference * 8
                    } else {
                        difference * 8
                    };
                    sx.write(SX_BASE + 0x1020, 0xc8e2_8500 | shift as u32, u32::MAX)
                        .unwrap();
                }
                let store =
                    0x800f_c000 | (((count - 1) as u32) << 23) | (10 << 7) | (dest as u32 & 7);
                sx.write(
                    SX_MEM_BASE + 0xff40_0000 + (dest as u64 & !7),
                    store,
                    u32::MAX,
                )
                .unwrap();
                same_vram(
                    &sx,
                    &expected,
                    &format!(
                        "source lane {source_lane}, destination lane {dest_lane}, {count} bytes"
                    ),
                );
            }
        }
    }
}
