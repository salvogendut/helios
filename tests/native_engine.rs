#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::machine::Machine;

fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn bare(words: &[u32]) -> Machine {
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(0, &image(words)).unwrap();
    machine
}

#[test]
fn persistent_cache_reuses_blocks_and_counts_executed_instructions() {
    let mut machine = bare(&[0x8200_6001, 0x8400_a002]);
    let first = machine.run_jit_native(2).unwrap();
    assert_eq!(first.executed, 2);
    assert_eq!(first.native_instructions, 2);
    assert_eq!(first.fallback_steps, 0);
    assert_eq!(first.compiled_instructions, 2);
    assert!(first.compile_calls > 0);
    machine.cpu.set_pc_npc(0, 4);
    let second = machine.run_jit_native(2).unwrap();
    assert_eq!(second.executed, 2);
    assert_eq!(second.native_instructions, 2);
    assert_eq!(second.compile_calls, 0);
    assert!(second.cache_hits > 0);
    assert_eq!(machine.cpu.read_reg(1), 2);
    assert_eq!(machine.cpu.read_reg(2), 4);
}

#[test]
fn ordinary_data_writes_do_not_evict_native_code() {
    let mut machine = bare(&[0x8200_6001]);
    let _first = machine.run_jit_native(1).unwrap();
    machine.bus.write32(0x800, 0x1234_5678).unwrap();
    machine.cpu.set_pc_npc(0, 4);
    let second = machine.run_jit_native(1).unwrap();
    assert_eq!(second.compile_calls, 0);
    assert!(second.cache_hits > 0);
}

#[test]
fn code_writes_invalidate_and_recompile() {
    let mut machine = bare(&[0x8200_6001]); // add 1
    machine.run_jit_native(1).unwrap();
    machine.bus.write32(0, 0x8200_6009).unwrap(); // add 9
    machine.cpu.write_reg(1, 0);
    machine.cpu.set_pc_npc(0, 4);
    let second = machine.run_jit_native(1).unwrap();
    assert!(second.compile_calls > 0);
    assert_eq!(machine.cpu.read_reg(1), 9);
}
