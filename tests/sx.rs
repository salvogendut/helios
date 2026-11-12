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
fn sx_vram_alias_is_shared_with_the_vsimm_window() {
    let profile = MachineProfile::Ss20;
    let rom = 0u32.to_be_bytes();
    let mut machine =
        Machine::with_profile(profile, 16 * 1024 * 1024, &rom, profile.default_nvram()).unwrap();
    machine
        .bus
        .write32(SX_VRAM_ALIAS_BASE, 0x1234_5678)
        .unwrap();
    assert_eq!(machine.bus.read32(0xfc00_0000).unwrap(), 0x1234_5678);
    assert_eq!(machine.bus.sx().unwrap().generation(), 1);
    let _ = Sx::default();
}
