use helios::bus::{Bus, MemoryBus};
use helios::cpu::{Cpu, Step, PSR_EF, PSR_ET, PSR_PIL, PSR_PS, PSR_S};
use helios::devices::interrupt::{InterruptController, Source, MASK_ALL, MASK_BITS};

const NOP: u32 = 0x0100_0000;

fn fixture(psr: u32) -> (Cpu, MemoryBus) {
    let mut cpu = Cpu::new(0);
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x2000).unwrap();
    // wr g1, 0, tbr; wr g2, 0, psr; three portable spacing NOPs.
    for (i, word) in [0x8198_6000, 0x8188_a000, NOP, NOP, NOP, NOP, NOP]
        .into_iter()
        .enumerate()
    {
        bus.write32(i as u64 * 4, word).unwrap();
    }
    cpu.write_reg(1, 0x1000);
    cpu.write_reg(2, psr);
    for _ in 0..5 {
        cpu.step(&mut bus).unwrap();
    }
    (cpu, bus)
}

#[test]
fn all_interrupt_levels_obey_et_pil_and_level_15_bypass() {
    for enabled in [false, true] {
        for pil in 0..16 {
            for level in 0..16 {
                let (mut cpu, mut bus) =
                    fixture(PSR_S | (pil << 8) | if enabled { PSR_ET } else { 0 });
                let step = cpu.step_with_interrupts(&mut bus, 1 << level).unwrap();
                let expected = enabled && level != 0 && (level == 15 || level > pil);
                assert_eq!(
                    matches!(step, Step::InterruptTaken { .. }),
                    expected,
                    "ET={enabled}, PIL={pil}, level={level}"
                );
                assert!(cpu.error_state().is_none());
                if expected {
                    assert_eq!(cpu.pc(), 0x1100 | level << 4);
                    assert_eq!(cpu.psr() & PSR_PIL, pil << 8);
                } else {
                    assert_eq!(cpu.pc(), 24);
                }
            }
        }
    }
}

#[test]
fn highest_interrupt_saves_both_pcs_and_rotates_without_fetching_or_checking_wim() {
    let (mut cpu, _) = fixture(PSR_ET | PSR_EF | 0x00b0_0000 | (8 << 8)); // user
    cpu.set_window_invalid_mask(0xff);
    let mut unmapped = MemoryBus::default();
    let step = cpu
        .step_with_interrupts(&mut unmapped, (1 << 4) | (1 << 12) | (1 << 10))
        .unwrap();
    assert_eq!(
        step,
        Step::InterruptTaken {
            pc: 20,
            npc: 24,
            level: 12,
            vector: 0x11c0
        }
    );
    assert_eq!(cpu.current_window(), 7);
    assert_eq!((cpu.read_reg(17), cpu.read_reg(18)), (20, 24));
    assert_eq!(cpu.psr() & (PSR_ET | PSR_S | PSR_PS), PSR_S);
    assert_eq!(
        cpu.psr() & (PSR_EF | PSR_PIL | 0x00f0_0000),
        PSR_EF | (8 << 8) | 0x00b0_0000
    );
    assert_eq!(cpu.mmu.fault_status(), 0);
}

#[test]
fn interrupt_in_branch_delay_slot_returns_to_the_original_branch_target() {
    let (mut cpu, mut bus) = fixture(PSR_S | PSR_ET);
    bus.write32(20, 0x1080_000b).unwrap(); // ba 64
    bus.write32(24, 0x8600_e001).unwrap(); // add g3, 1, g3 (delay)
    bus.write32(64, 0x8801_2001).unwrap(); // add g4, 1, g4 (target)
    bus.write32(0x11a0, 0x81c4_6000).unwrap(); // jmpl l1, g0
    bus.write32(0x11a4, 0x81cc_a000).unwrap(); // rett l2
    cpu.step(&mut bus).unwrap();
    let step = cpu.step_with_interrupts(&mut bus, 1 << 10).unwrap();
    assert_eq!(
        step,
        Step::InterruptTaken {
            pc: 24,
            npc: 64,
            level: 10,
            vector: 0x11a0
        }
    );
    assert_eq!(cpu.read_reg(3), 0);
    cpu.step(&mut bus).unwrap();
    cpu.step(&mut bus).unwrap();
    assert_eq!((cpu.pc(), cpu.npc(), cpu.current_window()), (24, 64, 0));
    assert_eq!(cpu.psr() & (PSR_ET | PSR_S), PSR_ET | PSR_S);
    cpu.step(&mut bus).unwrap();
    cpu.step(&mut bus).unwrap();
    assert_eq!((cpu.read_reg(3), cpu.read_reg(4), cpu.pc()), (1, 1, 68));
}

#[test]
fn a_held_line_retriggers_after_rett_and_cannot_clear_cpu_error_state() {
    let (mut cpu, mut bus) = fixture(PSR_S | PSR_ET);
    bus.write32(0x11f0, 0x81c4_6000).unwrap();
    bus.write32(0x11f4, 0x81cc_a000).unwrap();
    cpu.step_with_interrupts(&mut bus, 1 << 15).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            cpu.step_with_interrupts(&mut bus, 1 << 15).unwrap(),
            Step::Executed { .. }
        ));
    }
    assert!(matches!(
        cpu.step_with_interrupts(&mut bus, 1 << 15).unwrap(),
        Step::InterruptTaken {
            pc: 20,
            npc: 24,
            ..
        }
    ));

    let (mut cpu, mut bus) = fixture(PSR_S);
    bus.write32(20, 0).unwrap(); // UNIMP with ET=0
    let fault = cpu.step_with_interrupts(&mut bus, 1 << 15).unwrap_err();
    let before = cpu.clone();
    assert_eq!(
        cpu.step_with_interrupts(&mut bus, u16::MAX).unwrap_err(),
        fault
    );
    assert_eq!(cpu, before);
}

#[test]
fn controller_masks_gate_delivery_without_erasing_pending_requests() {
    let mut ic = InterruptController::default();
    assert_eq!(ic.read(true, 4).unwrap(), MASK_BITS);
    ic.set_source(Source::SystemTimer, true);
    ic.set_cpu_timer(true);
    ic.write(false, 8, 1 << 31).unwrap(); // soft 15
    assert_eq!(ic.read(true, 0).unwrap(), 1 << 19);
    assert_eq!(
        ic.read(false, 0).unwrap(),
        (1 << 31) | (1 << 14) | (1 << 10)
    );
    assert_eq!(ic.levels(), 0);
    ic.write(true, 8, MASK_ALL).unwrap();
    assert_eq!(ic.levels(), (1 << 15) | (1 << 14));
    ic.write(true, 8, 1 << 19).unwrap();
    assert_eq!(ic.levels(), (1 << 15) | (1 << 14) | (1 << 10));
    ic.write(true, 12, 1 << 19).unwrap();
    assert_eq!(ic.read(true, 0).unwrap(), 1 << 19);
    ic.write(false, 4, u32::MAX).unwrap(); // clears soft, not timer lines
    assert_eq!(ic.read(false, 0).unwrap(), (1 << 14) | (1 << 10));
    assert_eq!(ic.levels(), 1 << 14);
    ic.set_cpu_timer(false);
    assert_eq!(ic.levels(), 0);
}

#[test]
fn source_routes_and_shared_levels_remain_independent() {
    let routes = [
        (Source::SBus1, 2),
        (Source::SBus2, 3),
        (Source::SBus3, 5),
        (Source::SBus4, 7),
        (Source::SBus5, 9),
        (Source::SBus6, 11),
        (Source::SBus7, 13),
        (Source::Keyboard, 12),
        (Source::Serial, 12),
        (Source::Ethernet, 6),
        (Source::Scsi, 4),
        (Source::SystemTimer, 10),
        (Source::Floppy, 11),
    ];
    for (source, level) in routes {
        let mut ic = InterruptController::default();
        ic.write(true, 8, u32::MAX).unwrap();
        ic.set_source(source, true);
        assert_eq!(ic.levels(), 1 << level);
        assert_eq!(ic.read(true, 0).unwrap(), 1 << source as u8);
    }
    let mut ic = InterruptController::default();
    ic.write(true, 8, u32::MAX).unwrap();
    ic.set_source(Source::Keyboard, true);
    ic.set_source(Source::Serial, true);
    ic.set_source(Source::Keyboard, false);
    assert_eq!(ic.levels(), 1 << 12);
    ic.write(true, 12, 1 << 15).unwrap();
    assert_eq!(ic.levels(), 0);
    ic.write(true, 8, 1 << 15).unwrap();
    assert_eq!(ic.levels(), 1 << 12);
    ic.set_source(Source::Serial, false);
    assert_eq!(ic.levels(), 0);
}

#[test]
fn software_set_clear_reserved_bits_and_reset() {
    let mut ic = InterruptController::default();
    ic.write(true, 8, u32::MAX).unwrap();
    ic.write(false, 8, u32::MAX).unwrap();
    assert_eq!(ic.read(false, 0).unwrap(), 0xfffe_0000);
    assert_eq!(ic.levels(), 0xfffe);
    ic.write(false, 4, 1 << 31 | 1 << 17).unwrap();
    assert_eq!(ic.levels(), 0x7ffc);
    ic.write(true, 12, !MASK_BITS).unwrap();
    assert_eq!(ic.read(true, 4).unwrap(), 0);
    ic.write(true, 16, 15).unwrap(); // reserved MP target: no other CPU exists
    assert_eq!(ic.read(true, 16).unwrap(), 0);
    ic.reset();
    assert_eq!(ic, InterruptController::default());
}
