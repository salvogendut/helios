use helios::devices::{keyboard::Keyboard, nvram::Nvram};
use helios::system_bus::SystemBus;

fn bus() -> SystemBus {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    let d = bus.devices_mut().unwrap();
    d.keyboard = Some(Keyboard::default());
    for (reg, value) in [(4, 0x44), (14, 3), (3, 0xc1), (5, 0xea)] {
        d.keyboard_serial.write(4, reg).unwrap();
        d.keyboard_serial.write(4, value).unwrap();
    }
    bus
}
fn key(bus: &mut SystemBus, code: u8, down: bool) -> bool {
    bus.devices_mut()
        .unwrap()
        .keyboard
        .as_mut()
        .unwrap()
        .key(code, down)
}
fn read(bus: &mut SystemBus) -> u8 {
    bus.advance_ns(8_333_334);
    let s = &mut bus.devices_mut().unwrap().keyboard_serial;
    assert_eq!(s.read(4).unwrap() & 1, 1);
    s.read(6).unwrap()
}
fn no_data(bus: &mut SystemBus) {
    assert_eq!(
        bus.devices_mut().unwrap().keyboard_serial.read(4).unwrap() & 1,
        0
    );
}
fn boot(bus: &mut SystemBus) {
    assert_eq!([read(bus), read(bus), read(bus)], [0xff, 4, 0x7f]);
}
fn command(bus: &mut SystemBus, byte: u8) {
    bus.devices_mut()
        .unwrap()
        .keyboard_serial
        .write(6, byte)
        .unwrap();
}

#[test]
fn identification_obeys_1200_baud_and_layout_command() {
    let mut b = bus();
    b.advance_ns(8_333_333);
    no_data(&mut b);
    b.advance_ns(1);
    assert_eq!(
        b.devices_mut().unwrap().keyboard_serial.read(6).unwrap(),
        0xff
    );
    assert_eq!([read(&mut b), read(&mut b)], [4, 0x7f]);
    command(&mut b, 0x0f);
    assert_eq!([read(&mut b), read(&mut b)], [0xfe, 0x21]); // Type 5 US
    command(&mut b, 7);
    assert_eq!([read(&mut b), read(&mut b)], [0xfe, 0x21]); // Type 5 US
    b.advance_ns(100_000_000);
    no_data(&mut b);
}

#[test]
fn receiver_enable_resends_identification_when_fifo_is_empty() {
    let mut b = bus();
    b.advance_ns(8_333_333);
    assert_eq!([read(&mut b), read(&mut b), read(&mut b)], [0xff, 4, 0x7f]);
    b.devices_mut()
        .unwrap()
        .keyboard_serial
        .write(4, 3)
        .unwrap();
    b.devices_mut()
        .unwrap()
        .keyboard_serial
        .write(4, 0xc1)
        .unwrap();
    assert_eq!([read(&mut b), read(&mut b), read(&mut b)], [0xff, 4, 0x7f]);
}

#[test]
fn scan_codes_modifiers_stop_a_and_idle_are_ordered_without_repeat() {
    let mut b = bus();
    boot(&mut b);
    for (code, down) in [
        (1, true),
        (0x4d, true),
        (0x4d, true),
        (0x4d, false),
        (1, false),
    ] {
        assert!(key(&mut b, code, down));
    }
    assert_eq!(
        (0..5).map(|_| read(&mut b)).collect::<Vec<_>>(),
        [1, 0x4d, 0xcd, 0x81, 0x7f]
    );
    assert!(key(&mut b, 1, false));
    b.advance_ns(1_000_000_000);
    no_data(&mut b);
    assert!(!key(&mut b, 0, true));
    assert!(!key(&mut b, 0x7f, true));
    assert!(!key(&mut b, 0xff, true));
}

#[test]
fn bounded_fifo_retries_without_losing_keys_when_scc_is_full() {
    let mut b = bus();
    boot(&mut b);
    for code in 1..=16 {
        assert!(key(&mut b, code, true));
    }
    assert!(!key(&mut b, 17, true));
    b.advance_ns(1_000_000_000); // SCC retains only three bytes; keyboard keeps the rest.
    for code in 1..=16 {
        assert_eq!(read(&mut b), code);
    }
    assert!(key(&mut b, 17, true));
    assert_eq!(read(&mut b), 17);
    // Failed enqueue did not mark station 17 pressed, or this make would be lost.
    b.advance_ns(100_000_000);
    no_data(&mut b);
}

#[test]
fn reset_reports_held_key_and_led_bell_click_commands_are_distinct() {
    let mut b = bus();
    boot(&mut b);
    assert!(key(&mut b, 0x4d, true));
    assert_eq!(read(&mut b), 0x4d);
    command(&mut b, 1);
    assert_eq!([read(&mut b), read(&mut b), read(&mut b)], [0xff, 4, 0x4d]);
    assert!(key(&mut b, 0x4d, false));
    assert_eq!([read(&mut b), read(&mut b)], [0xcd, 0x7f]);
    for value in [0x0e, 0x1f, 2, 10] {
        command(&mut b, value);
    }
    b.advance_ns(100_000_000);
    let k = b.devices().unwrap().keyboard.as_ref().unwrap();
    assert_eq!(k.leds, 0x1f);
    assert!(k.bell);
    assert!(k.click);
    no_data(&mut b);
    for value in [3, 11, 0x0e, 0] {
        command(&mut b, value);
    }
    b.advance_ns(100_000_000);
    let k = b.devices().unwrap().keyboard.as_ref().unwrap();
    assert_eq!(k.leds, 0);
    assert!(!k.bell);
    assert!(!k.click);
}

#[test]
fn layout_reply_waits_for_fifo_space_instead_of_being_dropped() {
    let mut b = bus();
    boot(&mut b);
    for code in 1..=16 {
        assert!(key(&mut b, code, true));
    }
    command(&mut b, 15);
    for code in 1..=16 {
        assert_eq!(read(&mut b), code);
    }
    assert_eq!([read(&mut b), read(&mut b)], [0xfe, 0x21]);
}
