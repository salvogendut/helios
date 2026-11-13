use helios::devices::sx::{Sx, HEIGHT, VSIMM_CTL_BASE as CTL, VSIMM_VRAM_BASE as FB, WIDTH};

#[test]
fn vsimm2_identity_and_eight_megabyte_straps_are_read_only_at_every_width() {
    let mut sx = Sx::new();
    // Sun cg14reg.h: MDI revision/implementation are two four-bit fields;
    // VCA HIRES is bit 13 and VBC version is bits 11:10.
    assert_eq!(sx.read(CTL + 6, 1).unwrap(), 0x11);
    assert_eq!(sx.read(CTL + 6, 2).unwrap() >> 8, 0x11);
    assert_eq!((sx.read(CTL + 4, 4).unwrap() >> 8) & 0xff, 0x11);
    assert_eq!(sx.read(CTL + 0x20c, 4).unwrap(), 0x2400);
    assert_eq!(sx.read(CTL + 0x20e, 2).unwrap(), 0x2400);
    assert_eq!(sx.read(CTL + 0x20e, 1).unwrap(), 0x24);
    for (address, mask) in [(CTL + 6, 0xff), (CTL + 6, 0xffff), (CTL + 4, u32::MAX)] {
        sx.write(address, 0xffff_ffff, mask).unwrap();
        assert_eq!(sx.read(CTL + 6, 1).unwrap(), 0x11);
    }
    for (address, mask) in [
        (CTL + 0x20c, u32::MAX),
        (CTL + 0x20e, 0xffff),
        (CTL + 0x20e, 0xff),
    ] {
        sx.write(address, 0xffff_ffff, mask).unwrap();
        assert_eq!(sx.read(CTL + 0x20c, 4).unwrap(), 0x2400);
    }
    sx.reset();
    assert_eq!(sx.read(CTL + 6, 1).unwrap(), 0x11);
    assert_eq!(sx.read(CTL + 0x20c, 4).unwrap(), 0x2400);
}

#[test]
fn vsimm2_blanking_and_power_preserve_pixels_until_video_is_restored() {
    let mut sx = Sx::new();
    sx.write(CTL, 0x71, 0xff).unwrap();
    sx.write(FB, 0x0011_2233, u32::MAX).unwrap();
    let mut pixels = vec![0; WIDTH * HEIGHT];
    sx.pixels(&mut pixels);
    assert_eq!(pixels[0], 0x0033_2211);
    for mcr in [0x70, 0x31, 0x30] {
        sx.write(CTL, mcr, 0xff).unwrap();
        sx.pixels(&mut pixels);
        assert!(pixels.iter().all(|&pixel| pixel == 0));
        assert_eq!(sx.read(FB, 4).unwrap(), 0x0011_2233);
    }
    sx.write(CTL, 0x71, 0xff).unwrap();
    sx.pixels(&mut pixels);
    assert_eq!(pixels[0], 0x0033_2211);
}

#[test]
fn powered_off_video_does_not_generate_new_retrace_interrupts() {
    let mut sx = Sx::new();
    sx.write(CTL, 0xc0, 0xff).unwrap(); // IRQ enabled, video power off.
    sx.advance_ns(100_000_000);
    assert!(!sx.irq());
    assert_eq!(sx.read(CTL + 4, 1).unwrap() & 0x30, 0);
    sx.write(CTL, 0xc1, 0xff).unwrap();
    sx.advance_ns(100_000_000);
    assert!(sx.irq());
    assert_eq!(sx.read(CTL + 4, 1).unwrap() & 0x30, 0x30);
    assert_eq!(sx.read(CTL + 4, 4).unwrap() >> 24 & 0x30, 0x30);
}
