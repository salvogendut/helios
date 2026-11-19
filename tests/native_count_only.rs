#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};
use std::num::NonZeroUsize;

const VA: u32 = 0x1234_5000;
const PA: u64 = 0x8000;
const NOP: u32 = 0x0100_0000;
const ADD: u32 = 0x8200_6001;

fn mapped() -> Machine {
    let p = MachineProfile::Ss20;
    let mut m = Machine::with_profile(p, 16 << 20, &NOP.to_be_bytes(), p.default_nvram()).unwrap();
    for (a, w) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x088e),
    ] {
        m.bus.write32(a, w).unwrap();
    }
    let words = [
        ADD,
        ADD,
        ADD,
        0x1080_0000 | ((-2i32 as u32) & 0x3f_ffff),
        NOP,
    ];
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
    m.set_native_execution_enabled(true);
    m.set_interpret_span_fallback(true);
    m
}

#[test]
fn count_only_matches_detailed_outcomes_and_state() {
    let mut detailed = mapped();
    let mut counted = mapped();
    for _ in 0..12 {
        let d_calls = detailed.native_execution_calls();
        let c_calls = counted.native_execution_calls();
        let d_native = detailed.native_execution_count();
        let c_native = counted.native_execution_count();
        let d = detailed
            .step_native_span(NonZeroUsize::new(4).unwrap())
            .unwrap();
        let c = counted
            .step_native_span_count(NonZeroUsize::new(4).unwrap())
            .unwrap();
        assert_eq!(d.len(), c.outcomes);
        assert_eq!(d.native_instructions, c.native_instructions);
        assert_eq!(d.get(0), c.first);
        assert_eq!(
            detailed.native_execution_calls() - d_calls,
            counted.native_execution_calls() - c_calls
        );
        assert_eq!(
            detailed.native_execution_count() - d_native,
            counted.native_execution_count() - c_native
        );
        assert!(detailed
            .diagnostic_snapshot()
            .differences(&counted.diagnostic_snapshot())
            .is_empty());
    }
    assert_eq!(detailed.cpu.read_reg(1), counted.cpu.read_reg(1));
    assert_eq!(detailed.cpu.pc(), counted.cpu.pc());
    assert_eq!(detailed.cpu.npc(), counted.cpu.npc());
    assert_eq!(detailed.virtual_ns(), counted.virtual_ns());
    for m in [&mut detailed, &mut counted] {
        m.cpu.set_pc_npc(VA, VA + 4);
        m.bus.write32(PA, 0).unwrap();
    }
    assert_eq!(
        detailed
            .step_native_span(NonZeroUsize::new(2).unwrap())
            .unwrap_err(),
        counted
            .step_native_span_count(NonZeroUsize::new(2).unwrap())
            .unwrap_err()
    );
}
