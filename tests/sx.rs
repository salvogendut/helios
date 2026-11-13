use helios::bus::Bus;
use helios::devices::sx::{
    Sx, HEIGHT, SX_BASE, SX_VRAM_ALIAS_BASE, VSIMM_CTL_BASE, VSIMM_VRAM_BASE, WIDTH,
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
    machine.bus.write8(VSIMM_CTL_BASE, 0x30).unwrap();
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
    sx.write(VSIMM_CTL_BASE, 0x20, 0xff).unwrap();
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
    assert_eq!(sx.vram_byte(0), 0x11);
    assert_eq!(sx.vram_byte(5), 0x22);
    assert_eq!(sx.vram_byte(10), 0x33);
}
