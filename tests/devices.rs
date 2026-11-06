use helios::bus::{Bus, BusErrorKind, MapError};
use helios::cpu::FaultKind;
use helios::devices::nvram::{Nvram, NvramError, CLOCK_CONTROL, IDPROM_OFFSET, NVRAM_SIZE};
use helios::devices::serial::{Channel, Serial, OUTPUT_CAPACITY};
use helios::machine::{serial_demo_image, Machine};
use helios::mmu::{MemoryError, Srmmu, Width, PCR_BM, PCR_EN, PCR_NF};
use helios::system_bus::{SystemBus, NVRAM_BASE, SERIAL_BASE};

fn wr(s: &mut Serial, channel: Channel, reg: u8, value: u8) {
    let control = if channel == Channel::A { 4 } else { 0 };
    s.write(control, reg).unwrap();
    s.write(control, value).unwrap();
}
fn rr(s: &mut Serial, channel: Channel, reg: u8) -> u8 {
    let control = if channel == Channel::A { 4 } else { 0 };
    s.write(control, reg).unwrap();
    s.read(control).unwrap()
}
fn configure(s: &mut Serial, channel: Channel) {
    wr(s, channel, 4, 0x44); // x16, asynchronous, one stop, no parity
    wr(s, channel, 14, 3); // BRG, no loopback
    wr(s, channel, 3, 0xc1); // 8-bit receive enabled
    wr(s, channel, 5, 0xea); // 8-bit transmit, DTR/RTS
}
fn rtc(n: &Nvram) -> [u8; 7] {
    std::array::from_fn(|i| n.read(CLOCK_CONTROL + 1 + i).unwrap())
}
fn set_rtc(n: &mut Nvram, fields: [u8; 7]) {
    n.write(CLOCK_CONTROL, 0x80).unwrap();
    for (i, value) in fields.into_iter().enumerate() {
        n.write(CLOCK_CONTROL + 1 + i, value).unwrap();
    }
    n.write(CLOCK_CONTROL, 0).unwrap();
}

#[test]
fn idprom_has_ss5_type_mac_serial_and_independently_checkable_checksum() {
    let n = Nvram::default();
    let bytes = n.image();
    assert_eq!(
        &bytes[IDPROM_OFFSET..IDPROM_OFFSET + 16],
        &[1, 0x80, 8, 0, 0x20, 0xc0, 0xff, 0xee, 0, 0, 0, 0, 0xc0, 0xff, 0xee, 0xa9]
    );
    assert_eq!(
        bytes[IDPROM_OFFSET..IDPROM_OFFSET + 16]
            .iter()
            .fold(0, |sum, byte| sum ^ byte),
        0
    );
    let custom = Nvram::new([2, 1, 2, 3, 4, 5], [0xaa, 0xbb, 0xcc]).image();
    assert_eq!(
        &custom[IDPROM_OFFSET + 12..IDPROM_OFFSET + 15],
        &[0xaa, 0xbb, 0xcc]
    );
    assert_eq!(
        custom[IDPROM_OFFSET..IDPROM_OFFSET + 16]
            .iter()
            .fold(0, |sum, byte| sum ^ byte),
        0
    );
    assert_eq!(rtc(&n), [0, 0, 0, 7, 1, 1, 0x32]);
}

#[test]
fn serial_break_status_and_termination_null_are_distinct_from_ctrl_c() {
    let mut s = Serial::default();
    configure(&mut s, Channel::A);
    assert!(s.receive(Channel::A, 3));
    assert_eq!(s.read(4).unwrap() & 0x81, 1);
    assert_eq!(s.read(6).unwrap(), 3);
    s.set_break(Channel::A, true).unwrap();
    assert_eq!(s.read(4).unwrap() & 0x81, 0x80);
    assert_eq!(s.read(0).unwrap() & 0x80, 0);
    assert!(!s.receive(Channel::A, b'x'));
    s.write(4, 0x10).unwrap(); // reset external/status latch while BREAK held
    assert_eq!(s.read(4).unwrap() & 0x80, 0x80);
    s.set_break(Channel::A, false).unwrap();
    assert_eq!(s.read(4).unwrap() & 0x81, 1);
    assert_eq!(s.read(6).unwrap(), 0);
    assert_eq!(s.read(4).unwrap() & 0x81, 0);
    s.set_break(Channel::A, false).unwrap(); // no duplicate termination byte
    assert_eq!(s.read(4).unwrap() & 1, 0);
    s.set_break(Channel::A, true).unwrap();
    s.reset();
    assert_eq!(s.read(4).unwrap() & 0x81, 0);
}

#[test]
fn nvram_raw_image_roundtrip_preserves_guest_configuration_without_repair() {
    let mut n = Nvram::default();
    n.write(0x123, 0xab).unwrap();
    n.write(IDPROM_OFFSET + 15, 0xff).unwrap();
    let bytes = n.image();
    let restored = Nvram::from_image(&bytes).unwrap();
    assert_eq!(restored.image(), bytes);
    assert_eq!(restored.read(IDPROM_OFFSET + 15).unwrap(), 0xff);
    assert_eq!(
        Nvram::from_image(&bytes[..8191]).unwrap_err(),
        NvramError::InvalidSize
    );
    let mut invalid = bytes.clone();
    invalid[CLOCK_CONTROL + 1] = 0x7a;
    assert_eq!(
        Nvram::from_image(&invalid).unwrap_err(),
        NvramError::InvalidClock
    );
    assert_eq!(n.read(NVRAM_SIZE).unwrap_err(), BusErrorKind::Unmapped);
}

#[test]
fn rtc_read_latch_holds_visible_fields_while_counter_advances() {
    let mut n = Nvram::default();
    n.advance_seconds(59);
    n.write(CLOCK_CONTROL, 0x40).unwrap();
    n.advance_seconds(2);
    assert_eq!(rtc(&n)[..3], [0x59, 0, 0]);
    n.write(CLOCK_CONTROL, 0).unwrap();
    assert_eq!(rtc(&n)[..3], [1, 1, 0]);
}

#[test]
fn rtc_write_latch_calendar_rollover_and_stop_bit() {
    let mut n = Nvram::default();
    set_rtc(&mut n, [0x59, 0x59, 0x23, 1, 0x28, 2, 0x32]);
    n.advance_seconds(1);
    assert_eq!(rtc(&n), [0, 0, 0, 2, 0x29, 2, 0x32]);
    n.advance_seconds(86400);
    assert_eq!(rtc(&n), [0, 0, 0, 3, 1, 3, 0x32]);
    set_rtc(&mut n, [0x59, 0x59, 0x23, 7, 0x31, 0x12, 0x99]);
    n.advance_seconds(1);
    assert_eq!(rtc(&n), [0, 0, 0, 1, 1, 1, 0]);
    set_rtc(&mut n, [0x80, 0, 0, 1, 1, 1, 0]);
    n.advance_seconds(u64::MAX);
    assert_eq!(rtc(&n), [0x80, 0, 0, 1, 1, 1, 0]);
    set_rtc(&mut n, [0, 0, 0, 1, 1, 1, 0]);
    n.advance_seconds(60);
    assert_eq!(rtc(&n)[..3], [0, 1, 0]);
}

#[test]
fn rtc_writes_commit_together_and_reject_invalid_dates_without_clearing_w() {
    let mut n = Nvram::default();
    n.write(CLOCK_CONTROL, 0x80).unwrap();
    n.advance_seconds(3600);
    assert_eq!(rtc(&n)[2], 0);
    n.write(CLOCK_CONTROL + 5, 0x31).unwrap();
    n.write(CLOCK_CONTROL + 6, 2).unwrap();
    let before = n.clone();
    assert!(matches!(
        n.write(CLOCK_CONTROL, 0),
        Err(BusErrorKind::Unsupported(_))
    ));
    assert_eq!(n, before);
    n.write(CLOCK_CONTROL + 5, 0x29).unwrap();
    n.write(CLOCK_CONTROL, 0).unwrap();
    assert_eq!(rtc(&n), [0, 0, 0, 7, 0x29, 2, 0x32]);
    let before = n.clone();
    assert!(n.write(CLOCK_CONTROL, 1).is_err());
    assert_eq!(n, before);
}

#[test]
fn rtc_large_advance_is_bounded_and_keeps_weekday_across_year_wrap() {
    let mut n = Nvram::default();
    n.advance_seconds(36525 * 86400);
    assert_eq!(rtc(&n), [0, 0, 0, 6, 1, 1, 0x32]); // 100-year cycle is 6 weekdays
    n.advance_seconds(u64::MAX);
    let fields = rtc(&n);
    assert!((1..=7).contains(&fields[3]));
    assert!(Nvram::from_image(&n.image()).is_ok());
}

#[test]
fn serial_pointer_is_shared_between_channels_and_resets_after_control_access() {
    let mut s = Serial::default();
    s.write(0, 12).unwrap(); // select WR12 through B
    s.write(4, 0x55).unwrap(); // actual register access through A
    assert_eq!(rr(&mut s, Channel::A, 12), 0x55);
    assert_eq!(rr(&mut s, Channel::B, 12), 0);
    assert_eq!(s.read(4).unwrap() & 4, 4); // pointer reset to RR0
    wr(&mut s, Channel::A, 13, 0xaa);
    assert_eq!(rr(&mut s, Channel::A, 9), 0xaa); // NMOS RR9 aliases RR13
    wr(&mut s, Channel::B, 2, 0xa0);
    assert_eq!(rr(&mut s, Channel::A, 2), 0xa0); // shared vector
    assert_eq!(rr(&mut s, Channel::B, 2), 0xa6); // no pending interrupt
    wr(&mut s, Channel::A, 9, 0x10);
    assert_eq!(rr(&mut s, Channel::B, 2), 0xe0);
}

#[test]
fn serial_channels_poll_transmit_and_receive_without_cross_talk() {
    let mut s = Serial::default();
    assert!(!s.receive(Channel::A, b'x')); // receiver disabled
    configure(&mut s, Channel::A);
    configure(&mut s, Channel::B);
    assert_eq!(s.read(4).unwrap() & 5, 4);
    assert!(s.receive(Channel::A, 0x80));
    assert!(s.receive(Channel::A, 0xff));
    assert_eq!(s.read(4).unwrap() & 5, 5);
    assert_eq!(s.read(0).unwrap() & 1, 0);
    assert_eq!(s.read(6).unwrap(), 0x80);
    assert_eq!(rr(&mut s, Channel::A, 8), 0xff);
    assert_eq!(s.read(4).unwrap() & 1, 0);
    s.write(6, b'A').unwrap();
    s.write(2, b'B').unwrap();
    assert_eq!(s.take_output(Channel::A), b"A");
    assert_eq!(s.take_output(Channel::B), b"B");
    assert_eq!(rr(&mut s, Channel::A, 1) & 1, 1);
}

#[test]
fn serial_data_port_does_not_consume_the_shared_control_pointer() {
    let mut s = Serial::default();
    configure(&mut s, Channel::A);
    wr(&mut s, Channel::A, 12, 0x55);
    s.write(4, 12).unwrap();
    s.write(6, b'x').unwrap();
    assert_eq!(s.read(4).unwrap(), 0x55);
    assert_eq!(s.take_output(Channel::A), b"x");
    // WR8 shares the same transmit data path.
    wr(&mut s, Channel::A, 8, b'y');
    assert_eq!(s.take_output(Channel::A), b"y");
}

#[test]
fn serial_disabled_transmitter_holds_one_byte_and_enable_releases_it() {
    let mut s = Serial::default();
    wr(&mut s, Channel::A, 14, 3);
    s.write(6, b'Z').unwrap();
    assert_eq!(s.read(4).unwrap() & 4, 0);
    assert_eq!(rr(&mut s, Channel::A, 1) & 1, 0);
    let before = s.clone();
    assert_eq!(s.write(6, b'!'), Err(BusErrorKind::WouldBlock));
    assert_eq!(s, before);
    wr(&mut s, Channel::A, 5, 0x68);
    assert_eq!(s.take_output(Channel::A), b"Z");
    assert_eq!(s.read(4).unwrap() & 4, 4);
}

#[test]
fn serial_fifos_are_bounded_and_backpressure_does_not_lose_data() {
    let mut s = Serial::default();
    configure(&mut s, Channel::A);
    for byte in b"abc" {
        assert!(s.receive(Channel::A, *byte));
    }
    assert!(!s.receive(Channel::A, b'd'));
    assert_eq!(s.read(6).unwrap(), b'a');
    assert!(s.receive(Channel::A, b'd'));
    assert_eq!(
        [s.read(6).unwrap(), s.read(6).unwrap(), s.read(6).unwrap()],
        *b"bcd"
    );
    for _ in 0..OUTPUT_CAPACITY {
        s.write(6, b'x').unwrap();
    }
    let before = s.clone();
    assert_eq!(s.write(6, b'y'), Err(BusErrorKind::WouldBlock));
    assert_eq!(s, before);
    assert_eq!(s.take_output(Channel::A).len(), OUTPUT_CAPACITY);
    s.write(6, b'y').unwrap();
    assert_eq!(s.take_output(Channel::A), b"y");
}

#[test]
fn serial_loopback_masks_character_width_and_channel_reset_is_selective() {
    let mut s = Serial::default();
    configure(&mut s, Channel::A);
    configure(&mut s, Channel::B);
    wr(&mut s, Channel::A, 14, 0x13); // local loopback
    wr(&mut s, Channel::A, 3, 0x41); // 7-bit receive
    s.write(6, 0xff).unwrap();
    assert_eq!(s.read(6).unwrap(), 0x7f);
    assert!(s.take_output(Channel::A).is_empty());
    s.write(2, b'B').unwrap();
    assert!(s.receive(Channel::B, b'b'));
    wr(&mut s, Channel::A, 9, 0x80); // reset A, retain B
    assert!(!s.receive(Channel::A, b'a'));
    assert_eq!(s.read(2).unwrap(), b'b');
    assert_eq!(s.take_output(Channel::B), b"B");
    wr(&mut s, Channel::A, 9, 0xc0);
    assert!(!s.receive(Channel::B, b'b'));
}

#[test]
fn unsupported_serial_modes_stop_before_register_or_pointer_changes() {
    for (reg, value) in [
        (1, 0x10),
        (4, 0),
        (5, 0x10),
        (10, 0x20),
        (14, 0x20),
        (14, 0x08),
        (15, 4),
    ] {
        let mut s = Serial::default();
        s.write(4, reg).unwrap();
        let before = s.clone();
        assert!(matches!(
            s.write(4, value),
            Err(BusErrorKind::Unsupported(_))
        ));
        assert_eq!(s, before);
    }
}

#[test]
fn serial_disabled_dpll_configuration_keeps_async_io_working() {
    for channel in [Channel::A, Channel::B] {
        let control = if channel == Channel::A { 4 } else { 0 };
        let mut s = Serial::default();
        configure(&mut s, channel);
        for command in [0x80, 0xa0, 0xc0, 0xe0, 0x40, 0x60, 0] {
            wr(&mut s, channel, 14, command | 3);
            assert_eq!(rr(&mut s, channel, 10), 0); // no missing-clock latches
            assert!(s.receive(channel, b'R'));
            assert_eq!(s.read(control + 2).unwrap(), b'R');
            s.write(control + 2, b'T').unwrap();
            assert_eq!(s.take_output(channel), b"T");
        }
        s.write(control, 14).unwrap();
        let before = s.clone();
        assert!(s.write(control, 0x23).is_err()); // starting DPLL remains unsupported
        assert_eq!(s, before);
        s.write(control, 0x83).unwrap(); // retry still addresses WR14
        assert_eq!(s.read(control).unwrap() & 4, 4); // then RR0
    }
}

#[test]
fn keyboard_mouse_scc_has_independent_state_and_resets_with_the_machine() {
    use helios::system_bus::KEYBOARD_SERIAL_BASE;
    let mut m = Machine::sun4m(8192, &0x0100_0000u32.to_be_bytes()).unwrap();
    // Each chip has its own pointer; within a chip it is shared by A and B.
    m.bus.write8(KEYBOARD_SERIAL_BASE, 12).unwrap();
    m.bus.write8(SERIAL_BASE, 13).unwrap();
    m.bus.write8(KEYBOARD_SERIAL_BASE + 4, 0x55).unwrap();
    m.bus.write8(SERIAL_BASE + 4, 0xaa).unwrap();
    for (base, reg, value) in [(KEYBOARD_SERIAL_BASE, 12, 0x55), (SERIAL_BASE, 13, 0xaa)] {
        m.bus.write8(base + 4, reg).unwrap();
        assert_eq!(m.bus.read8(base + 4).unwrap(), value);
    }
    let devices = m.bus.devices_mut().unwrap();
    configure(&mut devices.keyboard_serial, Channel::A);
    configure(&mut devices.keyboard_serial, Channel::B);
    assert!(devices.keyboard_serial.receive(Channel::A, b'K'));
    assert!(devices.keyboard_serial.receive(Channel::B, b'M'));
    assert_eq!(m.bus.read8(KEYBOARD_SERIAL_BASE + 6).unwrap(), b'K');
    assert_eq!(m.bus.read8(KEYBOARD_SERIAL_BASE + 2).unwrap(), b'M');
    assert_eq!(m.bus.read8(SERIAL_BASE + 4).unwrap() & 1, 0);
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .keyboard_serial
        .receive(Channel::A, b'R'));
    assert!(m
        .bus
        .read32(KEYBOARD_SERIAL_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .ldstub8(KEYBOARD_SERIAL_BASE + 6)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert_eq!(m.bus.read8(KEYBOARD_SERIAL_BASE + 6).unwrap(), b'R');
    assert_eq!(
        m.bus.map_ram(KEYBOARD_SERIAL_BASE, 8),
        Err(MapError::Overlap)
    );
    assert!(m.bus.load_image(KEYBOARD_SERIAL_BASE, &[0; 8]).is_err());
    m.reset();
    assert!(!m
        .bus
        .devices_mut()
        .unwrap()
        .keyboard_serial
        .receive(Channel::A, b'K'));
    assert!(!m
        .bus
        .devices_mut()
        .unwrap()
        .keyboard_serial
        .receive(Channel::B, b'M'));
}

#[test]
fn device_bus_rejects_wide_atomic_and_image_access_before_side_effects() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    configure(&mut bus.devices_mut().unwrap().serial, Channel::A);
    bus.devices_mut().unwrap().serial.receive(Channel::A, b'x');
    assert!(matches!(
        bus.read32(SERIAL_BASE + 4).unwrap_err().kind,
        BusErrorKind::Unsupported(_)
    ));
    assert!(bus.read16(SERIAL_BASE + 6).is_err());
    assert!(bus.read_doubleword(SERIAL_BASE).is_err());
    assert!(bus.swap32(SERIAL_BASE + 4, 0).is_err());
    assert!(bus.ldstub8(SERIAL_BASE + 6).is_err());
    assert_eq!(bus.read8(SERIAL_BASE + 6).unwrap(), b'x');
    assert_eq!(bus.map_ram(SERIAL_BASE, 8), Err(MapError::Overlap));
    assert!(bus.load_image(NVRAM_BASE, &[1, 2]).is_err());
    assert_eq!(bus.read8(NVRAM_BASE).unwrap(), 0);
    assert!(bus.read32(NVRAM_BASE).is_err());
    assert_eq!(bus.read8(NVRAM_BASE + 0x1fd9).unwrap(), 0x80);
    assert_eq!(
        bus.read8(SERIAL_BASE + 5).unwrap(),
        bus.read8(SERIAL_BASE + 4).unwrap()
    );
}

#[test]
fn reset_retains_nvram_clock_and_completed_output_but_resets_serial_receiver() {
    let mut m = Machine::sun4m(4096, &0x0100_0000u32.to_be_bytes()).unwrap();
    m.bus.write8(NVRAM_BASE + 0x123, 0x55).unwrap();
    m.advance_seconds(12);
    let serial = &mut m.bus.devices_mut().unwrap().serial;
    configure(serial, Channel::A);
    serial.receive(Channel::A, b'x');
    serial.write(6, b'y').unwrap();
    m.reset();
    assert_eq!(m.bus.read8(NVRAM_BASE + 0x123).unwrap(), 0x55);
    assert_eq!(m.bus.read8(NVRAM_BASE + 0x1ff9).unwrap(), 0x12);
    assert_eq!(m.bus.read8(SERIAL_BASE + 4).unwrap() & 1, 0);
    assert!(!m
        .bus
        .devices_mut()
        .unwrap()
        .serial
        .receive(Channel::A, b'x'));
    assert_eq!(
        m.bus.devices_mut().unwrap().serial.take_output(Channel::A),
        b"y"
    );
}

#[test]
fn unsupported_device_access_is_a_host_stop_even_in_mmu_no_fault_mode() {
    let prom = 0xc400_6000u32.to_be_bytes(); // ld [g1], g2
    let mut m = Machine::sun4m(4096, &prom).unwrap();
    m.cpu.write_reg(1, SERIAL_BASE as u32 + 4);
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_BM | PCR_NF)
        .unwrap();
    let error = m.step().unwrap_err();
    assert!(
        matches!(error.kind, FaultKind::UnsupportedMemory(MemoryError::Bus(e)) if e.kind.is_host_stop())
    );
    assert_eq!(m.cpu.error_state(), None);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert_eq!(m.cpu.pc(), 0);
}

#[test]
fn page_walk_and_probe_propagate_unsupported_device_width_without_fabricating_fault() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    bus.map_ram(0, 4096).unwrap();
    bus.write32(0, 0x0711_0001).unwrap(); // root points to a table at serial base
    let mut mmu = Srmmu::default();
    mmu.store(&mut bus, 0, 4, Width::Word, PCR_EN | PCR_NF)
        .unwrap();
    for asi in [11, 3] {
        assert!(
            matches!(mmu.load(&mut bus, 0, asi, Width::Word), Err(MemoryError::Bus(e)) if e.kind.is_host_stop())
        );
        assert_eq!(mmu.fault_status(), 0);
    }
}

#[test]
fn translated_device_bytes_do_not_consume_adjacent_serial_registers() {
    let mut bus = SystemBus::default();
    bus.map_ram(0, 0x5000).unwrap();
    bus.install_sun4m(Nvram::default()).unwrap();
    // Map VA 0x2000 to the SCC page via a three-level table at 0x2000/3000/4000.
    for (address, value) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4008, 0x0711_008e),
    ] {
        bus.write32(address, value).unwrap();
    }
    configure(&mut bus.devices_mut().unwrap().serial, Channel::A);
    assert!(bus.devices_mut().unwrap().serial.receive(Channel::A, b'x'));
    assert!(bus.devices_mut().unwrap().serial.receive(Channel::A, b'y'));
    let mut mmu = Srmmu::default();
    mmu.store(&mut bus, 0x100, 4, Width::Word, 0x100).unwrap();
    mmu.store(&mut bus, 0, 4, Width::Word, PCR_EN).unwrap();
    assert_eq!(mmu.load(&mut bus, 0x2004, 11, Width::Byte).unwrap() & 1, 1);
    assert_eq!(
        mmu.load(&mut bus, 0x2006, 11, Width::Byte).unwrap(),
        u32::from(b'x')
    );
    mmu.store(&mut bus, 0x2006, 11, Width::Byte, u32::from(b'z'))
        .unwrap();
    assert_eq!(
        bus.devices_mut().unwrap().serial.take_output(Channel::A),
        b"z"
    );
    assert_eq!(
        mmu.load(&mut bus, 0x2006, 11, Width::Byte).unwrap(),
        u32::from(b'y')
    );
    assert_eq!(mmu.load(&mut bus, 0x2004, 11, Width::Byte).unwrap() & 1, 0);
    assert_eq!(bus.read32(0x4008).unwrap(), 0x0711_00ee); // R and M set
}

#[test]
fn serial_prom_verifies_identity_transmits_and_echoes_after_reset() {
    let mut m = Machine::sun4m(8192, &serial_demo_image()).unwrap();
    let mut received = false;
    for _ in 0..256 {
        if !received {
            received = m
                .bus
                .devices_mut()
                .unwrap()
                .serial
                .receive(Channel::A, b'!');
        }
        m.step().unwrap();
    }
    assert!(received);
    assert_eq!(m.cpu.read_reg(3), 0x80);
    assert_eq!(m.cpu.read_reg(5), 0);
    assert_eq!(m.cpu.read_reg(7), 1);
    let devices = m.bus.devices_mut().unwrap();
    assert_eq!(devices.serial.take_output(Channel::A), b"HELIOS!\n");
    assert!(devices.serial.take_output(Channel::B).is_empty());
}

#[test]
fn output_backpressure_stops_before_guest_commit_and_can_be_retried() {
    let prom = 0xc428_6006u32.to_be_bytes(); // stb g2, [g1+6]
    let mut m = Machine::sun4m(4096, &prom).unwrap();
    m.cpu.write_reg(1, SERIAL_BASE as u32);
    m.cpu.write_reg(2, u32::from(b'y'));
    let serial = &mut m.bus.devices_mut().unwrap().serial;
    configure(serial, Channel::A);
    for _ in 0..OUTPUT_CAPACITY {
        serial.write(6, b'x').unwrap();
    }
    let before = m.cpu.clone();
    let error = m.step().unwrap_err();
    assert!(
        matches!(error.kind, FaultKind::UnsupportedMemory(MemoryError::Bus(e))
        if e.kind == BusErrorKind::WouldBlock)
    );
    assert_eq!(m.cpu, before);
    assert_eq!(
        m.bus.devices_mut().unwrap().serial.take_output(Channel::A),
        vec![b'x'; OUTPUT_CAPACITY]
    );
    m.step().unwrap();
    assert_eq!(m.cpu.pc(), 4);
    assert_eq!(
        m.bus.devices_mut().unwrap().serial.take_output(Channel::A),
        b"y"
    );
}
