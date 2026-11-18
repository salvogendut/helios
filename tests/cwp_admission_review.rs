// Run beside the submitted parking_windows.rs in an isolated source copy.
include!("parking_windows.rs");

use helios::bus::MemoryBus;
use helios::cpu::Cpu;
use helios::machine::solaris_idle_boundary;
use helios::mmu::Width;

// Execute WRPSR on a private bus and restore only its setup effects. This
// changes CWP without touching the live bus, epochs, time, MMU or registers.
fn only_cwp(cpu: &mut Cpu, cwp: u32) {
    let saved_mmu = cpu.mmu.clone();
    let saved_g1 = cpu.read_reg(1);
    let (pc, npc) = (cpu.pc(), cpu.npc());
    let mut scratch = MemoryBus::default();
    scratch.map_ram(0, 0x4000).unwrap();
    scratch.write32(CODE_ADDR, WRPSR).unwrap();
    cpu.write_reg(1, (cpu.psr() & !31) | cwp);
    cpu.set_pc_npc(CODE_VADDR, CODE_VADDR + 4);
    assert!(matches!(
        cpu.step(&mut scratch).unwrap(),
        Step::Executed {
            instruction: WRPSR,
            ..
        }
    ));
    cpu.mmu = saved_mmu;
    cpu.write_reg(1, saved_g1);
    cpu.set_pc_npc(pc, npc);
    assert_eq!(cpu.current_window(), cwp as usize);
}

fn steady(cwp: u32) -> Machine {
    let mut m = seeded(cwp);
    m.cpu.set_window_invalid_mask(1 << ((cwp + 1) % 8));
    assert!(replay(&mut m));
    m
}

#[test]
fn review_predicate_preserves_every_other_bit_and_pc_npc() {
    for cwp in 0..32 {
        let psr = PSR_BASE | cwp;
        assert_eq!(solaris_idle_psr_matches(psr), cwp < 8);
        assert_eq!(
            solaris_idle_boundary(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC, psr),
            cwp < 8
        );
        for bit in 5..32 {
            assert!(
                !solaris_idle_psr_matches(psr ^ (1 << bit)),
                "CWP {cwp}, bit {bit}"
            );
        }
        assert!(!solaris_idle_boundary(
            SOLARIS_IDLE_PC + 4,
            SOLARIS_IDLE_NPC,
            psr
        ));
        assert!(!solaris_idle_boundary(
            SOLARIS_IDLE_PC,
            SOLARIS_IDLE_NPC + 4,
            psr
        ));
    }
}

#[test]
fn review_all_windows_admit_and_advance_without_cpu_or_ram_change() {
    for cwp in 0..8 {
        let mut m = steady(cwp);
        let before = m.diagnostic_snapshot();
        let token = m.parking_token().unwrap();
        assert!(m.validate_parking_token(&token));
        assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
        assert_eq!(m.advance_parked(token, 1_000).unwrap().advanced_ns, 1_000);
        let after = m.diagnostic_snapshot();
        assert_eq!(before.cpu, after.cpu);
        assert_eq!(before.bus.memory_digest, after.bus.memory_digest);
        assert_eq!(before.bus.dram_digest, after.bus.dram_digest);
    }
}

#[test]
fn review_eligible_cwp_change_stales_by_cpu_equality_alone() {
    let mut m = steady(0);
    // Windows 0 and 2 do not overlap each other's out/in aliases.
    only_cwp(&mut m.cpu, 2);
    for (reg, value) in [
        (8, 0xf025f800),
        (24, 0xf026de48),
        (15, 0xf006fa58),
        (16, 0xf0071708),
    ] {
        m.cpu.write_reg(reg, value);
    }
    only_cwp(&mut m.cpu, 0);
    let before = m.diagnostic_snapshot();
    let token = m.parking_token().unwrap();
    only_cwp(&mut m.cpu, 2);
    let after = m.diagnostic_snapshot();
    assert_eq!(before.bus, after.bus);
    assert_eq!(before.differences(&after), vec!["cpu0"]);
    // Both endpoints are eligible, including the full mapped proof.
    assert!(m.parking_token().is_ok());
    #[cfg(feature = "parking-profile")]
    helios::parking_profile::reset();
    assert!(!m.validate_parking_token(&token));
    #[cfg(feature = "parking-profile")]
    {
        let p = helios::parking_profile::snapshot();
        assert_eq!(p.snapshot_eq_failed, 1, "must reach exact CPU comparison");
        assert_eq!(p.digest_calls, 1);
    }
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);
    only_cwp(&mut m.cpu, 0);
    assert_eq!(m.cpu, before.cpu, "helper must change only CWP");
}

#[test]
fn review_truly_inactive_window_and_wim_stale_without_bus_changes() {
    let mut m = steady(0);
    let token = m.parking_token().unwrap();
    let before = m.diagnostic_snapshot();
    let visible: Vec<_> = (0..32).map(|reg| m.cpu.read_reg(reg)).collect();
    only_cwp(&mut m.cpu, 2);
    m.cpu.write_reg(16, 0x1234_5678); // window 2 local, invisible at CWP 0
    only_cwp(&mut m.cpu, 0);
    assert_eq!(
        visible,
        (0..32).map(|reg| m.cpu.read_reg(reg)).collect::<Vec<_>>()
    );
    assert_eq!(before.bus, m.diagnostic_snapshot().bus);
    assert!(m.parking_token().is_ok());
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);

    let token = m.parking_token().unwrap();
    let before = m.diagnostic_snapshot();
    m.cpu
        .set_window_invalid_mask(m.cpu.window_invalid_mask() ^ 1);
    assert_eq!(before.bus, m.diagnostic_snapshot().bus);
    assert!(m.parking_token().is_ok());
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);
}

#[test]
fn review_cwp7_operand_and_code_failures_reject_actual_admission() {
    for (reg, value) in [(8, 0), (24, 0), (15, 0xf006fa44), (16, 0xf025f800)] {
        let mut m = steady(7);
        m.cpu.write_reg(reg, value);
        assert_eq!(
            m.parking_token().unwrap_err(),
            ParkingError::NotIdleBoundary
        );
    }
    let mut m = steady(7);
    m.cpu.write_reg(8, 0);
    // Correct operands in another window must not satisfy the mapped proof.
    only_cwp(&mut m.cpu, 3);
    for (reg, value) in [
        (8, 0xf025f800),
        (24, 0xf026de48),
        (15, 0xf006fa58),
        (16, 0xf0071708),
    ] {
        m.cpu.write_reg(reg, value);
    }
    only_cwp(&mut m.cpu, 7);
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    for (pa, value) in [
        (0x071708, 0x01000000),
        (0x25f8ec, 0),
        (0x26dea0, 1),
        (0x25f9e8, 0xf0071800),
    ] {
        let mut m = steady(7);
        m.bus.write32(pa, value).unwrap();
        assert_eq!(
            m.parking_token().unwrap_err(),
            ParkingError::NotIdleBoundary
        );
    }
}

#[test]
fn review_cwp7_execute_permission_and_boot_override_reject() {
    let mut m = steady(7);
    for (offset, value) in [(0, 0xf0000000), (0x100, 0), (0x200, 0xa1)] {
        m.cpu
            .mmu
            .store(&mut m.bus, offset, 5, Width::Word, value)
            .unwrap();
    }
    let before = m.diagnostic_snapshot();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());

    let mut m = steady(7);
    let boot = m.cpu.mmu.config().boot_mode_bit;
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, helios::mmu::PCR_EN | boot)
        .unwrap();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
}

#[test]
fn review_cwp7_pending_irq_and_coincident_deadlines_keep_service_boundary() {
    let mut m = steady(7);
    let layout = m.bus.layout();
    m.bus
        .write32(layout.system_interrupt + 8, u32::MAX)
        .unwrap();
    m.bus.write32(layout.cpu_interrupt + 8, 1 << 28).unwrap();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::ServiceRequired
    );

    let mut m = steady(7);
    m.bus.write32(m.bus.layout().system_timer, 4 << 9).unwrap();
    m.bus.write32(m.bus.layout().cpu_timer, 4 << 9).unwrap();
    let token = m.parking_token().unwrap();
    let before = m.diagnostic_snapshot();
    let advanced = m.advance_parked(token, 10_000).unwrap();
    assert_eq!(advanced.advanced_ns, 1_500);
    assert!(advanced.clipped && advanced.service_required);
    assert!(m.bus.devices().unwrap().timers.system_irq());
    assert!(m.bus.devices().unwrap().timers.processor_irq());
    assert_eq!(m.cpu, before.cpu);
}

#[cfg(feature = "parking-profile")]
#[test]
fn review_non_cwp_psr_rejects_before_mapped_proof_or_digest() {
    let mut m = steady(7);
    m.cpu.write_reg(1, PSR_BASE | 7 | (1 << 8));
    m.cpu.set_pc_npc(CODE_VADDR, CODE_VADDR + 4);
    m.cpu.step(&mut m.bus).unwrap();
    m.cpu.set_pc_npc(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC);
    helios::parking_profile::reset();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    let p = helios::parking_profile::snapshot();
    assert_eq!(p.token_rejected, 1);
    assert_eq!(
        p.proof_ok + p.proof_failed + p.digest_calls + p.snapshot_calls,
        0
    );
}
