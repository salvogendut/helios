use helios::bus::{Bus, BusErrorKind, MapError};
use helios::devices::{esp::Esp, nvram::Nvram};
use helios::system_bus::{SystemBus, ESP_BASE, SCSI_DMA_BASE, SYSTEM_INTERRUPT_BASE};

#[test]
fn transfer_count_and_counter_are_separate_and_survive_reset() {
    let mut e = Esp::default();
    e.write(0, 0x34).unwrap();
    e.write(4, 0x12).unwrap();
    assert_eq!(e.read(0).unwrap(), 0);
    e.write(12, 0).unwrap();
    assert_eq!(e.read(0).unwrap(), 0);
    e.write(12, 0x80).unwrap();
    assert_eq!(e.read(0).unwrap(), 0x34);
    assert_eq!(e.read(4).unwrap(), 0x12);
    e.write(0, 0x56).unwrap();
    e.write(12, 2).unwrap();
    assert_eq!(e.read(0).unwrap(), 0x34);
    e.write(12, 0x80).unwrap();
    assert_eq!(e.read(0).unwrap(), 0x56);
    assert_eq!(e.read(4).unwrap(), 0x12);
    // Zero is encoded as zero when read (maximum count at transfer time),
    // and loading it does not assert terminal count or interrupt.
    e.write(0, 0).unwrap();
    e.write(4, 0).unwrap();
    e.write(12, 0x80).unwrap();
    assert_eq!(e.read(16).unwrap(), 0);
    assert!(!e.irq());
}

#[test]
fn chip_identity_requires_features_dma_nop_and_no_high_count_write() {
    let mut e = Esp::default();
    assert_eq!(e.read(56).unwrap(), 0);
    e.write(44, 0x40).unwrap();
    assert_eq!(e.read(56).unwrap(), 0);
    e.write(12, 0x80).unwrap();
    assert_eq!(e.read(56).unwrap(), 2);
    e.write(56, 0xab).unwrap();
    assert_eq!(e.read(56).unwrap(), 0);
    e.write(12, 0x80).unwrap();
    assert_eq!(e.read(56).unwrap(), 0xab);
    e.write(12, 2).unwrap();
    e.write(44, 0x40).unwrap();
    assert_eq!(e.read(56).unwrap(), 0xab);
    e.write(12, 0x80).unwrap();
    assert_eq!(e.read(56).unwrap(), 2);
    e.write(44, 0).unwrap();
    e.write(56, 0xee).unwrap(); // Disabled high-count register ignores writes.
    e.write(12, 0x80).unwrap();
    e.write(44, 0x40).unwrap();
    e.write(56, 0xab).unwrap();
    assert_eq!(e.read(56).unwrap(), 0); // Previous 16-bit DMA NOP cleared high counter.
}

#[test]
fn fifo_overwrite_empty_reads_and_interrupt_acknowledgement() {
    let mut e = Esp::default();
    e.write(12, 0).unwrap();
    assert_eq!(e.read(8).unwrap(), 0);
    for b in 1..=16 {
        e.write(8, b).unwrap();
    }
    e.write(8, 99).unwrap();
    assert_eq!(e.read(28).unwrap(), 16);
    assert_eq!(e.read(16).unwrap(), 0x40);
    assert!(!e.irq());
    assert_eq!(e.read(20).unwrap(), 0);
    assert_eq!(e.read(16).unwrap(), 0x40); // No IRQ: no status acknowledgement.
    for b in 1..=15 {
        assert_eq!(e.read(8).unwrap(), b);
    }
    assert_eq!(e.read(8).unwrap(), 99);
    assert_eq!(e.read(8).unwrap(), 99);
    assert_eq!(e.read(28).unwrap(), 0);
    e.write(12, 0x45).unwrap();
    assert_eq!(e.read(16).unwrap(), 0xc0);
    assert_eq!(e.read(20).unwrap(), 8);
    assert_eq!(e.read(16).unwrap(), 0);
    assert!(!e.irq());
    e.write(8, 0xaa).unwrap();
    e.write(12, 1).unwrap();
    assert_eq!(e.read(28).unwrap(), 0);
    assert_eq!(e.read(8).unwrap(), 0);
}

#[test]
fn configuration_readback_and_reset_follow_integrated_core() {
    let mut e = Esp::default();
    for (offset, value) in [(32, 0xf7), (44, 0xff), (48, 0xff)] {
        e.write(offset, value).unwrap();
        assert_eq!(e.read(offset).unwrap(), value);
    }
    e.reset();
    for offset in [32, 44, 48, 12, 16, 20, 24, 28] {
        assert_eq!(e.read(offset).unwrap(), 0);
    }
    let before = e.clone();
    for (offset, value) in [
        (32, 8),
        (36, 1),
        (40, 1),
        (52, 0),
        (12, 0x41),
        (12, 1),
        (1, 0),
    ] {
        assert!(e.write(offset, value).unwrap_err().is_host_stop());
        assert_eq!(e, before);
    }
    for offset in [36, 40, 52, 60, 1] {
        assert!(e.read(offset).unwrap_err().is_host_stop());
        assert_eq!(e, before);
    }
}

#[test]
fn scsi_bus_reset_retains_configuration_and_can_suppress_irq() {
    let mut e = Esp::default();
    e.write(12, 0).unwrap();
    e.write(36, 0).unwrap(); // 40 MHz -> factor 8.
    e.write(32, 0x17).unwrap();
    e.write(12, 3).unwrap();
    assert_eq!(e.read(12).unwrap(), 0);
    assert_eq!(e.read(32).unwrap(), 0x17);
    assert!(e.irq());
    assert_eq!(e.read(20).unwrap(), 0x80);
    assert!(!e.irq());
    e.advance_ns(25_999);
    let before = e.clone();
    assert!(e.write(12, 0x44).unwrap_err().is_host_stop());
    assert_eq!(e, before);
    e.advance_ns(1);
    e.write(12, 0x44).unwrap();
    e.write(32, 0x57).unwrap();
    e.write(12, 3).unwrap();
    assert_eq!(e.read(20).unwrap(), 0x80);
    assert!(!e.irq());
    assert_eq!(e.read(16).unwrap() & 0x80, 0);
}

#[test]
fn stacked_interrupt_is_explicit_and_atomic_until_acknowledgement() {
    let mut e = Esp::default();
    e.write(12, 0).unwrap();
    e.write(12, 0x45).unwrap();
    let before = e.clone();
    assert!(e.write(12, 0xc5).unwrap_err().is_host_stop());
    assert_eq!(e, before);
    assert_eq!(e.read(20).unwrap(), 8);
    e.write(12, 0x45).unwrap();
    assert!(e.irq());
}

#[test]
fn dma_reset_holds_esp_idle_and_irq_acknowledgement_reaches_the_cpu_wire() {
    let mut b = SystemBus::default();
    b.install_sun4m(Nvram::default()).unwrap();
    b.write32(SYSTEM_INTERRUPT_BASE + 8, (1 << 31) | (1 << 18))
        .unwrap();
    b.write32(SCSI_DMA_BASE, 0x10).unwrap();
    b.write8(ESP_BASE + 12, 0).unwrap();
    b.write8(ESP_BASE + 12, 0x45).unwrap();
    assert_eq!(b.interrupt_levels(), 1 << 4);
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa000_0011);
    assert_eq!(b.read8(ESP_BASE + 20).unwrap(), 8);
    assert_eq!(b.interrupt_levels(), 0);
    b.write32(SCSI_DMA_BASE, 0x80).unwrap();
    b.write8(ESP_BASE + 32, 0x17).unwrap();
    b.write8(ESP_BASE + 12, 0x45).unwrap();
    assert_eq!(b.read8(ESP_BASE + 32).unwrap(), 0);
    b.advance_ns(1_000_000);
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa000_0080);
    b.write32(SCSI_DMA_BASE, 0x10).unwrap();
    assert_eq!(b.read8(ESP_BASE + 32).unwrap(), 0);
    b.write8(ESP_BASE + 12, 0).unwrap();
    b.write8(ESP_BASE + 12, 0x45).unwrap();
    assert_eq!(b.interrupt_levels(), 1 << 4);
    b.reset_devices();
    assert_eq!(b.interrupt_levels(), 0);
    assert_eq!(b.read8(ESP_BASE + 28).unwrap(), 0);
}

#[test]
fn bus_widths_and_unsupported_operations_do_not_pop_fifo_or_mutate_state() {
    let mut b = SystemBus::default();
    b.install_sun4m(Nvram::default()).unwrap();
    b.write8(ESP_BASE + 8, 0x5a).unwrap();
    for error in [
        b.read32(ESP_BASE + 8).unwrap_err(),
        b.write16(ESP_BASE + 8, 0).unwrap_err(),
        b.swap32(ESP_BASE + 8, 0).unwrap_err(),
        b.write32_masked(ESP_BASE + 8, 0, 0xff).unwrap_err(),
        b.read_doubleword(ESP_BASE + 8).unwrap_err(),
        b.load_image(ESP_BASE + 8, &[0]).unwrap_err(),
        b.read8(ESP_BASE + 9).unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop());
    }
    assert_eq!(b.read8(ESP_BASE + 28).unwrap(), 1);
    assert_eq!(b.read8(ESP_BASE + 8).unwrap(), 0x5a);
    assert_eq!(b.map_ram(ESP_BASE, 64), Err(MapError::Overlap));
    assert_eq!(
        b.read8(ESP_BASE + 64).unwrap_err().kind,
        BusErrorKind::SBusTimeout
    );
}
