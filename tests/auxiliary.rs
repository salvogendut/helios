use helios::bus::{Bus, MapError};
use helios::devices::{auxiliary::Auxiliary, nvram::Nvram};
use helios::machine::Machine;
use helios::system_bus::{SystemBus, AUXILIARY_BASE, AUXILIARY_POWER_BASE};

#[test]
fn auxiliary_outputs_density_and_terminal_count_follow_independent_pins() {
    let mut aux = Auxiliary::default();
    assert_eq!(aux.read(), 0);
    for input in [false, true] {
        aux.set_floppy_high_density(input);
        for value in 0..=255u8 {
            aux.write(value);
            assert_eq!(aux.read(), (value & 0x0d) | if input { 0x20 } else { 0 });
            assert_eq!(aux.take_terminal_count_pulses(), u64::from(value & 2 != 0));
            assert_eq!(aux.take_terminal_count_pulses(), 0);
        }
    }
    aux.write(2);
    aux.write(2);
    aux.write(0);
    assert_eq!(aux.take_terminal_count_pulses(), 2);
    aux.write(255);
    aux.reset();
    assert_eq!(aux.read(), 0x20); // reset preserves external density
    assert_eq!(aux.take_terminal_count_pulses(), 0);
}

#[test]
fn auxiliary_port_is_byte_only_and_invalid_accesses_have_no_effect() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    bus.write8(AUXILIARY_BASE, 15).unwrap();
    assert_eq!(bus.read8(AUXILIARY_BASE).unwrap(), 13);
    let before = bus.devices().unwrap().auxiliary.clone();
    for error in [
        bus.read16(AUXILIARY_BASE).unwrap_err(),
        bus.read32(AUXILIARY_BASE).unwrap_err(),
        bus.read_doubleword(AUXILIARY_BASE).unwrap_err(),
        bus.write16(AUXILIARY_BASE, 0).unwrap_err(),
        bus.write32(AUXILIARY_BASE, 0).unwrap_err(),
        bus.write_doubleword(AUXILIARY_BASE, [0, 0]).unwrap_err(),
        bus.write32_masked(AUXILIARY_BASE, 0, 0xff00_0000)
            .unwrap_err(),
        bus.ldstub8(AUXILIARY_BASE).unwrap_err(),
        bus.swap32(AUXILIARY_BASE, 0).unwrap_err(),
        bus.load_image(AUXILIARY_BASE, &[0]).unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop());
        assert_eq!(bus.devices().unwrap().auxiliary, before);
    }
    assert_eq!(bus.map_ram(AUXILIARY_BASE, 4), Err(MapError::Overlap));
    assert_eq!(bus.map_rom(AUXILIARY_BASE, &[0; 4]), Err(MapError::Overlap));
}

#[test]
fn prom_byte_access_reads_and_controls_led_then_reset_clears_outputs() {
    // stb g2,[g1]; ldub [g1],g3
    let prom: Vec<u8> = [0xc428_4000u32, 0xc608_4000]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    let mut m = Machine::sun4m(8192, &prom).unwrap();
    m.cpu.write_reg(1, AUXILIARY_BASE as u32);
    m.cpu.write_reg(2, 0xff);
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(3), 0x0d);
    assert_eq!(
        m.bus
            .devices_mut()
            .unwrap()
            .auxiliary
            .take_terminal_count_pulses(),
        1
    );
    m.bus.reset_devices();
    assert_eq!(m.bus.read8(AUXILIARY_BASE).unwrap(), 0);
}

#[test]
fn auxiliary_power_port_preserves_only_scratch_and_stops_before_power_off() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    assert_eq!(bus.read8(AUXILIARY_POWER_BASE).unwrap(), 0);
    for value in (0..=254u8).step_by(2) {
        bus.write8(AUXILIARY_POWER_BASE, value).unwrap();
        assert_eq!(bus.read8(AUXILIARY_POWER_BASE).unwrap(), value & 12);
        assert_eq!(bus.read8(AUXILIARY_BASE).unwrap(), 0);
        let before = bus.devices().unwrap().auxiliary.clone();
        assert!(bus
            .write8(AUXILIARY_POWER_BASE, value | 1)
            .unwrap_err()
            .kind
            .is_host_stop());
        assert_eq!(bus.devices().unwrap().auxiliary, before);
    }
    assert!(bus
        .write32(AUXILIARY_POWER_BASE, 0)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(bus
        .ldstub8(AUXILIARY_POWER_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert_eq!(bus.read8(AUXILIARY_POWER_BASE).unwrap(), 12);
    bus.reset_devices();
    assert_eq!(bus.read8(AUXILIARY_POWER_BASE).unwrap(), 0);
}
