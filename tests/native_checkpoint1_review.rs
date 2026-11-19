#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::machine::Machine;

const ADD1: u32 = 0x8200_6001;
const ADD9: u32 = 0x8200_6009;

fn bare() -> Machine {
    let mut m = Machine::bare_metal(0x20000, 0).unwrap();
    m.load_image(0, &ADD1.to_be_bytes()).unwrap();
    m
}

fn warm(m: &mut Machine) {
    m.cpu.set_pc_npc(0, 4);
    m.run_jit_native(1).unwrap();
    m.cpu.write_reg(1, 0);
    m.cpu.set_pc_npc(0, 4);
}

#[test]
fn doubleword_store_revalidates_warm_native_code() {
    let mut m = bare();
    warm(&mut m);
    m.bus.write_doubleword(0, [ADD9, 0x0100_0000]).unwrap();
    assert_eq!(m.bus.read32(0).unwrap(), ADD9);
    m.run_jit_native(1).unwrap();
    assert_eq!(m.cpu.read_reg(1), 9);
}

#[test]
fn swap_revalidates_warm_native_code() {
    let mut m = bare();
    warm(&mut m);
    assert_eq!(m.bus.swap32(0, ADD9).unwrap(), ADD1);
    m.run_jit_native(1).unwrap();
    assert_eq!(m.cpu.read_reg(1), 9);
}

#[test]
fn ldstub_revalidates_warm_native_code() {
    let mut m = bare();
    warm(&mut m);
    assert_eq!(m.bus.ldstub8(3).unwrap(), 1);
    assert_eq!(m.bus.read32(0).unwrap(), 0x8200_60ff);
    m.run_jit_native(1).unwrap();
    assert_eq!(m.cpu.read_reg(1), 255);
}

#[test]
fn watch_capacity_rollover_keeps_revalidated_blocks_watched() {
    let mut m = bare();
    warm(&mut m);
    for n in 1..=512u32 {
        let pc = n * 16;
        m.bus.write32(u64::from(pc), ADD1).unwrap();
        m.cpu.set_pc_npc(pc, pc + 4);
        m.run_jit_native(1).unwrap();
        // Keep PC 0 resident in the bounded engine while exhausting watches.
        m.cpu.set_pc_npc(0, 4);
        m.run_jit_native(1).unwrap();
    }
    m.bus.write32(0, ADD9).unwrap();
    m.cpu.write_reg(1, 0);
    m.cpu.set_pc_npc(0, 4);
    m.run_jit_native(1).unwrap();
    assert_eq!(m.cpu.read_reg(1), 9);
}

#[test]
fn empty_run_reports_no_new_cache_activity() {
    let mut m = bare();
    warm(&mut m);
    let stats = m.run_jit_native(0).unwrap();
    assert_eq!(stats.executed, 0);
    assert_eq!(stats.native_instructions, 0);
    assert_eq!(
        stats.cache_hits, 0,
        "run result must report this call's delta"
    );
    assert_eq!(stats.evictions, 0);
}

#[test]
fn ordinary_word_store_control_revalidates() {
    let mut m = bare();
    warm(&mut m);
    m.bus.write32(0, ADD9).unwrap();
    m.run_jit_native(1).unwrap();
    assert_eq!(m.cpu.read_reg(1), 9);
}

#[test]
fn selective_register_masks_preserve_dependencies_and_g0() {
    let words: [u32; 5] = [
        0x8200_6001,
        0x8400_4001,
        0x8000_a007,
        0x8600_0002,
        0x88a0_e001,
    ];
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let mut reference = bare();
    let mut native = bare();
    reference.load_image(0, &bytes).unwrap();
    native.load_image(0, &bytes).unwrap();
    reference.cpu.write_reg(1, 5);
    native.cpu.write_reg(1, 5);
    for _ in 0..5 {
        reference.step().unwrap();
    }
    let stats = native.run_jit_native(5).unwrap();
    assert_eq!(stats.native_instructions, 5);
    assert_eq!(native.cpu.read_reg(0), 0);
    assert_eq!(native.cpu.read_reg(1), 6);
    assert_eq!(native.cpu.read_reg(2), 12);
    assert_eq!(native.cpu.read_reg(3), 12);
    assert_eq!(native.cpu.read_reg(4), 11);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&native.diagnostic_snapshot())
        .is_empty());
}
