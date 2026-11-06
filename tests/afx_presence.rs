use helios::bus::{Bus, BusErrorKind, MapError};
use helios::cpu::Step;
use helios::devices::nvram::Nvram;
use helios::machine::Machine;
use helios::system_bus::{SystemBus, AFX_PRESENCE_BASE};

#[test]
fn empty_afx_slot_presence_is_board_state_and_survives_reset() {
    let mut bus = SystemBus::default();
    assert_eq!(
        bus.read8(AFX_PRESENCE_BASE).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    bus.install_sun4m(Nvram::default()).unwrap();
    assert_eq!(bus.read8(AFX_PRESENCE_BASE).unwrap(), 0);
    for error in [
        bus.write8(AFX_PRESENCE_BASE, 1).unwrap_err(),
        bus.read16(AFX_PRESENCE_BASE).unwrap_err(),
        bus.read32(AFX_PRESENCE_BASE).unwrap_err(),
        bus.read_doubleword(AFX_PRESENCE_BASE).unwrap_err(),
        bus.write32(AFX_PRESENCE_BASE, 1).unwrap_err(),
        bus.write32_masked(AFX_PRESENCE_BASE, 1, 255).unwrap_err(),
        bus.ldstub8(AFX_PRESENCE_BASE).unwrap_err(),
        bus.load_image(AFX_PRESENCE_BASE, &[1]).unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop());
    }
    assert_eq!(bus.read8(AFX_PRESENCE_BASE).unwrap(), 0);
    bus.reset_devices();
    assert_eq!(bus.read8(AFX_PRESENCE_BASE).unwrap(), 0);
    assert_eq!(bus.map_ram(AFX_PRESENCE_BASE, 4), Err(MapError::Overlap));
    assert_eq!(
        bus.map_rom(AFX_PRESENCE_BASE, &[0; 4]),
        Err(MapError::Overlap)
    );
    // Presence does not map an AFX framebuffer or adjacent undocumented bytes.
    assert_eq!(
        bus.read8(AFX_PRESENCE_BASE + 1).unwrap_err().kind,
        BusErrorKind::SBusTimeout
    );
}

#[test]
fn reset_prom_reads_afx_presence_through_physical_bypass_without_a_trap() {
    let prom = 0xc488_4400u32.to_be_bytes(); // lduba [g1] ASI20, g2
    let mut m = Machine::sun4m(4096, &prom).unwrap();
    m.cpu.write_reg(1, AFX_PRESENCE_BASE as u32);
    m.cpu.write_reg(2, u32::MAX);
    assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    assert_eq!(m.cpu.read_reg(2), 0);
    assert!(m.cpu.error_state().is_none());
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}
