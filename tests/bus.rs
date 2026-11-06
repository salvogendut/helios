use helios::bus::{Bus, BusErrorKind, MapError, MemoryBus};

#[test]
fn image_bytes_and_narrow_stores_use_big_endian_lanes() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0x1000, 8).unwrap();
    bus.load_image(0x1000, &[0x12, 0x34, 0x56, 0x78, 0x9a])
        .unwrap();
    assert_eq!(bus.read32(0x1000).unwrap(), 0x1234_5678);
    for (offset, expected) in [0x12, 0x34, 0x56, 0x78, 0x9a].into_iter().enumerate() {
        assert_eq!(bus.read8(0x1000 + offset as u64).unwrap(), expected);
    }
    assert_eq!(bus.read16(0x1000).unwrap(), 0x1234);
    assert_eq!(bus.read16(0x1002).unwrap(), 0x5678);
    bus.write8(0x1001, 0xab).unwrap();
    bus.write16(0x1002, 0xcdef).unwrap();
    assert_eq!(bus.read32(0x1000).unwrap(), 0x12ab_cdef);
    bus.load_image(0x1003, &[0x11, 0x22]).unwrap();
    assert_eq!(bus.read32(0x1000).unwrap(), 0x12ab_cd11);
    assert_eq!(bus.read32(0x1004).unwrap(), 0x2200_0000);
}

#[test]
fn invalid_maps_and_out_of_range_images_are_atomic() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 8).unwrap();
    assert_eq!(bus.map_ram(4, 8), Err(MapError::Overlap));
    assert_eq!(bus.map_ram(1, 4), Err(MapError::InvalidRange));
    assert_eq!(bus.map_ram(8, 0), Err(MapError::InvalidRange));
    assert_eq!(bus.map_ram(u64::MAX - 3, 8), Err(MapError::InvalidRange));
    assert_eq!(
        bus.load_image(6, &[1, 2, 3]).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    assert_eq!(bus.read32(4).unwrap(), 0);
    assert_eq!(bus.read32(8).unwrap_err().kind, BusErrorKind::Unmapped);
    assert_eq!(
        bus.write16(1, 0xffff).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(bus.read32(0).unwrap(), 0);
}

#[test]
fn rom_rejects_writes_and_reports_original_byte_address() {
    let mut bus = MemoryBus::default();
    bus.map_rom(0x7000_0000, &[0x81, 0xc3, 0xe0, 0x08]).unwrap();
    assert_eq!(bus.read32(0x7000_0000).unwrap(), 0x81c3_e008);
    let error = bus.write8(0x7000_0003, 0).unwrap_err();
    assert_eq!(
        (error.kind, error.address),
        (BusErrorKind::ReadOnly, 0x7000_0003)
    );
    assert_eq!(
        bus.load_image(0x7000_0000, &[0]).unwrap_err().kind,
        BusErrorKind::ReadOnly
    );
    assert_eq!(bus.read32(0x7000_0000).unwrap(), 0x81c3_e008);
}

#[test]
fn physical_addresses_are_not_truncated_to_32_bits() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 4).unwrap();
    bus.map_ram(0x1_0000_0000, 4).unwrap();
    bus.write32(0x1_0000_0000, 0xfeed_beef).unwrap();
    assert_eq!(bus.read32(0).unwrap(), 0);
    assert_eq!(bus.read32(0x1_0000_0000).unwrap(), 0xfeed_beef);
}

#[test]
fn doubleword_transfer_validates_both_words_before_writing() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 4).unwrap();
    bus.write32(0, 0x1122_3344).unwrap();
    assert_eq!(
        bus.write_doubleword(0, [1, 2]).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    assert_eq!(bus.read32(0).unwrap(), 0x1122_3344);
    bus.map_rom(4, &0x5566_7788u32.to_be_bytes()).unwrap();
    assert_eq!(bus.read_doubleword(0).unwrap(), [0x1122_3344, 0x5566_7788]);
    assert_eq!(
        bus.write_doubleword(0, [1, 2]).unwrap_err().kind,
        BusErrorKind::ReadOnly
    );
    assert_eq!(bus.read_doubleword(0).unwrap(), [0x1122_3344, 0x5566_7788]);
    // Separate adjacent RAM mappings still form one valid doubleword.
    bus.map_ram(8, 4).unwrap();
    bus.map_ram(12, 4).unwrap();
    bus.write_doubleword(8, [0xaabb_ccdd, 0xeeff_0011]).unwrap();
    assert_eq!(bus.read_doubleword(8).unwrap(), [0xaabb_ccdd, 0xeeff_0011]);
    assert_eq!(
        bus.write_doubleword(12, [0, 0]).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(
        bus.read_doubleword(12).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
}

#[test]
fn bus_exchanges_preserve_byte_lanes_and_reject_rom_without_mutation() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 4).unwrap();
    bus.map_rom(4, &0x1234_5678u32.to_be_bytes()).unwrap();
    bus.write32(0, 0x0180_feff).unwrap();
    for (offset, old) in [(0, 1), (1, 0x80), (2, 0xfe), (3, 0xff)] {
        assert_eq!(bus.ldstub8(offset).unwrap(), old);
    }
    assert_eq!(bus.swap32(0, 0xdead_beef).unwrap(), u32::MAX);
    assert_eq!(bus.read32(0).unwrap(), 0xdead_beef);
    assert_eq!(bus.swap32(1, 0).unwrap_err().kind, BusErrorKind::Misaligned);
    assert_eq!(bus.ldstub8(5).unwrap_err().address, 5);
    assert_eq!(bus.swap32(4, 0).unwrap_err().kind, BusErrorKind::ReadOnly);
    assert_eq!(bus.read32(4).unwrap(), 0x1234_5678);
}
