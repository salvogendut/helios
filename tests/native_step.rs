#![cfg(feature = "jit-native")]

use helios::cpu::Step;
use helios::machine::{Machine, MachineProfile};

fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

#[test]
fn ordinary_step_executes_supported_instructions_natively() {
    let words = [0x0300_0004, 0x8400_6001, 0x8600_6002];
    let mut reference = Machine::bare_metal(0x10000, 0).unwrap();
    reference.load_image(0, &image(&words)).unwrap();
    let mut native = Machine::bare_metal(0x10000, 0).unwrap();
    native.load_image(0, &image(&words)).unwrap();
    native.set_native_execution_enabled(true);
    for _ in 0..3 {
        assert!(matches!(reference.step().unwrap(), Step::Executed { .. }));
    }
    for _ in 0..3 {
        assert!(matches!(native.step().unwrap(), Step::Executed { .. }));
    }
    assert_eq!(native.native_execution_count(), 3);
    assert_eq!(
        reference
            .diagnostic_snapshot()
            .differences(&native.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

#[test]
fn device_machine_executes_native_bridge_after_fetch() {
    let words = [0x0300_0004, 0x8400_6001];
    let build = || {
        let mut machine = Machine::with_profile(
            MachineProfile::Ss5,
            16 * 1024 * 1024,
            &image(&words),
            MachineProfile::Ss5.default_nvram(),
        )
        .unwrap();
        machine.cpu.set_pc_npc(0, 4);
        machine
    };
    let mut reference = build();
    let mut native = build();
    native.set_native_execution_enabled(true);
    for _ in 0..2 {
        assert!(matches!(reference.step().unwrap(), Step::Executed { .. }));
        assert!(matches!(native.step().unwrap(), Step::Executed { .. }));
    }
    assert!(native.native_execution_count() >= 1);
    assert_eq!(reference.cpu.read_reg(2), native.cpu.read_reg(2));
}
