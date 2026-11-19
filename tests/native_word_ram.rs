#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};
use std::num::NonZeroUsize;

const VA: u32 = 0x1234_5000;
const PA: u64 = 0x8000;
const DATA: u32 = 0x89ab_cdef;
const ADD4: u32 = 0x8400_a004; // add %g2,4,%g2
const LD: u32 = 0xc600_8000; // ld [%g2],%g3
const ST: u32 = 0xc620_8000; // st %g3,[%g2]
const INC1: u32 = 0x8801_2001; // add %g4,1,%g4
const INC7: u32 = 0x8801_2007;
const NOP: u32 = 0x0100_0000;

fn mapped(words: &[u32], native: bool) -> Machine {
    let p = MachineProfile::Ss20;
    let mut m = Machine::with_profile(p, 16 << 20, &NOP.to_be_bytes(), p.default_nvram()).unwrap();
    // Adjacent virtual pages alias the same physical RAM page.
    for (address, value) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
        (0x4018, 0x088e),
    ] {
        m.bus.write32(address, value).unwrap();
    }
    for (i, w) in words.iter().enumerate() {
        m.bus.write32(PA + 4 * i as u64, *w).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.set_pc_npc(VA, VA + 4);
    m.set_native_execution_enabled(native);
    m.set_interpret_span_fallback(true);
    m
}

fn same(a: &Machine, b: &Machine) {
    assert!(a
        .diagnostic_snapshot()
        .differences(&b.diagnostic_snapshot())
        .is_empty());
    #[cfg(feature = "perf-counters")]
    assert_eq!(
        a.cpu.mmu.translation_counters(),
        b.cpu.mmu.translation_counters()
    );
}

fn run(a: &mut Machine, b: &mut Machine, steps: usize, require_pair: bool) {
    let mut remaining = steps;
    while let Some(budget) = NonZeroUsize::new(remaining) {
        let before = b.native_execution_calls();
        let out = b.step_native_span(budget).unwrap();
        if require_pair {
            assert_eq!(
                (
                    out.len(),
                    out.native_instructions,
                    b.native_execution_calls() - before
                ),
                (2, 2, 1),
                "expected one native call for the address calculation and data access"
            );
        }
        assert!(!out.is_empty() && out.len() <= remaining);
        for outcome in out.iter() {
            assert_eq!(outcome, a.step().unwrap());
        }
        same(a, b);
        remaining -= out.len();
    }
}

fn word_load(require_pair: bool) {
    let words = [ADD4, LD];
    let mut a = mapped(&words, false);
    let mut b = mapped(&words, true);
    for m in [&mut a, &mut b] {
        m.bus.write32(PA + 0x200, DATA).unwrap();
        assert_eq!(
            m.cpu
                .mmu
                .load(&mut m.bus, VA + 0x200, 11, Width::Word)
                .unwrap(),
            DATA
        );
        m.cpu.write_reg(2, VA + 0x1fc);
        m.cpu.write_reg(3, 0xface_feed);
    }
    run(&mut a, &mut b, 2, require_pair);
    assert_eq!(b.cpu.read_reg(3), DATA);
}

fn alias_store(require_pair: bool) {
    let words = [ADD4, ST, INC1, NOP];
    let mut a = mapped(&words, false);
    let mut b = mapped(&words, true);
    for m in [&mut a, &mut b] {
        m.cpu
            .mmu
            .store(&mut m.bus, VA + 0x1008, 11, Width::Word, INC1)
            .unwrap();
        m.cpu.set_pc_npc(VA + 8, VA + 12);
    }
    run(&mut a, &mut b, 2, true); // cache the old instruction at the destination
    for m in [&mut a, &mut b] {
        m.cpu.set_pc_npc(VA, VA + 4);
        m.cpu.write_reg(2, VA + 0x1004);
        m.cpu.write_reg(3, INC7);
        m.cpu.write_reg(4, 0);
    }
    run(&mut a, &mut b, 2, require_pair);
    for m in [&mut a, &mut b] {
        assert_eq!(m.bus.read32(PA + 8).unwrap(), INC7);
    }
    run(&mut a, &mut b, 2, true); // same VA as warm-up, new raw word
    assert_eq!(b.cpu.read_reg(4), 7);
}

#[test]
fn current_interpreter_reference_for_load_and_alias_store() {
    word_load(false);
    alias_store(false);
}
#[test]
fn native_warm_word_load_uses_prefix_result() {
    word_load(true);
}
#[test]
fn native_warm_alias_store_replaces_next_cached_word() {
    alias_store(true);
}

#[test]
fn misaligned_load_preserves_the_retired_prefix_and_destination() {
    let words = [ADD4, LD];
    let mut a = mapped(&words, false);
    let mut b = mapped(&words, true);
    for m in [&mut a, &mut b] {
        m.cpu.write_reg(2, VA + 0x1fd);
        m.cpu.write_reg(3, DATA);
    }
    let prefix = b.step_native_span(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix.get(0), Some(a.step().unwrap()));
    assert_eq!(b.cpu.read_reg(2), VA + 0x201);
    let fault = b
        .step_native_span(NonZeroUsize::new(2).unwrap())
        .unwrap_err();
    assert_eq!(fault, a.step().unwrap_err());
    assert_eq!(
        (fault.pc, fault.npc, fault.instruction),
        (VA + 4, VA + 8, Some(LD))
    );
    assert_eq!(b.cpu.read_reg(3), DATA);
    same(&a, &b);
}

#[test]
fn first_store_takes_the_normal_dirty_bit_update_then_warm_store_fuses() {
    let mut a = mapped(&[ADD4, ST], false);
    let mut b = mapped(&[ADD4, ST], true);
    for m in [&mut a, &mut b] {
        m.cpu
            .mmu
            .load(&mut m.bus, VA + 0x1100, 11, Width::Word)
            .unwrap();
        assert_eq!(m.bus.read32(0x4018).unwrap() & 0x60, 0x20);
        m.cpu.write_reg(2, VA + 0x10fc);
        m.cpu.write_reg(3, DATA);
    }
    let out = b.step_native_span(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out.get(0), Some(a.step().unwrap()));
    run(&mut a, &mut b, 1, false);
    for m in [&mut a, &mut b] {
        assert_eq!(m.bus.read32(0x4018).unwrap() & 0x60, 0x60);
        assert_eq!(m.bus.read32(PA + 0x100).unwrap(), DATA);
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    run(&mut a, &mut b, 2, true);
}

#[test]
fn warm_read_only_data_mapping_faults_once_on_store() {
    let mut a = mapped(&[ADD4, ST], false);
    let mut b = mapped(&[ADD4, ST], true);
    for m in [&mut a, &mut b] {
        m.bus.write32(0x4018, 0x0882).unwrap(); // ACC=0, read only
        m.cpu
            .mmu
            .load(&mut m.bus, VA + 0x1100, 11, Width::Word)
            .unwrap();
        m.cpu.write_reg(2, VA + 0x10fc);
        m.cpu.write_reg(3, DATA);
    }
    let out = b.step_native_span(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!((out.len(), out.get(0)), (1, Some(a.step().unwrap())));
    let fault = b
        .step_native_span(NonZeroUsize::new(1).unwrap())
        .unwrap_err();
    assert_eq!(fault, a.step().unwrap_err());
    assert_eq!((fault.pc, fault.instruction), (VA + 4, Some(ST)));
    assert_eq!(b.bus.read32(PA + 0x100).unwrap(), 0);
    same(&a, &b);
}

#[test]
fn emitted_word_access_uses_the_new_source_and_selection_of_byte_order() {
    use helios_jit_native::{compile_word_pair, State};
    let addcc = 0x8680_e001; // addcc %g3,1,%g3
    for little in [false, true] {
        let mut state = State::default();
        state.regs[3] = 0x7fff_ffff;
        let mut word = 0;
        compile_word_pair(addcc, ST, little)
            .unwrap()
            .execute(&mut state, &mut word);
        assert_eq!(word, if little { 0x80 } else { 0x8000_0000 });
        assert_eq!((state.icc, state.regs[3]), (5, 0x8000_0000));
        state.regs[3] = 0;
        compile_word_pair(NOP, LD, little)
            .unwrap()
            .execute(&mut state, &mut word);
        assert_eq!(state.regs[3], 0x8000_0000);
    }
}

#[test]
fn warm_mxcc_port_remains_a_bus_access() {
    let mut a = mapped(&[ADD4, LD], false);
    let mut b = mapped(&[ADD4, LD], true);
    for m in [&mut a, &mut b] {
        // The SS-20 CPU-0 MXCC port is at physical 0xff8fffffc.
        m.bus.write32(0x4018, 0xff8f_ff8e).unwrap();
        assert_eq!(
            m.cpu
                .mmu
                .load(&mut m.bus, VA + 0x1ffc, 11, Width::Word)
                .unwrap(),
            0x0800_0104
        );
        m.cpu.write_reg(2, VA + 0x1ff8);
    }
    let out = b.step_native_span(NonZeroUsize::new(2).unwrap()).unwrap();
    assert_eq!((out.len(), out.get(0)), (1, Some(a.step().unwrap())));
    run(&mut a, &mut b, 1, false);
    assert_eq!(b.cpu.read_reg(3), 0x0800_0104);
}
