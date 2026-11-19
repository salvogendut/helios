#![cfg(feature = "jit-native")]

use helios::cpu::Step;
use helios::machine::{Machine, SchedulerError};

fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn bare(words: &[u32]) -> Machine {
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(0, &image(words)).unwrap();
    machine
}

fn steps(machine: &mut Machine, count: usize) {
    for _ in 0..count {
        assert!(matches!(machine.step().unwrap(), Step::Executed { .. }));
    }
}

#[test]
fn native_sethi_and_alu_match_interpreter() {
    let words = [
        0x0300_0123,
        0x8400_6001,
        0x8600_200f,
        0x88a0_6001,
        0x0100_0000,
    ];
    let mut reference = bare(&words);
    let mut native = bare(&words);
    steps(&mut reference, 5);
    let stats = native.run_jit_native(5).unwrap();
    assert_eq!(stats.executed, 5);
    assert!(stats.native_blocks > 0);
    assert_eq!(reference.cpu.read_reg(4), native.cpu.read_reg(4));
    assert_eq!(reference.cpu.icc(), native.cpu.icc());
    assert_eq!(
        reference
            .diagnostic_snapshot()
            .differences(&native.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

const BRANCH_PROGRAM: [u32; 5] = [
    0x1080_0004, // ba 0x10
    0x8200_6001, // delay add %g1, 1
    0x8400_a063, // skipped add %g2, 99
    0x0100_0000,
    0x8600_e007, // target add %g3, 7
];

#[test]
fn split_native_calls_preserve_branch_delay_target() {
    let mut reference = bare(&BRANCH_PROGRAM);
    let mut native = bare(&BRANCH_PROGRAM);
    steps(&mut reference, 3);
    assert_eq!(native.run_jit_native(1).unwrap().executed, 1);
    assert_eq!((native.cpu.pc(), native.cpu.npc()), (4, 16));
    assert_eq!(native.run_jit_native(2).unwrap().executed, 2);
    assert_eq!(native.cpu.read_reg(1), 1);
    assert_eq!(native.cpu.read_reg(2), 0);
    assert_eq!(native.cpu.read_reg(3), 7);
    assert_eq!(
        reference
            .diagnostic_snapshot()
            .differences(&native.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

#[test]
fn latched_native_fault_is_returned() {
    let mut reference = bare(&[0]);
    let mut native = bare(&[0]);
    assert_eq!(reference.step().unwrap_err(), native.step().unwrap_err());
    let expected = reference.step().unwrap_err();
    let actual = native.run_jit_native(1);
    assert!(
        matches!(actual, Err(SchedulerError::Cpu { processor: 0, fault }) if fault == expected)
    );
}

#[test]
fn native_reset_is_serviced_and_reported() {
    let build = || {
        let mut machine = Machine::sun4m(0x10000, &image(&[0x0100_0000])).unwrap();
        machine.bus.devices_mut().unwrap().reset.write(1);
        machine
    };
    let mut reference = build();
    let mut native = build();
    let expected = reference.step().unwrap();
    let stats = native.run_jit_native(1).unwrap();
    assert_eq!(stats.executed, 0);
    assert_eq!(stats.stopped, Some(expected));
    assert!(!native.bus.devices().unwrap().reset.requested());
    assert_eq!(
        reference
            .diagnostic_snapshot()
            .differences(&native.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

fn sethi(rd: u32, value: u32) -> u32 {
    0x0100_0000 | (rd << 25) | ((value >> 10) & 0x3f_ffff)
}

fn or_imm(rd: u32, rs1: u32, value: u32) -> u32 {
    0x8000_0000 | (rd << 25) | (2 << 19) | (rs1 << 14) | 0x2000 | (value & 0x1fff)
}

#[test]
fn warm_cached_target_is_invalidated_by_guest_store() {
    let words = [
        0x8600_2001, // target: add %g3, 1, %g3
        0x1080_0003, // ba setup (PC 0x10)
        0x0100_0000, // delay
        0x0100_0000,
        0x0300_0000, // setup: sethi %g1, 0
        sethi(2, 0x8800_2001),
        or_imm(2, 2, 1),                            // %g2 = add %g4, 1, %g4
        0xc420_6000,                                // st %g2, [%g1]
        0x1080_0000 | ((-8i32 as u32) & 0x3f_ffff), // ba target
        0x0100_0000,                                // delay
    ];
    let mut reference = bare(&words);
    let mut native = bare(&words);
    steps(&mut reference, 10);
    let stats = native.run_jit_native(10).unwrap();
    assert_eq!(stats.executed, 10);
    assert_eq!(native.cpu.read_reg(3), 1);
    assert_eq!(
        native.cpu.read_reg(4),
        1,
        "new target must run after warm cache"
    );
    assert_eq!(
        reference
            .diagnostic_snapshot()
            .differences(&native.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}
#[test]
fn device_machine_stays_on_interpreter() {
    use helios::machine::MachineProfile;
    let words = [0x0300_0003, 0x8400_6001];
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
    steps(&mut reference, 2);
    let stats = native.run_jit_native(2).unwrap();
    assert_eq!(stats.executed, 2);
    assert_eq!(stats.native_blocks, 0);
    assert_eq!(reference.cpu.read_reg(2), native.cpu.read_reg(2));
}
