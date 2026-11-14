use helios::bus::Bus;
use helios::devices::sx::{
    Sx, HEIGHT, SX_BASE, SX_MEM_BASE, SX_VRAM_ALIAS_BASE, VRAM_BYTES, VSIMM_CTL_BASE,
    VSIMM_VRAM_BASE, WIDTH,
};
use helios::machine::{Machine, MachineProfile};

#[test]
fn ss20_installs_builtin_sx_and_maps_vsimm_control_and_vram() {
    let profile = MachineProfile::Ss20;
    let rom = 0u32.to_be_bytes();
    let mut machine =
        Machine::with_profile(profile, 16 * 1024 * 1024, &rom, profile.default_nvram()).unwrap();
    assert!(machine.bus.sx().is_some());
    assert_eq!(machine.bus.read32(SX_BASE + 0x28).unwrap(), 1);

    machine
        .bus
        .write32(VSIMM_CTL_BASE + 0x4004, 0x0012_3456)
        .unwrap();
    machine.bus.write32(VSIMM_VRAM_BASE, 0x0101_0101).unwrap();
    let mut pixels = vec![0u32; WIDTH * HEIGHT];
    assert!(machine.framebuffer_pixels(&mut pixels));
    assert_eq!(pixels[0], 0x0056_3412); // CLUT word 00BBGGRR
    assert_eq!(pixels[1], 0x0056_3412);
}

#[test]
fn sx_scanout_uses_32bit_bgr_pixels() {
    let profile = MachineProfile::Ss20;
    let rom = 0u32.to_be_bytes();
    let mut machine =
        Machine::with_profile(profile, 16 * 1024 * 1024, &rom, profile.default_nvram()).unwrap();
    machine.bus.write8(VSIMM_CTL_BASE, 0x71).unwrap();
    machine
        .bus
        .write32(VSIMM_VRAM_BASE + 0x0100_0000, 0x0011_2233)
        .unwrap();
    let mut pixels = vec![0u32; WIDTH * HEIGHT];
    machine.framebuffer_pixels(&mut pixels);
    assert_eq!(pixels[0], 0x0033_2211);
}

#[test]
fn sx_bgr_window_blocks_x_byte_writes() {
    let mut sx = Sx::new();
    sx.write(VSIMM_VRAM_BASE + 0x0100_0000, 0xaa11_2233, u32::MAX)
        .unwrap();
    assert_eq!(sx.vram_byte(0), 0);
    assert_eq!(sx.vram_byte(1), 0x11);
    assert_eq!(sx.vram_byte(2), 0x22);
    assert_eq!(sx.vram_byte(3), 0x33);
}

#[test]
fn sx_scanout_uses_16bit_x_c_pixels() {
    let mut sx = Sx::new();
    sx.write(VSIMM_CTL_BASE, 0x61, 0xff).unwrap();
    sx.write(VSIMM_VRAM_BASE, 0x0040_0080, u32::MAX).unwrap();
    let mut pixels = vec![0u32; WIDTH * HEIGHT];
    sx.pixels(&mut pixels);
    assert_eq!(pixels[0], 0x0000_0040);
    assert_eq!(pixels[1], 0x0000_0080);
}

fn wr(sx: &mut Sx, offset: u64, value: u32) {
    sx.write(SX_BASE + offset, value, u32::MAX).unwrap();
}

fn rd(sx: &mut Sx, offset: u64) -> u32 {
    sx.read(SX_BASE + offset, 4).unwrap()
}

#[test]
fn sx_user_and_supervisor_pages_share_plane_mask() {
    let mut sx = Sx::new();
    wr(&mut sx, 0x1010, 0x1234_5678);
    assert_eq!(rd(&mut sx, 0x10), 0x1234_5678);
}

#[test]
fn sx_queued_register_write_updates_execution_bank() {
    let mut sx = Sx::new();
    wr(&mut sx, 0, 0x1000);
    wr(&mut sx, 0x1324, 0xfafa_fafa);
    wr(&mut sx, 0x34, 0);
    wr(&mut sx, 0, 0);
    assert_eq!(rd(&mut sx, 0x124), 0xfafa_fafa);
}

#[test]
fn sx_recorded_or_command_writes_two_registers() {
    let mut sx = Sx::new();
    wr(&mut sx, 0x2c, 0);
    wr(&mut sx, 0x104, 0);
    wr(&mut sx, 0x124, 0xfafa_fafa);
    wr(&mut sx, 0, 0x1000);
    wr(&mut sx, 0x1020, 0xb1e0_0509);
    wr(&mut sx, 0x34, 0);
    wr(&mut sx, 0, 0);
    assert_eq!(rd(&mut sx, 0x128), 0xfafa_fafa);
    assert_eq!(rd(&mut sx, 0x12c), 0xfafa_fafa);
}

#[test]
fn sx_recorded_store_command_fills_framebuffer() {
    let mut sx = Sx::new();
    wr(&mut sx, 0x128, 0xfafa_fafa);
    wr(&mut sx, 0x12c, 0xfafa_fafa);
    wr(&mut sx, 0x10, u32::MAX);
    wr(&mut sx, 0, 0x1000);
    sx.write(SX_VRAM_ALIAS_BASE, 0x8fce_c500, u32::MAX).unwrap();
    wr(&mut sx, 0x34, 0);
    for word in 0..32 {
        assert_eq!(sx.read(VSIMM_VRAM_BASE + word * 4, 4).unwrap(), 0xfafa_fafa);
    }
    assert_eq!(sx.read(VSIMM_VRAM_BASE + 128, 4).unwrap(), 0);
}

#[test]
fn sx_bgr_byte_write_preserves_other_lanes() {
    let mut sx = Sx::new();
    sx.write(VSIMM_VRAM_BASE, 0xaa11_2233, u32::MAX).unwrap();
    sx.write(VSIMM_VRAM_BASE + 0x0100_0001, 0x44, 0xff).unwrap();
    assert_eq!(sx.read(VSIMM_VRAM_BASE, 4).unwrap(), 0xaa44_2233);
}

#[test]
fn sx_recorded_slli_updates_r2_without_touching_neighbors() {
    let mut sx = Sx::new();
    for (reg, value) in [
        (1, 0x1111_1111),
        (2, 0x2222_2222),
        (3, 0x3333_3333),
        (4, 0x0000_0041),
        (5, 0x5555_5555),
    ] {
        wr(&mut sx, 0x100 + reg * 4, value);
    }
    wr(&mut sx, 0, 0x1000);
    wr(&mut sx, 0x1020, 0xc0a1_0106);
    wr(&mut sx, 0, 0);
    assert_eq!(rd(&mut sx, 0x108), 0x0000_1040);
    assert_eq!(rd(&mut sx, 0x104), 0x1111_1111);
    assert_eq!(rd(&mut sx, 0x10c), 0x3333_3333);
    assert_eq!(rd(&mut sx, 0x110), 0x0000_0041);
    assert_eq!(rd(&mut sx, 0x114), 0x5555_5555);
}

#[test]
fn sx_array_byte_store_uses_offset_vector() {
    let mut sx = Sx::new();
    for (reg, value) in [
        (2, u32::MAX),
        (10, 0u32),
        (11, 5),
        (12, 10),
        (32, 0x11),
        (33, 0x22),
        (34, 0x33),
    ] {
        wr(&mut sx, 0x100 + reg * 4, value);
    }
    wr(&mut sx, 0, 0x1000);
    sx.write(SX_VRAM_ALIAS_BASE, 0x8148_100a, u32::MAX).unwrap();
    assert_eq!(sx.vram_byte(0), 0x22);
    assert_eq!(sx.vram_byte(5), 0x22);
    assert_eq!(sx.vram_byte(10), 0x22);
}

#[test]
fn sx_glyph_selector_reads_r2_from_bit31() {
    let mut sx = Sx::new();
    for (reg, value) in [(8, 0x22), (9, 0xaa)] {
        wr(&mut sx, 0x100 + reg * 4, value);
    }
    let rows = [
        (0x0000_0000, "............"),
        (0x0000_0000, "............"),
        (0x0000_0000, "............"),
        (0x5000_0000, ".#.#........"),
        (0x5000_0000, ".#.#........"),
        (0xf800_0000, "#####......."),
        (0x5000_0000, ".#.#........"),
        (0xf800_0000, "#####......."),
        (0x5000_0000, ".#.#........"),
        (0x5000_0000, ".#.#........"),
        (0x0000_0000, "............"),
        (0x0000_0000, "............"),
        (0x0000_0000, "............"),
    ];
    for (row, (mask, pattern)) in rows.into_iter().enumerate() {
        wr(&mut sx, 0x108, mask);
        let base = 0xff41_cfe0 + row as u64 * 1152;
        sx.write(SX_MEM_BASE + base, 0x85c8_0407, u32::MAX).unwrap();
        for (column, p) in pattern.bytes().enumerate() {
            let pixel = 0x1cfe7 + row * 1152 + column;
            assert_eq!(
                sx.vram_byte(pixel * 4 + 1),
                if p == b'#' { 0xaa } else { 0x22 },
                "row {row}, column {column}"
            );
        }
    }
}

#[test]
fn sx_long_selector_reads_r2_from_bit31() {
    let mut sx = Sx::new();
    for (reg, value) in [(8, 0x1122_3344), (9, 0xaabb_ccdd), (2, 0xa000_0000)] {
        wr(&mut sx, 0x100 + reg * 4, value);
    }
    sx.write(SX_VRAM_ALIAS_BASE, 0x83c6_c400, u32::MAX).unwrap();
    let expected = [
        0xaabb_ccdd,
        0x1122_3344,
        0xaabb_ccdd,
        0x1122_3344,
        0x1122_3344,
        0x1122_3344,
        0x1122_3344,
        0x1122_3344,
    ];
    for (index, want) in expected.into_iter().enumerate() {
        assert_eq!(sx.vram_word(index * 4), want, "word {index}");
    }
}

#[test]
fn sx_clut_stream_selectors_cover_all_256_entries() {
    for table in [0, 1] {
        for index in [63, 64, 255] {
            let mut sx = Sx::new();
            let ctl = 0x4000 + table * 0x1000;
            sx.write(VSIMM_CTL_BASE + ctl + 0x800 + index * 4, 0, u32::MAX)
                .unwrap();
            sx.write(VSIMM_CTL_BASE + 0xf000, 0x8012_3456, u32::MAX)
                .unwrap();
            assert_eq!(
                sx.control_word((ctl + index * 4) as usize),
                0x8012_3456,
                "table {table}, index {index}"
            );
        }
    }
}

#[test]
fn sx_mcr_disable_deasserts_pending_video_irq() {
    let profile = MachineProfile::Ss20;
    let mut machine =
        Machine::with_profile(profile, 16 * 1024 * 1024, &[0; 4], profile.default_nvram()).unwrap();
    machine
        .bus
        .write32(0xf_f141_0008, (1 << 31) | (1 << 20))
        .unwrap();
    machine.bus.write8(VSIMM_CTL_BASE, 0xf1).unwrap();
    machine.bus.advance_seconds(1);
    assert_eq!(machine.bus.interrupt_levels() & (1 << 8), 1 << 8);
    machine.bus.write8(VSIMM_CTL_BASE, 0x71).unwrap();
    assert_eq!(machine.bus.interrupt_levels() & (1 << 8), 0);
}

fn packed_reg(sx: &mut Sx, index: u64, value: u32) {
    wr(sx, 0x100 + index * 4, value);
}

fn packed_snapshot(sx: &Sx) -> Vec<u8> {
    (0..VRAM_BYTES).map(|offset| sx.vram_byte(offset)).collect()
}

#[test]
fn sx_packed_conditional_glyph_preserves_masked_pixels() {
    let mut sx = Sx::new();
    for index in 10..19 {
        packed_reg(&mut sx, index, 0x0101_0101);
    }
    packed_reg(&mut sx, 2, 0x7c00_0000);
    wr(&mut sx, 0x10, u32::MAX);
    for pixel in 0x1ebe0..0x1ebe8 {
        sx.write(VSIMM_VRAM_BASE + pixel * 4, 0xaa5a_ccdd, u32::MAX)
            .unwrap();
    }
    let mut expected = packed_snapshot(&sx);
    for pixel in 0x1ebe2..=0x1ebe6 {
        expected[pixel * 4 + 1] = 1;
    }
    wr(&mut sx, 0, 0x1000);
    sx.write(SX_MEM_BASE + 0xff41_ebe0, 0x82af_c501, u32::MAX)
        .unwrap();
    for (offset, value) in expected.into_iter().enumerate() {
        assert_eq!(sx.vram_byte(offset), value, "VRAM byte {offset:#x}");
    }
}

#[test]
fn sx_packed_store_consumes_six_bytes_and_merges_plane_mask() {
    for (enable, output) in [
        (u32::MAX, [0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xaa, 0xaa]),
        (
            0xa800_0000,
            [0xa1, 0xaa, 0xa3, 0xaa, 0xa5, 0xaa, 0xaa, 0xaa],
        ),
    ] {
        let mut sx = Sx::new();
        packed_reg(&mut sx, 10, 0x1122_3344);
        packed_reg(&mut sx, 11, 0x5566_7788);
        packed_reg(&mut sx, 2, enable);
        wr(&mut sx, 0x10, 0x0f0f_0f0f);
        for offset in (0..32).step_by(4) {
            sx.write(VSIMM_VRAM_BASE + offset, 0xaaaa_aaaa, u32::MAX)
                .unwrap();
        }
        let mut expected = packed_snapshot(&sx);
        expected[..output.len()].copy_from_slice(&output);
        wr(&mut sx, 0, 0x1000);
        sx.write(SX_VRAM_ALIAS_BASE, 0x82af_c500, u32::MAX).unwrap();
        for (offset, value) in expected.into_iter().enumerate() {
            assert_eq!(sx.vram_byte(offset), value, "VRAM byte {offset:#x}");
        }
    }
}

#[test]
fn sx_packed_store_with_zero_enables_preserves_vram() {
    let mut sx = Sx::new();
    packed_reg(&mut sx, 10, 0x1122_3344);
    packed_reg(&mut sx, 11, 0x5566_7788);
    packed_reg(&mut sx, 2, 0);
    wr(&mut sx, 0x10, u32::MAX);
    sx.write(VSIMM_VRAM_BASE, 0xaabb_ccdd, u32::MAX).unwrap();
    sx.write(VSIMM_VRAM_BASE + 4, 0xeeff_1234, u32::MAX)
        .unwrap();
    let expected = packed_snapshot(&sx);
    wr(&mut sx, 0, 0x1000);
    sx.write(SX_VRAM_ALIAS_BASE, 0x82af_c500, u32::MAX).unwrap();
    for (offset, value) in expected.into_iter().enumerate() {
        assert_eq!(sx.vram_byte(offset), value, "VRAM byte {offset:#x}");
    }
}

#[test]
fn sx_captured_packed_selected_fill_writes_b_plane_pixels() {
    let mut sx = Sx::new();
    for (reg, value) in [(10, 0x0505_0505), (11, 0x0505_0505), (2, 0xa800_0000)] {
        wr(&mut sx, 0x100 + reg * 4, value);
    }
    for pixel in 0x7a5e3..0x7a5ea {
        sx.write(VSIMM_VRAM_BASE + pixel * 4, 0xaa11_ccdd, u32::MAX)
            .unwrap();
    }
    let mut expected = packed_snapshot(&sx);
    for pixel in 0x7a5e4..0x7a5e9 {
        expected[pixel * 4 + 1] = 5;
    }
    wr(&mut sx, 0, 0x1000);
    sx.write(SX_MEM_BASE + 0xff47_a5e0, 0x824f_c504, u32::MAX)
        .unwrap();
    for (offset, value) in expected.into_iter().enumerate() {
        assert_eq!(sx.vram_byte(offset), value, "VRAM byte {offset:#x}");
    }
}

#[test]
fn single_selected_byte_store_is_direct_not_an_offset_array() {
    let mut sx = Sx::new();
    for (reg, value) in [(2, 0), (32, 0x5a), (33, 0xa5)] {
        wr(&mut sx, 0x100 + reg * 4, value);
    }
    wr(&mut sx, 0, 0x1000);
    sx.write(SX_VRAM_ALIAS_BASE, 0x8048_1002, u32::MAX)
        .expect("captured one-byte selected store");
    assert_eq!(sx.vram_byte(2), 0x5a);
    assert_eq!(sx.vram_byte(0), 0);
}

#[test]
fn sx_scanout_generation_ignores_nonvisual_writes_and_retrace_ack() {
    let mut sx = Sx::new();
    let key = sx.generation();
    sx.write(VSIMM_CTL_BASE + 4, 0, u32::MAX).unwrap();
    sx.write(SX_BASE + 0x1010, 0, u32::MAX).unwrap();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    sx.advance_ns(14_000_000);
    assert_eq!(sx.generation(), key);
    sx.write(VSIMM_CTL_BASE + 0x1100, 4, u32::MAX).unwrap();
    assert_ne!(sx.generation(), key);
    let cursor = sx.generation();
    sx.write(VSIMM_VRAM_BASE, 0x1234_5678, u32::MAX).unwrap();
    assert_ne!(sx.generation(), cursor);
}

#[test]
fn sx_scanout_generation_tracks_visible_tables_modes_and_reset() {
    let mut sx = Sx::new();
    let base = sx.generation();
    sx.write(VSIMM_CTL_BASE, 0x51, u32::MAX).unwrap();
    assert_ne!(sx.generation(), base);
    let mode = sx.generation();
    sx.write(VSIMM_CTL_BASE + 0x4000, 0x0012_3456, u32::MAX)
        .unwrap();
    assert_ne!(sx.generation(), mode);
    let clut = sx.generation();
    sx.write(VSIMM_CTL_BASE + 0x3000, 0x40, u32::MAX).unwrap();
    assert_ne!(sx.generation(), clut);
    let xlut = sx.generation();
    sx.write(VSIMM_CTL_BASE + 0x3800, 0, u32::MAX).unwrap();
    assert_eq!(sx.generation(), xlut);
    sx.write(VSIMM_CTL_BASE + 0xf000, 0x12, u32::MAX).unwrap();
    assert_ne!(sx.generation(), xlut);
    let stream = sx.generation();
    sx.write(VSIMM_CTL_BASE + 0x1000, 0x8000_0000, u32::MAX)
        .unwrap();
    assert_ne!(sx.generation(), stream);
    let cursor = sx.generation();
    sx.reset();
    assert_ne!(sx.generation(), cursor);
}
