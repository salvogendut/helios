use helios::{
    bus::Bus,
    machine::{Machine, MachineProfile},
};

fn machine() -> Machine {
    let profile = MachineProfile::Ss20;
    Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &0u32.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap()
}

#[test]
fn processor_stream_read_of_module_control_never_panics_the_host() {
    let mut m = machine();
    // Either modeled completion or an explicit error is acceptable; host
    // unwinding is not.
    let _ = m
        .cpu
        .mmu
        .store_doubleword(&mut m.bus, 0x01c0_0100, 2, [0xf, 0xf8c0_0a00]);
}

#[test]
fn physical_stream_write_to_module_control_never_panics_the_host() {
    let mut m = machine();
    let _ = m.bus.write_doubleword(0x000f_f8c0_0200, [0xf, 0xf8c0_0a00]);
}

#[test]
fn module_port_identity_ignores_guest_writes() {
    let mut m = machine();
    let before = m
        .cpu
        .mmu
        .load_doubleword(&mut m.bus, 0x01c0_0f00, 2)
        .unwrap();
    m.cpu
        .mmu
        .store_doubleword(&mut m.bus, 0x01c0_0f00, 2, [0xffff_ffff, 0xffff_ffff])
        .unwrap();
    let after = m
        .cpu
        .mmu
        .load_doubleword(&mut m.bus, 0x01c0_0f00, 2)
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(before, [0, 0x0800_0104]);
}

#[test]
fn populated_module_has_the_architected_mbus_port_address() {
    let mut m = machine();
    assert_eq!(m.bus.read32(0x000f_f8ff_fffc).unwrap(), 0x0800_0104);
}
