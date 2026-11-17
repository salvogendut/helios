mod support;

use helios::bus::Bus;
use helios::cpu::Step;
use helios::devices::{
    cgsix::Cgsix,
    sx::{Sx, SX_BASE, VSIMM_VRAM_BASE},
};
use helios::machine::{ParkingError, SOLARIS_IDLE_PC};
use helios::mmu::{Width, PCR_EN};
use support::{idle_machine, iteration};

#[test]
fn boot_mode_fetch_cannot_be_proved_by_a_retained_ram_tlb_entry() {
    let mut m = idle_machine();
    let boot_bit = m.cpu.mmu.config().boot_mode_bit;
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN | boot_bit)
        .unwrap();
    assert!(m.cpu.mmu.boot_mode());
    let admitted = m.parking_token().is_ok();
    let reference = m.cpu.step(&mut m.bus);
    assert!(
        !matches!(
            reference,
            Ok(Step::Executed {
                instruction: 0xd00220ec,
                ..
            })
        ),
        "reference must fetch PROM instead of the captured RAM instruction"
    );
    assert!(
        !admitted,
        "RAM proof admitted boot-mode fetch; normal step={reference:?}"
    );
}

#[test]
fn instruction_subregion_override_must_agree_with_normal_translation() {
    let mut m = idle_machine();
    // Evict only the direct fast-cache key, keeping the architectural L1 TLB.
    m.cpu
        .mmu
        .load(&mut m.bus, SOLARIS_IDLE_PC + 0x10_0000, 9, Width::Word)
        .unwrap();
    // Reference I-TLB PTP2: f0040000 region, table at physical 4000.
    m.cpu
        .mmu
        .store(&mut m.bus, 0x500, 5, Width::Word, 0x401)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0x600, 5, Width::Word, 0xf0040000)
        .unwrap();
    m.bus.write32(0x40bc, 0x000400ee).unwrap(); // PTE: PA 00400000, 256 KiB
    m.bus.write32(0x42fa24, 0x82006001).unwrap(); // actual next instruction: add g1,1,g1
    let admitted = m.parking_token().is_ok();
    let reference = m.cpu.step(&mut m.bus).unwrap();
    assert!(
        matches!(
            reference,
            Step::Executed {
                instruction: 0x82006001,
                ..
            }
        ),
        "expected reference PTP2 override, got {reference:?}"
    );
    assert!(
        !admitted,
        "proof used the base TLB while normal translation selects active subregion code"
    );
}

#[test]
fn data_subregion_override_must_agree_with_normal_translation() {
    let mut m = idle_machine();
    m.cpu
        .mmu
        .load(&mut m.bus, 0xf035f8ec, 11, Width::Word)
        .unwrap();
    // Reference D-TLB PTP2 overrides the f0240000 data region.
    m.cpu
        .mmu
        .store(&mut m.bus, 0x500, 6, Width::Word, 0x401)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0x600, 6, Width::Word, 0xf0240000)
        .unwrap();
    m.bus.write32(0x407c, 0x000400ee).unwrap();
    m.bus.write32(0x41f8ec, 0).unwrap(); // normal first load exits the idle path
    let admitted = m.parking_token().is_ok();
    assert!(matches!(
        m.cpu.step(&mut m.bus).unwrap(),
        Step::Executed {
            instruction: 0xd00220ec,
            ..
        }
    ));
    assert_eq!(m.cpu.read_reg(8), 0);
    assert!(
        !admitted,
        "proof read 1 through the base TLB while actual data access reads 0"
    );
}

fn overwritten_register(reg: u8, expected: u32) {
    let mut m = idle_machine();
    m.cpu.write_reg(reg, 0xdeadbeef);
    let admitted = m.parking_token().is_ok();
    iteration(&mut m);
    assert_eq!(m.cpu.read_reg(reg), expected);
    assert!(
        !admitted,
        "R{reg} changes from deadbeef to {expected:08x} in the first iteration"
    );
}

#[test]
fn nonsteady_o7_link_register_requires_a_normal_iteration() {
    overwritten_register(15, 0xf006fa58);
}

#[test]
fn nonsteady_l0_helper_register_requires_a_normal_iteration() {
    overwritten_register(16, 0xf0071708);
}

#[test]
fn empty_diagnostic_field_requires_the_normal_mmu_update() {
    let mut m = idle_machine();
    // Clear the data-region tag view without removing the actual L1 mapping.
    m.cpu
        .mmu
        .store(&mut m.bus, 0x9000, 6, Width::Word, 0)
        .unwrap();
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x9000, 6, Width::Word).unwrap(),
        0
    );
    let admitted = m.parking_token().is_ok();
    iteration(&mut m);
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x9000, 6, Width::Word).unwrap(),
        0xf0240000
    );
    assert!(
        !admitted,
        "a normal loop fills guest-visible MMU state that parking leaves empty"
    );
}

#[test]
fn removing_execute_permission_rejects_without_mutating_machine() {
    let mut m = idle_machine();
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
    assert!(matches!(
        m.cpu.step(&mut m.bus).unwrap(),
        Step::TrapTaken { .. }
    ));
}

#[test]
fn removing_data_read_permission_rejects_without_mutating_machine() {
    let mut m = idle_machine();
    for (offset, value) in [(0, 0xf0000000), (0x100, 0), (0x200, 0xb1)] {
        m.cpu
            .mmu
            .store(&mut m.bus, offset, 6, Width::Word, value)
            .unwrap();
    }
    let before = m.diagnostic_snapshot();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
    assert!(matches!(
        m.cpu.step(&mut m.bus).unwrap(),
        Step::TrapTaken { .. }
    ));
}

#[test]
fn adding_sx_invalidates_token_even_when_new_generation_is_zero() {
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.bus.install_sx(Sx::new()).unwrap();
    assert!(
        !m.validate_parking_token(&token),
        "None and new SX generation 0 must not compare equal"
    );
}

#[test]
fn adding_cgsix_invalidates_token_even_when_new_generations_are_zero() {
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.install_cgsix(Cgsix::new(&[0; 32768]).unwrap()).unwrap();
    assert!(
        !m.validate_parking_token(&token),
        "None and new CGsix generation 0 must not compare equal"
    );
}

#[test]
fn removing_sx_invalidates_a_token_issued_with_the_card() {
    let mut m = idle_machine();
    m.bus.install_sx(Sx::new()).unwrap();
    let token = m.parking_token().unwrap();
    assert!(m.remove_sx().is_some());
    assert!(!m.validate_parking_token(&token));
}

#[test]
fn replacement_sx_with_same_generation_and_different_vram_stales_token() {
    let mut m = idle_machine();
    let mut original = Sx::new();
    original
        .write(VSIMM_VRAM_BASE, 0x11223344, u32::MAX)
        .unwrap();
    let mut replacement = Sx::new();
    replacement
        .write(VSIMM_VRAM_BASE, 0x55667788, u32::MAX)
        .unwrap();
    assert_eq!(original.generation(), replacement.generation());
    m.bus.install_sx(original).unwrap();
    let token = m.parking_token().unwrap();
    m.remove_sx().unwrap();
    m.bus.install_sx(replacement).unwrap();
    assert!(
        !m.validate_parking_token(&token),
        "per-card generation cannot identify replacement"
    );
}

#[test]
fn video_bus_writes_including_nonvisual_registers_stale_tokens() {
    for address in [VSIMM_VRAM_BASE, SX_BASE + 0x110] {
        let mut m = idle_machine();
        m.bus.install_sx(Sx::new()).unwrap();
        let token = m.parking_token().unwrap();
        m.bus.write32(address, 0x11223344).unwrap();
        assert!(!m.validate_parking_token(&token));
        assert_eq!(
            m.advance_parked(token, 1000).unwrap_err(),
            ParkingError::Stale
        );
    }
}

#[test]
fn timer_before_exact_and_past_boundary_reports_required_service() {
    for requested in [1499, 1500, 1501] {
        let mut m = idle_machine();
        m.bus.write32(m.bus.layout().system_timer, 4 << 9).unwrap();
        let token = m.parking_token().unwrap();
        let result = m.advance_parked(token, requested).unwrap();
        assert_eq!(result.advanced_ns, requested.min(1500));
        assert_eq!(result.service_required, requested >= 1500);
        assert_eq!(result.clipped, requested > 1500);
    }
}
