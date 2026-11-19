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

fn compare_step(d: &mut Machine, c: &mut Machine, budget: usize) -> helios::jit::NativeSpanCount {
    let budget = NonZeroUsize::new(budget).unwrap();
    let detailed = d.step_native_span(budget).unwrap();
    let count = c.step_native_span_count(budget).unwrap();
    assert_eq!(count.first, detailed.get(0), "event at this boundary");
    assert_eq!(count.outcomes, detailed.len());
    assert_eq!(
        count.native_instructions, detailed.native_instructions,
        "reported native retirements"
    );
    assert_eq!(
        c.native_execution_calls(),
        d.native_execution_calls(),
        "actual native calls"
    );
    assert_eq!(c.native_execution_count(), d.native_execution_count());
    assert_eq!(
        c.diagnostic_snapshot()
            .differences(&d.diagnostic_snapshot()),
        Vec::<&str>::new()
    );
    #[cfg(feature = "perf-counters")]
    assert_eq!(
        c.cpu.mmu.translation_counters(),
        d.cpu.mmu.translation_counters()
    );
    #[cfg(feature = "jit-profile")]
    {
        let deterministic = |m: &Machine| {
            let mut stats = m.cpu.jit_phase().counts;
            stats.words.compile_ns = 0;
            stats.spans.compile_ns = 0;
            stats
        };
        assert_eq!(deterministic(c), deterministic(d));
    }
    count
}

#[test]
fn count_reports_native_single_word_with_guards() {
    let (mut d, mut c) = (mapped(), mapped());
    for m in [&mut d, &mut c] {
        m.set_interpret_span_fallback(false);
    }
    assert_eq!(compare_step(&mut d, &mut c, 1).native_instructions, 1);
}

#[test]
fn count_reports_native_single_word_without_guards() {
    let (mut d, mut c) = (mapped(), mapped());
    for m in [&mut d, &mut c] {
        m.set_device_service_guards(false);
        m.set_interpret_span_fallback(false);
    }
    assert_eq!(compare_step(&mut d, &mut c, 16).native_instructions, 1);
}

#[test]
fn count_honors_interpreted_fallback_without_guards() {
    let (mut d, mut c) = (mapped(), mapped());
    for m in [&mut d, &mut c] {
        m.set_device_service_guards(false);
    }
    assert_eq!(compare_step(&mut d, &mut c, 16).native_instructions, 0);
}

#[test]
fn count_attributes_mmio_reset_to_issuing_store() {
    let (mut d, mut c) = (mapped(), mapped());
    let sta = 0xc4a0_45e0; // sta %g2,[%g1] ASI 0x2f
    for m in [&mut d, &mut c] {
        m.bus.write32(PA, sta).unwrap();
        m.cpu.write_reg(1, m.bus.layout().reset as u32);
        m.cpu.write_reg(2, 1);
    }
    assert_eq!(
        compare_step(&mut d, &mut c, 16).first,
        Some(helios::cpu::Step::SoftwareReset {
            pc: VA,
            npc: VA + 4,
            instruction: Some(sta)
        })
    );
}

#[test]
fn count_latches_watchdog_at_first_error_only() {
    let (mut d, mut c) = (mapped(), mapped());
    for m in [&mut d, &mut c] {
        m.bus.write32(PA, 0).unwrap();
    }
    for expected in [0x10, 0] {
        let err = d.step_native_span(NonZeroUsize::MIN).unwrap_err();
        assert_eq!(
            err,
            c.step_native_span_count(NonZeroUsize::MIN).unwrap_err()
        );
        for m in [&mut d, &mut c] {
            let addr = m.bus.layout().reset;
            assert_eq!(m.bus.read32(addr).unwrap() & 0x10, expected);
            m.bus.write32(addr, 0).unwrap();
        }
    }
}

#[test]
fn count_preserves_power_off_fault_and_latches() {
    let (mut d, mut c) = (mapped(), mapped());
    for m in [&mut d, &mut c] {
        m.bus.write32(PA, 0xc4a8_45e0).unwrap(); // stba %g2,[%g1] ASI 0x2f
        m.cpu.write_reg(1, m.bus.layout().auxiliary_power as u32);
        m.cpu.write_reg(2, 1);
    }
    let fault = d.step_native_span(NonZeroUsize::MIN).unwrap_err();
    assert!(fault.is_power_off());
    assert_eq!(
        c.step_native_span_count(NonZeroUsize::MIN).unwrap_err(),
        fault
    );
    assert_eq!(
        c.diagnostic_snapshot()
            .differences(&d.diagnostic_snapshot()),
        Vec::<&str>::new()
    );
}

#[test]
fn count_respects_branch_annulment_and_time_quantum() {
    for input in [0, 1] {
        let (mut d, mut c) = (mapped(), mapped());
        for m in [&mut d, &mut c] {
            for (i, w) in [0x80a0_6001, 0x3280_0003, 0x8400_a001, NOP, NOP]
                .iter()
                .enumerate()
            {
                m.bus.write32(PA + 4 * i as u64, *w).unwrap();
            }
            m.cpu.write_reg(1, input);
        }
        let got = compare_step(&mut d, &mut c, 3);
        assert_eq!(got.outcomes, if input == 1 { 2 } else { 3 });
        assert_eq!(c.cpu.read_reg(2), u32::from(input != 1));
    }
    let (mut d, mut c) = (mapped(), mapped());
    for _ in 0..18 {
        compare_step(&mut d, &mut c, 1);
    }
    for m in [&mut d, &mut c] {
        m.cpu.set_pc_npc(VA, VA + 4);
    }
    assert_eq!(c.bus.deferred_ns(), 1800);
    assert_eq!(compare_step(&mut d, &mut c, 2).outcomes, 1);
}

#[test]
fn count_matches_ram_load_store_and_pending_reset() {
    for store in [false, true] {
        let (mut d, mut c) = (mapped(), mapped());
        for m in [&mut d, &mut c] {
            m.bus.write32(PA, 0x8400_a004).unwrap(); // add %g2,4,%g2
            m.bus
                .write32(PA + 4, if store { 0xc620_8000 } else { 0xc600_8000 })
                .unwrap();
            m.cpu
                .mmu
                .store(&mut m.bus, VA + 0x100, 11, Width::Word, 0x7654)
                .unwrap();
            m.cpu.write_reg(2, VA + 0xfc);
            m.cpu.write_reg(3, 0x4321);
        }
        assert_eq!(compare_step(&mut d, &mut c, 2).native_instructions, 2);
        assert_eq!(
            c.bus.read32(PA + 0x100).unwrap(),
            if store { 0x4321 } else { 0x7654 }
        );
        for m in [&mut d, &mut c] {
            m.bus.write32(m.bus.layout().reset, 1).unwrap();
        }
        assert!(matches!(
            compare_step(&mut d, &mut c, 16).first,
            Some(helios::cpu::Step::SoftwareReset {
                instruction: None,
                ..
            })
        ));
    }
}

#[test]
fn count_matches_single_trap_and_interrupt_entry() {
    use helios::cpu::{Step, PSR_ET, PSR_S};
    for irq in [false, true] {
        let (mut d, mut c) = (mapped(), mapped());
        for m in [&mut d, &mut c] {
            for (i, w) in [0x8188_6000, NOP, NOP, NOP].iter().enumerate() {
                m.bus.write32(PA + 0x40 + 4 * i as u64, *w).unwrap();
            }
            m.cpu.set_pc_npc(VA + 0x40, VA + 0x44);
            m.cpu.write_reg(1, PSR_S | PSR_ET);
            for _ in 0..4 {
                m.cpu.step(&mut m.bus).unwrap();
            }
            m.cpu.set_pc_npc(VA, VA + 4);
            m.bus.write32(PA, 0).unwrap();
            if irq {
                let layout = m.bus.layout();
                m.bus.write32(layout.system_interrupt + 8, 1 << 31).unwrap();
                m.bus.write32(layout.cpu_interrupt + 8, 1 << 28).unwrap();
            }
        }
        let got = compare_step(&mut d, &mut c, 16);
        assert_eq!((got.outcomes, got.native_instructions), (1, 0));
        if irq {
            assert!(matches!(
                got.first,
                Some(Step::InterruptTaken { level: 12, .. })
            ));
        } else {
            assert!(matches!(got.first, Some(Step::TrapTaken { .. })));
        }
    }
}
