use helios::devices::sx::{
    Sx, SX_BASE, SX_MEM_BASE, SX_VRAM_ALIAS_BASE, VRAM_APERTURE_BYTES, VRAM_BYTES, VSIMM_VRAM_BASE,
    WIDTH,
};

fn reg(sx: &mut Sx, index: usize, value: u32) {
    sx.write(SX_BASE + 0x100 + index as u64 * 4, value, u32::MAX)
        .unwrap();
}

fn read_reg(sx: &Sx, index: usize) -> u32 {
    sx.register_word(0x100 + index * 4)
}

fn start(sx: &mut Sx) {
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
}

fn assert_vram(sx: &Sx, expected: &[u8]) {
    for (offset, &value) in expected.iter().enumerate() {
        assert_eq!(sx.vram_byte(offset), value, "VRAM byte {offset:#x}");
    }
}

#[test]
fn captured_byte_load_zero_extends_b32_pixel_into_r10() {
    let mut sx = Sx::new();
    let source_pixel = 116 * WIDTH + 103;
    sx.write(
        VSIMM_VRAM_BASE + source_pixel as u64 * 4,
        0xaa81_ccdd,
        u32::MAX,
    )
    .unwrap();
    reg(&mut sx, 9, 0x9999_9999);
    reg(&mut sx, 10, 0xaaaa_aaaa);
    reg(&mut sx, 11, 0xbbbb_bbbb);
    // Loads must ignore store enable and plane masks.
    reg(&mut sx, 2, 0);
    sx.write(SX_BASE + 0x10, 0, u32::MAX).unwrap();
    start(&mut sx);
    let before = sx.vram_bytes().to_vec();
    sx.write(SX_MEM_BASE + 0xff42_0a60, 0x8050_0507, u32::MAX)
        .expect("captured SX_LDB R10,count=1,displacement=7");
    assert_eq!(read_reg(&sx, 10), 0x81);
    assert_eq!(read_reg(&sx, 9), 0x9999_9999);
    assert_eq!(read_reg(&sx, 11), 0xbbbb_bbbb);
    assert_vram(&sx, &before);
}

#[test]
fn byte_load_into_r32_is_direct_and_fills_one_register_per_byte() {
    let mut sx = Sx::new();
    sx.write(VSIMM_VRAM_BASE + 4, 0x1122_3380, u32::MAX)
        .unwrap();
    sx.write(VSIMM_VRAM_BASE + 8, 0xfe00_7f44, u32::MAX)
        .unwrap();
    for index in 1..128 {
        reg(&mut sx, index, 0xaabb_ccdd);
    }
    reg(&mut sx, 2, 0);
    sx.write(SX_BASE + 0x10, 0, u32::MAX).unwrap();
    start(&mut sx);
    let before = sx.vram_bytes().to_vec();
    // SX_LDB(32, 3, 7): four unsigned bytes, crossing the eight-byte boundary.
    // Operation 10 includes bit 3, but that is not STORE_SELECT for a load.
    sx.write(SX_VRAM_ALIAS_BASE, 0x81d0_1007, u32::MAX)
        .expect("direct byte vector load into R32");
    let expected = [0x80, 0xfe, 0, 0x7f];
    for index in 0..128 {
        let value = match index {
            0 | 2 => 0,
            32..=35 => expected[index - 32],
            _ => 0xaabb_ccdd,
        };
        assert_eq!(read_reg(&sx, index), value, "R{index}");
    }
    assert_vram(&sx, &before);
}

#[test]
fn bulk_long_load_copies_128_b32_pixels_through_r10_to_r41() {
    let mut sx = Sx::new();
    let source_pixel = 116 * WIDTH + 104;
    let dest_pixel = 103 * WIDTH + 104;
    for index in 0..128 {
        let color = ((index * 37 + 0x81) & 0xff) as u32;
        sx.write(
            VSIMM_VRAM_BASE + (source_pixel + index) as u64 * 4,
            0xaa00_ccdd | (color << 16),
            u32::MAX,
        )
        .unwrap();
        sx.write(
            VSIMM_VRAM_BASE + (dest_pixel + index) as u64 * 4,
            0x1144_2233,
            u32::MAX,
        )
        .unwrap();
    }
    reg(&mut sx, 9, 0x9999_9999);
    reg(&mut sx, 42, 0x4242_4242);
    reg(&mut sx, 2, 0);
    start(&mut sx);
    let before = sx.vram_bytes().to_vec();
    // SX_LD(10, 31, 0): 32 words containing 128 consecutive B-plane pixels.
    sx.write(SX_MEM_BASE + 0xff42_0a68, 0x8fd6_c500, u32::MAX)
        .expect("bulk 32-word load from B32 alias");
    for index in 0..32 {
        let bytes = std::array::from_fn(|lane| before[(source_pixel + index * 4 + lane) * 4 + 1]);
        assert_eq!(read_reg(&sx, 10 + index), u32::from_be_bytes(bytes));
    }
    assert_eq!(read_reg(&sx, 9), 0x9999_9999);
    assert_eq!(read_reg(&sx, 42), 0x4242_4242);
    assert_vram(&sx, &before);
    // Sun sl_rect_copy_8 follows the load with a masked long store.
    sx.write(SX_MEM_BASE + 0xff41_cfe8, 0x8f8e_c500, u32::MAX)
        .unwrap();
    let mut expected = before.clone();
    for index in 0..128 {
        expected[(dest_pixel + index) * 4 + 1] = before[(source_pixel + index) * 4 + 1];
    }
    assert_vram(&sx, &expected);
}

// Command sequence derived from sl_rect_copy_8: byte alignment prefix,
// aligned long vectors, then a short byte tail. Expected pixels come from
// a snapshot of the original rectangle, independently of the command loop.
#[test]
fn synthetic_terminal_scroll_preserves_overlapping_source_and_other_channels() {
    let mut sx = Sx::new();
    for y in 103..415 {
        for x in 102..584 {
            let color = ((x * 17 + y * 31) & 0xff) as u32;
            sx.write(
                VSIMM_VRAM_BASE + (y * WIDTH + x) as u64 * 4,
                0xaa00_ccdd | (color << 16),
                u32::MAX,
            )
            .unwrap();
        }
    }
    start(&mut sx);
    let before = sx.vram_bytes().to_vec();
    let mut expected = before.clone();
    for y in 103..402 {
        for x in 103..583 {
            expected[(y * WIDTH + x) * 4 + 1] = before[((y + 13) * WIDTH + x) * 4 + 1];
        }
    }
    for y in 103..402 {
        let source = 0xff40_0000 + ((y + 13) * WIDTH + 103) as u64;
        let dest = 0xff40_0000 + (y * WIDTH + 103) as u64;
        // First pixel lies in lane 3, requiring a one-byte prefix.
        sx.write(SX_MEM_BASE + (source & !7), 0x8050_0507, u32::MAX)
            .expect("scroll byte prefix load");
        sx.write(SX_MEM_BASE + (dest & !7), 0x8008_0507, u32::MAX)
            .unwrap();
        // 3 * 32 words + 23 words = 476 pixels, followed by three bytes.
        for (offset, load, store) in [
            (1, 0x8fd6_c500, 0x8f8e_c500),
            (129, 0x8fd6_c500, 0x8f8e_c500),
            (257, 0x8fd6_c500, 0x8f8e_c500),
            (385, 0x8b56_c500, 0x8b0e_c500),
            (477, 0x8150_0504, 0x8108_0504),
        ] {
            sx.write(SX_MEM_BASE + ((source + offset) & !7), load, u32::MAX)
                .expect("scroll bulk/tail load");
            sx.write(SX_MEM_BASE + ((dest + offset) & !7), store, u32::MAX)
                .unwrap();
        }
    }
    assert_vram(&sx, &expected);
}

#[test]
fn rejected_load_does_not_partially_update_registers_or_vram() {
    let mut sx = Sx::new();
    for index in 10..42 {
        reg(&mut sx, index, 0xa000_0000 | index as u32);
    }
    start(&mut sx);
    let before = sx.clone();
    // First eight byte targets exist, the remaining 24 are outside the aperture.
    let address = SX_VRAM_ALIAS_BASE + VRAM_APERTURE_BYTES - 8;
    assert!(sx.write(address, 0x8fd0_0500, u32::MAX).is_err());
    assert!(
        sx == before,
        "failed load must preserve all prior device state"
    );
    assert_eq!(sx.vram_bytes().len(), VRAM_BYTES);
}
