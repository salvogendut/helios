use helios::bus::{Bus, BusErrorKind, MapError};
use helios::devices::cgsix::{Cgsix, HEIGHT, VRAM_BYTES, WIDTH};
use helios::devices::nvram::Nvram;
use helios::system_bus::{SystemBus, CGSIX_BASE as BASE, SYSTEM_INTERRUPT_BASE};

const VRAM: u64 = BASE + 0x800000;
const FBC: u64 = BASE + 0x700000;
const DAC: u64 = BASE + 0x200000;
const THC: u64 = BASE + 0x301000;

fn bus() -> SystemBus {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    let mut rom = vec![0xff; 32768];
    rom[..8].copy_from_slice(&[0xfd, 3, 0x12, 0x34, 0, 0, 0x40, 0]);
    bus.install_cgsix(Cgsix::new(&rom).unwrap()).unwrap();
    bus
}
fn reg(bus: &mut SystemBus, offset: u64, value: u32) {
    bus.write32(FBC + offset, value).unwrap();
}
fn drawing(bus: &mut SystemBus) {
    reg(bus, 4, 0x22b280); // 8-bit, render, source blit, buffer 0
    reg(bus, 0xf0, WIDTH as u32 - 1);
    reg(bus, 0xf4, HEIGHT as u32 - 1);
    reg(bus, 0x100, 0xff);
    reg(bus, 0x104, 0);
    reg(bus, 0x108, 0xea80cccc); // source copy, nonoverlap, masks enabled
    reg(bus, 0x10c, 255);
    reg(bus, 0x110, u32::MAX);
}
fn rgb(bus: &mut SystemBus, index: u8, color: [u8; 3], overlay: bool) {
    bus.write32(DAC, u32::from(index) << 24).unwrap();
    for c in color {
        bus.write32(DAC + if overlay { 12 } else { 4 }, u32::from(c) << 24)
            .unwrap();
    }
}
fn control(bus: &mut SystemBus, index: u8, value: u8) {
    bus.write32(DAC, u32::from(index) << 24).unwrap();
    bus.write32(DAC + 8, u32::from(value) << 24).unwrap();
}
fn rectangle(bus: &mut SystemBus, x0: u32, y0: u32, x1: u32, y1: u32) {
    reg(bus, 0x904, y0);
    reg(bus, 0x900, x0);
    reg(bus, 0x904, y1);
    reg(bus, 0x900, x1);
    assert_eq!(bus.read32(FBC + 0x14).unwrap(), 0);
}

#[test]
fn rom_and_vram_lanes_doublewords_and_mapping_are_atomic() {
    let mut b = bus();
    assert_eq!(b.read32(BASE).unwrap(), 0xfd031234);
    assert_eq!(b.read8(BASE + 1).unwrap(), 3);
    assert_eq!(b.read16(BASE + 2).unwrap(), 0x1234);
    assert_eq!(b.read_doubleword(BASE).unwrap(), [0xfd031234, 0x4000]);
    assert_eq!(b.write8(BASE, 0).unwrap_err().kind, BusErrorKind::ReadOnly);
    assert_eq!(b.read16(BASE + 0x300002).unwrap(), 0x0800);
    b.write32(VRAM, 0x12345678).unwrap();
    b.write8(VRAM + 1, 0x9a).unwrap();
    b.write16(VRAM + 2, 0xbcde).unwrap();
    assert_eq!(b.read32(VRAM).unwrap(), 0x129abcde);
    b.write32_masked(VRAM, 0xffffffff, 0x00000f00).unwrap();
    assert_eq!(b.read32(VRAM).unwrap(), 0x129abfde);
    b.write_doubleword(VRAM + VRAM_BYTES as u64 - 8, [0x12345678, 0x90abcdef])
        .unwrap();
    assert_eq!(
        b.read_doubleword(VRAM + VRAM_BYTES as u64 - 8).unwrap(),
        [0x12345678, 0x90abcdef]
    );
    assert_eq!(
        b.write_doubleword(VRAM + 4, [0, 0]).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    assert!(b.read32(VRAM + VRAM_BYTES as u64).is_err());
    assert_eq!(b.map_ram(BASE + 0x100000, 4), Err(MapError::Overlap));
    assert_eq!(b.map_rom(VRAM, &[0; 4]), Err(MapError::Overlap));
    let mut occupied = SystemBus::default();
    occupied.map_ram(BASE, 4).unwrap();
    assert_eq!(
        occupied.install_cgsix(Cgsix::new(&[0; 32768]).unwrap()),
        Err(MapError::Overlap)
    );
}

#[test]
fn dac_rgb_sequencing_masks_video_and_cursor_overlay() {
    let mut b = bus();
    rgb(&mut b, 1, [0x12, 0x34, 0x56], false);
    rgb(&mut b, 2, [0xab, 0xcd, 0xef], false);
    b.write32(VRAM, 0x01020301).unwrap();
    b.write32(DAC, 1 << 24).unwrap();
    assert_eq!(b.read32(DAC + 4).unwrap(), 0x12000000);
    // Reading the address resets the RGB component pointer.
    assert_eq!(b.read32(DAC).unwrap(), 1 << 24);
    assert_eq!(b.read32(DAC + 4).unwrap(), 0x12000000);
    assert_eq!(b.read32(DAC + 4).unwrap(), 0x34000000);
    assert_eq!(b.read32(DAC + 4).unwrap(), 0x56000000);
    assert_eq!(b.read32(DAC).unwrap(), 2 << 24);
    let mut pixels = vec![0; WIDTH * HEIGHT];
    b.cgsix().unwrap().pixels(&mut pixels);
    assert_eq!(&pixels[..4], &[0; 4]);
    b.write32(THC + 0x818, 0x400).unwrap();
    control(&mut b, 4, 1);
    b.cgsix().unwrap().pixels(&mut pixels);
    assert_eq!(&pixels[..4], &[0x123456, 0, 0x123456, 0x123456]);
    rgb(&mut b, 1, [255, 0, 0], true);
    rgb(&mut b, 2, [0, 255, 0], true);
    rgb(&mut b, 3, [0, 0, 255], true);
    control(&mut b, 6, 0x43);
    b.write32(THC + 0x8fc, 0).unwrap();
    b.write32(THC + 0x900, 0xa0000000).unwrap();
    b.write32(THC + 0x980, 0x60000000).unwrap();
    b.cgsix().unwrap().pixels(&mut pixels);
    assert_eq!(&pixels[..4], &[0xff0000, 0x00ff00, 0x0000ff, 0x123456]);
    control(&mut b, 6, 0x41); // disable overlay plane 1
    b.cgsix().unwrap().pixels(&mut pixels);
    assert_eq!(&pixels[..4], &[0xff0000, 0, 0xff0000, 0x123456]);
}

#[test]
fn all_sixteen_boolean_rops_and_plane_masks() {
    let mut b = bus();
    drawing(&mut b);
    // Independent truth-table results for source 0x5a and destination 0xc3.
    let expected = [
        0x00, 0x24, 0x81, 0xa5, 0x18, 0x3c, 0x99, 0xbd, 0x42, 0x66, 0xc3, 0xe7, 0x5a, 0x7e, 0xdb,
        0xff,
    ];
    for (rop, result) in expected.into_iter().enumerate() {
        b.write8(VRAM, 0xc3).unwrap();
        reg(&mut b, 0x80, 0);
        reg(&mut b, 0x90, 0);
        reg(&mut b, 0x84, 0);
        reg(&mut b, 0x108, 0xea800000 | (rop as u32 * 0x1111));
        reg(&mut b, 0x1c, 0x5a000000);
        assert_eq!(b.read8(VRAM).unwrap(), result);
    }
    reg(&mut b, 0x108, 0xfa80cccc); // register plane mask
    reg(&mut b, 0x10c, 0x0f);
    b.write8(VRAM, 0xc3).unwrap();
    reg(&mut b, 0x1c, 0x5a000000);
    assert_eq!(b.read8(VRAM).unwrap(), 0xca);
}

#[test]
fn monochrome_font_selects_foreground_background_and_autoincrements() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 4, 0x40000); // only change data mode; other fields retained
    reg(&mut b, 0x100, 0x96);
    reg(&mut b, 0x104, 0x39);
    reg(&mut b, 0x108, 0xea80fc30);
    reg(&mut b, 0x80, 2);
    reg(&mut b, 0x90, 5);
    reg(&mut b, 0x84, 3);
    reg(&mut b, 0xd4, 1);
    reg(&mut b, 0x1c, 0xa0000000);
    let row = VRAM + (3 * WIDTH) as u64;
    assert_eq!(
        (0..8)
            .map(|x| b.read8(row + x).unwrap())
            .collect::<Vec<_>>(),
        [0, 0, 0x96, 0x39, 0x96, 0x39, 0, 0]
    );
    assert_eq!(b.read32(FBC + 0x84).unwrap(), 4);
    assert_eq!(b.read32(FBC + 0x94).unwrap(), 1);
    assert_eq!(b.read32(FBC + 0x80).unwrap(), 2);
    reg(&mut b, 0xe0, 3);
    reg(&mut b, 0xf0, 4); // inclusive clipping
    reg(&mut b, 0x1c, u32::MAX);
    let row = VRAM + (4 * WIDTH) as u64;
    assert_eq!(
        (2..6)
            .map(|x| b.read8(row + x).unwrap())
            .collect::<Vec<_>>(),
        [0, 0x96, 0x96, 0]
    );
}

#[test]
fn rectangles_exclude_upper_edges_and_clip_signed_coordinates() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xea80ff00);
    reg(&mut b, 0x100, 0x76);
    rectangle(&mut b, 1, 2, 4, 4);
    for y in 0..6 {
        for x in 0..6 {
            assert_eq!(
                b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(),
                if (1..4).contains(&x) && (2..4).contains(&y) {
                    0x76
                } else {
                    0
                }
            );
        }
    }
    rectangle(&mut b, (-2i32) as u32, (-2i32) as u32, 2, 1);
    assert_eq!(b.read32(VRAM).unwrap(), 0x76760000);
    // Reading DRAW is a command each time, with the retained vertex bank.
    b.write8(VRAM, 0).unwrap();
    assert_eq!(b.read32(FBC + 0x14).unwrap(), 0);
    assert_eq!(b.read8(VRAM).unwrap(), 0x76);
}

#[test]
fn blits_include_last_row_and_column_and_preserve_overlapping_sources() {
    let mut b = bus();
    drawing(&mut b);
    for (src, dst) in [(0, 1), (1, 0)] {
        for y in 0..4 {
            for x in 0..4 {
                b.write8(VRAM + (y * WIDTH + x) as u64, (y * 4 + x + 1) as u8)
                    .unwrap();
            }
        }
        reg(&mut b, 0x80, src);
        reg(&mut b, 0x84, src);
        reg(&mut b, 0x90, src + 2);
        reg(&mut b, 0x94, src + 2);
        reg(&mut b, 0xa0, dst);
        reg(&mut b, 0xa4, dst);
        reg(&mut b, 0xb0, dst + 2);
        reg(&mut b, 0xb4, dst + 2);
        b.read32(FBC + 0x18).unwrap();
        for y in 0..3 {
            for x in 0..3 {
                assert_eq!(
                    b.read8(VRAM + ((y + dst as usize) * WIDTH + x + dst as usize) as u64)
                        .unwrap(),
                    ((y + src as usize) * 4 + x + src as usize + 1) as u8
                );
            }
        }
    }
}

#[test]
fn unsupported_accesses_do_not_draw_or_consume_dac_components() {
    let mut b = bus();
    drawing(&mut b);
    rgb(&mut b, 1, [0x12, 0x34, 0x56], false);
    b.write32(DAC, 1 << 24).unwrap();
    assert!(matches!(
        b.read16(DAC + 4).unwrap_err().kind,
        BusErrorKind::Unsupported(_)
    ));
    assert_eq!(b.read32(DAC + 4).unwrap(), 0x12000000);
    assert!(b.write_doubleword(DAC, [0, 0]).is_err());
    assert_eq!(b.read32(DAC + 4).unwrap(), 0x34000000);
    reg(&mut b, 0x80, 0);
    reg(&mut b, 0x90, 0);
    reg(&mut b, 4, 0x60000); // HRMONO
    let generation = b.cgsix().unwrap().generation();
    assert!(matches!(
        b.write32(FBC + 0x1c, u32::MAX).unwrap_err().kind,
        BusErrorKind::Unsupported(_)
    ));
    assert_eq!(b.cgsix().unwrap().generation(), generation);
    assert_eq!(b.read32(VRAM).unwrap(), 0);
    assert!(b.write32(FBC + 0x2000, 1).is_err()); // unsupported register
    assert!(b.swap32(VRAM, 1).is_err());
    assert_eq!(b.read32(VRAM).unwrap(), 0);
}

#[test]
fn line_engine_draws_between_alternating_absolute_endpoints() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe980ff00);
    reg(&mut b, 0x100, 0x5a);
    reg(&mut b, 0x844, 2); // y1
    reg(&mut b, 0x840, 1); // x1
    reg(&mut b, 0x844, 4); // y2
    reg(&mut b, 0x840, 4); // x2
    assert_eq!(b.read32(FBC + 0x14).unwrap(), 0);
    for y in 0..6 {
        for x in 0..6 {
            let expected = (x == 1 && y == 2)
                || (x == 2 && y == 3)
                || (x == 3 && y == 3)
                || (x == 4 && y == 4);
            assert_eq!(
                b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(),
                if expected { 0x5a } else { 0 },
                "x={x} y={y}"
            );
        }
    }
}

#[test]
fn vblank_irq_is_latched_acknowledged_and_rearmed() {
    let mut b = bus();
    b.write32(SYSTEM_INTERRUPT_BASE + 8, (1 << 31) | (1 << 11))
        .unwrap(); // global + SBus level 5 unmask
    b.write32(THC + 0x818, 0x420).unwrap();
    b.advance_ns(12_550_000); // before line 900
    assert_eq!(b.interrupt_levels() & (1 << 9), 0);
    b.advance_ns(1_000);
    assert_ne!(b.read32(THC + 0x818).unwrap() & 0x110, 0);
    assert_eq!(b.interrupt_levels() & (1 << 9), 1 << 9);
    b.write32(THC + 0x818, 0x430).unwrap();
    assert_eq!(b.interrupt_levels() & (1 << 9), 0);
    b.advance_ns(100);
    assert_eq!(b.interrupt_levels() & (1 << 9), 0);
    b.advance_ns(14_000_000);
    assert_eq!(b.interrupt_levels() & (1 << 9), 1 << 9);
    b.reset_devices();
    assert_eq!(b.interrupt_levels() & (1 << 9), 0);
}

#[test]
fn fcode_can_reset_and_reinitialize_the_engine_while_video_remains_on() {
    let mut b = bus();
    drawing(&mut b);
    b.write32(THC + 0x818, 0x400).unwrap();
    b.write32(VRAM, 0x12345678).unwrap();
    b.write32(BASE + 0x300000, 0x8000).unwrap();
    b.write32(BASE + 0x300000, 0x01ff).unwrap();
    assert_eq!(b.read32(FBC + 4).unwrap(), 0);
    assert_eq!(b.read32(VRAM).unwrap(), 0x12345678);
    assert!(b.cgsix().unwrap().enabled());
    b.write32(BASE + 0x300000, 0x09ff).unwrap();
    drawing(&mut b);
    reg(&mut b, 0x80, 0);
    reg(&mut b, 0x90, 0);
    reg(&mut b, 0x1c, 0xab000000);
    assert_eq!(b.read32(VRAM).unwrap(), 0xab345678);
}

#[test]
fn tec_control_values_are_retained_without_transform_side_effects() {
    let mut b = bus();
    let tec = BASE + 0x701000;
    for (offset, value) in [(0, 0x1234_5678), (4, 0x9abc_def0), (8, 0x0fed_cba9)] {
        b.write32(tec + offset, value).unwrap();
        assert_eq!(b.read32(tec + offset).unwrap(), value);
    }
    assert_eq!(b.read32(VRAM).unwrap(), 0);
}

#[test]
fn stipple_mode_uses_pattern_bits_and_ones_mode_ignores_them() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x100, 0x96);
    reg(&mut b, 0x104, 0x39);
    reg(&mut b, 0x108, 0xec80fc30); // opaque stipple with pattern register
    reg(&mut b, 0x11c, 0);
    reg(&mut b, 0x120, 0x80008000); // (0,0) and (0,1) set
    rectangle(&mut b, 0, 0, 2, 2);
    assert_eq!(b.read8(VRAM).unwrap(), 0x96);
    assert_eq!(b.read8(VRAM + 1).unwrap(), 0x39);
    assert_eq!(b.read8(VRAM + WIDTH as u64).unwrap(), 0x96);
    assert_eq!(b.read8(VRAM + WIDTH as u64 + 1).unwrap(), 0x39);
    reg(&mut b, 0x108, 0xea80ff00); // forced ones
    rectangle(&mut b, 0, 0, 2, 2);
    assert_eq!(b.read8(VRAM + 1).unwrap(), 0x96);
}

#[test]
fn connected_absolute_lines_start_at_apoint() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe980ff00);
    reg(&mut b, 0x100, 0x5a);
    reg(&mut b, 0x804, 2);
    reg(&mut b, 0x800, 1);
    reg(&mut b, 0x844, 4);
    reg(&mut b, 0x840, 3);
    b.read32(FBC + 0x14).unwrap();
    reg(&mut b, 0x844, 6);
    reg(&mut b, 0x840, 5);
    b.read32(FBC + 0x14).unwrap();
    for (x, y) in [(1, 2), (2, 3), (3, 4), (4, 5), (5, 6)] {
        assert_eq!(b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(), 0x5a);
    }
}

#[test]
fn relative_lines_advance_from_current_point() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe980ff00);
    reg(&mut b, 0x100, 0x33);
    reg(&mut b, 0x804, 1);
    reg(&mut b, 0x800, 1);
    reg(&mut b, 0x854, 2);
    reg(&mut b, 0x850, 2);
    b.read32(FBC + 0x14).unwrap();
    reg(&mut b, 0x854, (-2i32) as u32);
    reg(&mut b, 0x850, 2);
    b.read32(FBC + 0x14).unwrap();
    for (x, y) in [(1, 1), (2, 2), (3, 3), (4, 2), (5, 1)] {
        assert_eq!(b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(), 0x33);
    }
}

#[test]
fn stippled_lines_use_pattern_bits() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x100, 0x96);
    reg(&mut b, 0x104, 0x39);
    reg(&mut b, 0x108, 0xed80fc30);
    reg(&mut b, 0x11c, 0);
    reg(&mut b, 0x120, 0x80008000);
    reg(&mut b, 0x844, 0);
    reg(&mut b, 0x840, 0);
    reg(&mut b, 0x844, 0);
    reg(&mut b, 0x840, 1);
    b.read32(FBC + 0x14).unwrap();
    assert_eq!(b.read8(VRAM).unwrap(), 0x96);
    assert_eq!(b.read8(VRAM + 1).unwrap(), 0x39);
}

#[test]
fn color_expansion_font_words_advance_horizontally() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 4, 0x140000); // GX_BLIT_NOSRC | GX_MODE_COLOR1
    reg(&mut b, 0x100, 0x96);
    reg(&mut b, 0x104, 0x39);
    reg(&mut b, 0x108, 0xea80fc30);
    reg(&mut b, 0xd0, 32);
    reg(&mut b, 0xd4, 0);
    reg(&mut b, 0x80, 0);
    reg(&mut b, 0x90, 31);
    reg(&mut b, 0x84, 0);
    reg(&mut b, 0x1c, 0xffffffff);
    reg(&mut b, 0x1c, 0);
    let row = VRAM;
    for x in 0..64 {
        let expected = if x < 32 { 0x96 } else { 0x39 };
        assert_eq!(b.read8(row + x as u64).unwrap(), expected, "x={x}");
    }
    assert_eq!(b.read32(FBC + 0x80).unwrap(), 64);
}

// Sun FBC Reference Manual 3.0, table 3-1: destination-register masks,
// explicitly transcribed for each primitive and each initial address index.
#[test]
fn indexed_coordinates_update_the_shared_bank_and_report_the_index() {
    let masks = [
        [0b1111, 0b1111, 0b1111, 0b1111], // point
        [0b0111, 0b1110, 0b1101, 0b1011], // line
        [0b0011, 0b0110, 0b1100, 0b1001], // triangle
        [0b0001, 0b0010, 0b0100, 0b1000], // quadrilateral
        [0b0011, 0b0110, 0b1100, 0b1001], // rectangle
    ];
    for (kind, destinations) in masks.into_iter().enumerate() {
        for (index, mask) in destinations.into_iter().enumerate() {
            for axis in [0, 4] {
                for relative in [false, true] {
                    let mut b = bus();
                    drawing(&mut b);
                    for vertex in 0..4 {
                        reg(&mut b, 0x80 + vertex * 16, 100 + vertex as u32);
                        reg(&mut b, 0x84 + vertex * 16, 200 + vertex as u32);
                    }
                    reg(&mut b, 0xc0, 11);
                    reg(&mut b, 0xc4, (-7i32) as u32);
                    reg(&mut b, 4, 0x40 | (index as u32 * 16));
                    let mode = b.read32(FBC + 4).unwrap() & !0x30;
                    let register = 0x800 + kind as u64 * 64 + axis + if relative { 16 } else { 0 };
                    reg(&mut b, register, 9);
                    let value = if relative {
                        if axis == 0 {
                            [112, 109, 110, 111][index]
                        } else {
                            [212, 209, 210, 211][index]
                        }
                    } else if axis == 0 {
                        20
                    } else {
                        2
                    };
                    for vertex in 0..4 {
                        for checked_axis in [0, 4] {
                            let original =
                                if checked_axis == 0 { 100 } else { 200 } + vertex as u32;
                            let expected = if checked_axis == axis && mask & (1 << vertex) != 0 {
                                value
                            } else {
                                original
                            };
                            assert_eq!(b.read32(FBC + 0x80 + vertex * 16 + checked_axis).unwrap(), expected,
                                "kind={kind} index={index} axis={axis} relative={relative} vertex={vertex}");
                        }
                    }
                    let next = (index + usize::from(axis == 0 || kind == 4)) % 4;
                    assert_eq!(b.read32(FBC + 4).unwrap(), mode | (next as u32 * 16));
                }
            }
        }
    }
}

#[test]
fn draw_commands_use_current_vertices_and_attributes_without_a_fifo() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xea80ff00);
    reg(&mut b, 0x100, 0x11);
    for (offset, value) in [(0x904, 2), (0x900, 2), (0x904, 5), (0x900, 5)] {
        reg(&mut b, offset, value);
    }
    let pixel = VRAM + (3 * WIDTH + 3) as u64;
    assert_eq!(b.read8(pixel).unwrap(), 0); // coordinate writes never submit
    reg(&mut b, 0x100, 0x42);
    b.read32(FBC + 0x14).unwrap();
    assert_eq!(b.read8(pixel).unwrap(), 0x42);
    reg(&mut b, 0x100, 0x73);
    b.read32(FBC + 0x10).unwrap(); // plain STATUS never submits
    assert_eq!(b.read8(pixel).unwrap(), 0x42);
    b.read32(FBC + 0x14).unwrap(); // DRAW explicitly resubmits the same bank
    assert_eq!(b.read8(pixel).unwrap(), 0x73);
}

#[test]
fn relative_rectangles_and_lines_share_coordinates_without_stale_fills() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe980ff00); // overlapping: relative extents include endpoint
    reg(&mut b, 0x100, 0x31);
    for (x, y, w, h) in [(2, 2, 3, 2), (10, 10, 1, 1)] {
        for (r, v) in [(0x904, y), (0x900, x), (0x914, h), (0x910, w)] {
            reg(&mut b, r, v);
        }
        b.read32(FBC + 0x14).unwrap();
    }
    reg(&mut b, 0x100, 0x5a);
    for (r, v) in [(0x844, 4), (0x840, 7), (0x844, 6), (0x840, 7)] {
        reg(&mut b, r, v);
    }
    b.read32(FBC + 0x14).unwrap();
    for y in 0..15 {
        for x in 0..15 {
            let expected = if x == 7 && (4..=6).contains(&y) {
                0x5a
            } else if ((2..=5).contains(&x) && (2..=4).contains(&y))
                || ((10..=11).contains(&x) && (10..=11).contains(&y))
            {
                0x31
            } else {
                0
            };
            assert_eq!(
                b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(),
                expected,
                "{x},{y}"
            );
        }
    }
}

#[test]
fn apoint_arectx_spans_obey_draw_submission_offsets_and_overlap_mode() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe980ff00);
    reg(&mut b, 0x100, 0x42);
    reg(&mut b, 0xc0, 10);
    reg(&mut b, 0xc4, 20);
    reg(&mut b, 0x804, 3);
    reg(&mut b, 0x800, 2);
    reg(&mut b, 0x900, 5);
    let row = VRAM + (23 * WIDTH) as u64;
    assert_eq!(b.read8(row + 12).unwrap(), 0);
    // Offsets apply during indexed writes, never a second time at DRAW.
    reg(&mut b, 0xc0, 100);
    reg(&mut b, 0xc4, 200);
    b.read32(FBC + 0x14).unwrap();
    for x in 0..20 {
        assert_eq!(
            b.read8(row + x).unwrap(),
            if (12..=15).contains(&x) { 0x42 } else { 0 }
        );
    }
    reg(&mut b, 0x108, 0xea80ff00); // nonoverlapping lines have no area
    reg(&mut b, 0x100, 0x99);
    b.read32(FBC + 0x14).unwrap();
    assert_eq!(b.read8(row + 12).unwrap(), 0x42);
}

#[test]
fn line_clipping_and_midpoint_ties_do_not_depend_on_endpoint_order() {
    let cases = [
        ((1, 1), (5, 3), [(1, 1), (2, 2), (3, 2), (4, 3), (5, 3)]),
        ((1, 3), (5, 1), [(1, 3), (2, 3), (3, 2), (4, 2), (5, 1)]),
        ((1, 1), (3, 5), [(1, 1), (2, 2), (2, 3), (3, 4), (3, 5)]),
        ((3, 1), (1, 5), [(3, 1), (3, 2), (2, 3), (2, 4), (1, 5)]),
    ];
    for (start, end, expected) in cases {
        for reverse in [false, true] {
            let mut b = bus();
            drawing(&mut b);
            reg(&mut b, 0x108, 0xe980ff00);
            reg(&mut b, 0x100, 0x5a);
            reg(&mut b, 0xe0, 2);
            reg(&mut b, 0xf0, 4);
            reg(&mut b, 0xe4, 2);
            reg(&mut b, 0xf4, 4);
            let points = if reverse { [end, start] } else { [start, end] };
            for (x, y) in points {
                reg(&mut b, 0x844, y);
                reg(&mut b, 0x840, x);
            }
            b.read32(FBC + 0x14).unwrap();
            for y in 0..7 {
                for x in 0..7 {
                    let visible =
                        expected.contains(&(x, y)) && (2..=4).contains(&x) && (2..=4).contains(&y);
                    assert_eq!(
                        b.read8(VRAM + u64::from(y * WIDTH as u32 + x)).unwrap(),
                        if visible { 0x5a } else { 0 },
                        "{start:?} {end:?}, reverse={reverse}, {x},{y}"
                    );
                }
            }
        }
    }
}

#[test]
fn direct_coordinates_restore_context_and_font_does_not_reapply_offsets() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xea80ff00);
    reg(&mut b, 0x100, 0x6c);
    reg(&mut b, 0xc0, 100);
    reg(&mut b, 0xc4, 200);
    for (i, (x, y)) in [(2, 3), (5, 3), (5, 6), (2, 6)].into_iter().enumerate() {
        reg(&mut b, 0x80 + i as u64 * 16, x);
        reg(&mut b, 0x84 + i as u64 * 16, y);
    }
    b.read32(FBC + 0x14).unwrap();
    assert_eq!(b.read8(VRAM + (4 * WIDTH + 3) as u64).unwrap(), 0x6c);
    reg(&mut b, 0x108, 0xea80cccc); // COLOR8 source copy
    reg(&mut b, 0x80, 8);
    reg(&mut b, 0x90, 8);
    reg(&mut b, 0x84, 9);
    reg(&mut b, 0x1c, 0xab000000);
    assert_eq!(b.read8(VRAM + (9 * WIDTH + 8) as u64).unwrap(), 0xab);
    assert_eq!(b.read8(VRAM + (209 * WIDTH + 108) as u64).unwrap(), 0);
}

#[test]
fn rasterop_zero_selectors_preserve_masks_and_overlap_but_load_truth_table() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xfd80cccc);
    reg(&mut b, 0x108, 0x6666);
    assert_eq!(b.read32(FBC + 0x108).unwrap(), 0xfd806666);
    reg(&mut b, 0x108, 0x0200ff00); // change only polygon selector and truth table
    assert_eq!(b.read32(FBC + 0x108).unwrap(), 0xfe80ff00);
    reg(&mut b, 0x10c, 0x0f);
    reg(&mut b, 0x110, 0x80000000);
    reg(&mut b, 0x100, 0xff);
    b.write32(VRAM, 0x30303030).unwrap();
    rectangle(&mut b, 0, 0, 4, 1);
    assert_eq!(b.read32(VRAM).unwrap(), 0x3f303030);
}

#[test]
fn indexed_wrapping_and_extreme_lines_are_bounded_and_faults_leave_vram_intact() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe980ff00);
    reg(&mut b, 0x100, 0x5a);
    reg(&mut b, 0xc0, 1);
    reg(&mut b, 0x800, i32::MAX as u32);
    assert_eq!(b.read32(FBC + 0x80).unwrap(), i32::MIN as u32);
    reg(&mut b, 0xc0, 0);
    for (x, y) in [(i32::MIN as u32, 0), (i32::MAX as u32, 1)] {
        reg(&mut b, 0x844, y);
        reg(&mut b, 0x840, x);
    }
    b.read32(FBC + 0x14).unwrap();
    assert_eq!(b.read32(VRAM + WIDTH as u64).unwrap(), 0x5a5a5a5a);
    assert_eq!(b.read32(VRAM).unwrap(), 0);
    // An unsupported quadrilateral must not partially draw or corrupt the bank.
    for (i, (x, y)) in [(1, 1), (5, 5), (9, 1), (5, 3)].into_iter().enumerate() {
        reg(&mut b, 0x80 + i as u64 * 16, x);
        reg(&mut b, 0x84 + i as u64 * 16, y);
    }
    let generation = b.cgsix().unwrap().generation();
    assert_eq!(
        b.read32(FBC + 0x14).unwrap_err().kind,
        BusErrorKind::Unsupported("CGsix polygon with more than two active edges")
    );
    assert_eq!(b.cgsix().unwrap().generation(), generation);
    assert_eq!(b.read32(VRAM + WIDTH as u64).unwrap(), 0x5a5a5a5a);
}

#[test]
fn adjacent_nonoverlapping_triangles_tile_without_gaps_or_double_xor() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xea806666); // XOR with all-one pattern
    for triangle in [[(1, 1), (5, 1), (1, 5)], [(5, 1), (5, 5), (1, 5)]] {
        for (x, y) in triangle {
            reg(&mut b, 0x884, y);
            reg(&mut b, 0x880, x);
        }
        b.read32(FBC + 0x14).unwrap();
    }
    for y in 0..7 {
        for x in 0..7 {
            let expected = if (1..5).contains(&x) && (1..5).contains(&y) {
                255
            } else {
                0
            };
            assert_eq!(
                b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(),
                expected,
                "{x},{y}"
            );
        }
    }
}

#[test]
fn overlapping_triangle_edges_and_interior_apply_rop_once_per_pixel() {
    let mut b = bus();
    drawing(&mut b);
    reg(&mut b, 0x108, 0xe9806666);
    for y in 0..7 {
        for x in 0..7 {
            b.write8(VRAM + (y * WIDTH + x) as u64, 0x3c).unwrap();
        }
    }
    for (x, y) in [(1, 1), (5, 1), (1, 5)] {
        reg(&mut b, 0x884, y);
        reg(&mut b, 0x880, x);
    }
    b.read32(FBC + 0x14).unwrap();
    for y in 0..7 {
        for x in 0..7 {
            let expected = if x >= 1 && y >= 1 && x + y <= 6 {
                0xc3
            } else {
                0x3c
            };
            assert_eq!(
                b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(),
                expected,
                "{x},{y}"
            );
        }
    }
    b.read32(FBC + 0x14).unwrap();
    for y in 0..7 {
        for x in 0..7 {
            assert_eq!(b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(), 0x3c);
        }
    }
}

#[test]
fn quadrilateral_fills_use_ideal_edges_in_nonoverlap_mode() {
    // Each mask names the covered x coordinates of an integer-pixel row.
    let cases = [
        (
            [(1, 1), (6, 1), (4, 5), (3, 5)],
            [0u8, 0b00111110, 0b00111100, 0b00011100, 0b00011000, 0, 0],
        ),
        (
            // A bow tie has four rectangular corners but crossed edges.
            [(1, 1), (5, 5), (1, 5), (5, 1)],
            [0u8, 0b00011110, 0b00001100, 0, 0b00001100, 0, 0],
        ),
    ];
    for (points, rows) in cases {
        for reverse in [false, true] {
            let mut b = bus();
            drawing(&mut b);
            reg(&mut b, 0x108, 0xea80ff00);
            reg(&mut b, 0x100, 0x42);
            let mut vertices = points;
            if reverse {
                vertices.reverse();
            }
            for (x, y) in vertices {
                reg(&mut b, 0x8c4, y);
                reg(&mut b, 0x8c0, x);
            }
            b.read32(FBC + 0x14).unwrap();
            for (y, mask) in rows.into_iter().enumerate() {
                for x in 0..8 {
                    assert_eq!(
                        b.read8(VRAM + (y * WIDTH + x) as u64).unwrap(),
                        if mask & (1 << x) != 0 { 0x42 } else { 0 },
                        "{points:?}, reverse={reverse}, {x},{y}"
                    );
                }
            }
        }
    }
}
