use helios::bus::{Bus, MapError};
use helios::devices::{cs4231::Cs4231, nvram::Nvram};
use helios::system_bus::{SystemBus, CS4231_BASE};

#[test]
fn indirect_register_bank_matches_cs4231_byte_port_layout() {
    let mut cs = Cs4231::default();
    assert_eq!(cs.read_byte(0).unwrap(), 0);
    assert_eq!(cs.read_byte(4).unwrap(), 0);
    cs.write_byte(0, 25).unwrap();
    assert_eq!(cs.read_byte(0).unwrap(), 25);
    assert_eq!(cs.read_byte(4).unwrap(), 0xa0);
    cs.write_byte(0, 12).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0x8a);
    cs.write_byte(4, 0x40).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0xca);
    cs.write_byte(0, 11).unwrap();
    cs.write_byte(4, 0x5a).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0);
    cs.write_byte(0, 25).unwrap();
    cs.write_byte(4, 0x5a).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0xa0);
    cs.write_byte(0, 3).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0);
    cs.write_byte(8, 0x33).unwrap();
    assert_eq!(cs.read_byte(8).unwrap(), 0);
}

#[test]
fn control_write_reset_restores_versions_and_keeps_control_value() {
    let mut cs = Cs4231::default();
    cs.write_byte(0, 12).unwrap();
    cs.write_byte(4, 0x40).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0xca);
    cs.write_byte(0, 25).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0xa0);
    cs.write(0x10, 1).unwrap();
    assert_eq!(cs.read(0x10).unwrap(), 1);
    cs.write_byte(0, 12).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0x8a);
    cs.write_byte(0, 25).unwrap();
    assert_eq!(cs.read_byte(4).unwrap(), 0xa0);
}

#[test]
fn bus_word_and_byte_ports_share_state_and_reset_with_the_machine() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    assert_eq!(bus.read32(CS4231_BASE).unwrap(), 0);
    bus.write8(CS4231_BASE, 25).unwrap();
    assert_eq!(bus.read8(CS4231_BASE).unwrap(), 25);
    assert_eq!(bus.read8(CS4231_BASE + 4).unwrap(), 0xa0);
    bus.write8(CS4231_BASE, 12).unwrap();
    bus.write8(CS4231_BASE + 4, 0x40).unwrap();
    assert_eq!(bus.read8(CS4231_BASE + 4).unwrap(), 0xca);
    bus.write8(CS4231_BASE, 0).unwrap();
    bus.write32(CS4231_BASE + 4, 0x12).unwrap();
    assert_eq!(bus.read8(CS4231_BASE + 4).unwrap(), 0x12);
    bus.write8(CS4231_BASE + 1, 0x5a).unwrap();
    assert_eq!(bus.read8(CS4231_BASE).unwrap(), 0x5a);
    bus.write16(CS4231_BASE + 2, 0x1234).unwrap();
    assert_eq!(bus.read16(CS4231_BASE).unwrap(), 0x1234);
    bus.write32(CS4231_BASE + 0x10, 1).unwrap();
    assert_eq!(bus.read8(CS4231_BASE).unwrap(), 0);
    bus.reset_devices();
    assert_eq!(bus.read32(CS4231_BASE).unwrap(), 0);
    assert_eq!(bus.map_ram(CS4231_BASE, 4), Err(MapError::Overlap));
    assert!(bus
        .write32_masked(CS4231_BASE, 0, 0xff00_0000)
        .unwrap_err()
        .kind
        .is_host_stop());
}
