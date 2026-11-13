use helios::bus::Bus;
use helios::machine::{Machine, MachineProfile};

const SCC: u64 = 0xf_f100_0000;

fn machine() -> Machine {
    let profile = MachineProfile::Ss20;
    let mut machine =
        Machine::with_profile(profile, 16 * 1024 * 1024, &[0; 4], profile.default_nvram()).unwrap();
    // B channel: asynchronous 8-bit reception and receive interrupts.
    for (register, value) in [(4, 0x44), (14, 3), (3, 0xc1), (1, 0x10), (9, 8)] {
        machine.bus.write8(SCC, register).unwrap();
        machine.bus.write8(SCC, value).unwrap();
    }
    machine
}

#[test]
fn mouse_does_not_deliver_a_1200_baud_character_after_one_nanosecond() {
    let mut machine = machine();
    machine.mouse_motion(20, 0);
    machine.advance_ns(1);
    assert_eq!(
        machine.bus.read8(SCC).unwrap() & 1,
        0,
        "no complete byte yet"
    );
    assert!(!machine.bus.devices().unwrap().keyboard_serial.irq());
}

#[test]
fn negative_motion_never_uses_sun_packet_header_bytes_as_delta_data() {
    let mut machine = machine();
    machine.mouse_motion(-450, -340);
    let mut bytes = Vec::new();
    for _ in 0..300 {
        machine.advance_ns(1_000_000);
        while machine.bus.read8(SCC).unwrap() & 1 != 0 {
            bytes.push(machine.bus.read8(SCC + 2).unwrap());
        }
    }
    assert!(!bytes.is_empty());
    assert_eq!(bytes.len() % 5, 0);
    let mut dx = 0i32;
    let mut dy = 0i32;
    for packet in bytes.chunks_exact(5) {
        assert_eq!(packet[0], 0x87);
        for &byte in &packet[1..] {
            assert!(
                !(0x80..=0x87).contains(&byte),
                "reserved header byte {byte:#04x} in movement packet {packet:02x?}"
            );
        }
        dx += i32::from(packet[1] as i8) + i32::from(packet[3] as i8);
        dy += i32::from(packet[2] as i8) + i32::from(packet[4] as i8);
    }
    assert_eq!((dx, dy), (-450, -340), "splitting must retain all movement");
}
