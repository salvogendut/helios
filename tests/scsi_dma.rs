use helios::bus::{Bus, BusErrorKind, MapError};
use helios::devices::{nvram::Nvram, scsi_dma::ScsiDma};
use helios::system_bus::{
    SystemBus, ESP_BASE, MACIO_ID_BASE, SCSI_DMA_BASE, SYSTEM_INTERRUPT_BASE,
};

#[test]
fn csr_identity_masks_and_empty_fifo_invalidate() {
    let mut d = ScsiDma::default();
    assert_eq!(d.read(0).unwrap(), 0xa000_0000);
    // Read-only ID, terminal count, status and reserved bits cannot be set.
    d.write(0, 0xfe00_dc0f).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa000_0000);
    // Ordinary writable controls: IRQ, direction, DMA, count, burst, timing.
    d.write(0, 0x00ab_2310).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa0ab_2310);
    d.write(4, 0x1234_5679).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa6ab_2310);
    d.write(8, 0xab12_3456).unwrap();
    assert_eq!(d.read(8).unwrap(), 0x0012_3456);
    d.write(0, 0x00ab_2330).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa6ab_2310);
    assert_eq!(d.read(4).unwrap(), 0x1234_5679);
    assert_eq!(d.read(8).unwrap(), 0x0012_3456);
}

#[test]
fn reset_is_held_and_clears_control_status_and_loaded_flags() {
    let mut d = ScsiDma::default();
    d.write(0, 0x0000_2310).unwrap();
    d.write(4, 0xfeed_0001).unwrap();
    d.set_scsi_irq(true);
    assert!(d.irq());
    d.write(0, 0x0000_2390).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa000_0080);
    assert!(d.reset_asserted());
    assert!(!d.irq());
    d.write(4, 123).unwrap();
    d.set_scsi_irq(true);
    assert_eq!(d.read(0).unwrap(), 0xa000_0080);
    d.write(0, 0x10).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa000_0010);
    d.reset();
    assert_eq!(d.read(0).unwrap(), 0xa000_0000);
}

#[test]
fn chained_address_and_count_loads_have_distinct_current_and_next_latches() {
    let mut d = ScsiDma::default();
    d.write(0, 0x0100_2200).unwrap();
    d.write(8, 0xff00_0003).unwrap();
    assert_eq!(d.read(8).unwrap(), 0);
    d.write(4, 0x1000).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa700_2200);
    assert_eq!(d.read(8).unwrap(), 3);
    d.write(8, 7).unwrap();
    d.write(4, 0x2000).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xaf00_2200);
    assert_eq!(d.read(4).unwrap(), 0x1000);
    assert_eq!(d.read(8).unwrap(), 3);
    d.write(0, 0x0100_2220).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa100_2200);
    d.write(4, 0x3000).unwrap();
    assert_eq!(d.read(4).unwrap(), 0x3000);
    assert_eq!(d.read(8).unwrap(), 7);
    d.write(4, 0x4000).unwrap();
    d.write(0, 0x2000).unwrap();
    assert_eq!(d.read(0).unwrap(), 0xa400_2000);
}

#[test]
fn unsupported_diagnostic_and_invalid_modes_preserve_state() {
    let mut d = ScsiDma::default();
    d.write(4, 0x1234).unwrap();
    let before = d.clone();
    for (offset, value) in [(12, 0), (0, 1 << 24), (0, 0xc0000), (0, 0x600000)] {
        assert!(d.write(offset, value).unwrap_err().is_host_stop());
        assert_eq!(d, before);
    }
    assert!(d.read(12).unwrap_err().is_host_stop());
    assert_eq!(d, before);
}

#[test]
fn macio_width_errors_latch_slave_error_and_w1c_acknowledges_it() {
    let mut b = SystemBus::default();
    b.install_sun4m(Nvram::default()).unwrap();
    assert_eq!(b.read32(MACIO_ID_BASE).unwrap(), 0xfe81_0103);
    assert_eq!(b.read8(MACIO_ID_BASE).unwrap(), 0xfe);
    assert_eq!(b.read8(MACIO_ID_BASE + 3).unwrap(), 0x03);
    assert_eq!(b.read16(MACIO_ID_BASE).unwrap(), 0xfe81);
    assert_eq!(b.read16(MACIO_ID_BASE + 2).unwrap(), 0x0103);
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa000_0000);
    b.write32(SCSI_DMA_BASE + 4, 0x1020_3040).unwrap();
    for e in [
        b.read8(SCSI_DMA_BASE).unwrap_err(),
        b.write16(SCSI_DMA_BASE + 4, 1).unwrap_err(),
        b.read_doubleword(SCSI_DMA_BASE).unwrap_err(),
        b.write_doubleword(SCSI_DMA_BASE, [1, 2]).unwrap_err(),
    ] {
        assert_eq!(e.kind, BusErrorKind::SBusError);
    }
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa400_0040);
    b.write32(SCSI_DMA_BASE, 0).unwrap();
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa400_0040);
    b.write32(SCSI_DMA_BASE, 0x40).unwrap();
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa400_0000);
    assert_eq!(b.read32(SCSI_DMA_BASE + 4).unwrap(), 0x1020_3040);
    assert_eq!(b.map_ram(SCSI_DMA_BASE, 16), Err(MapError::Overlap));
    for e in [
        b.ldstub8(SCSI_DMA_BASE).unwrap_err(),
        b.swap32(SCSI_DMA_BASE, 0).unwrap_err(),
        b.write32_masked(SCSI_DMA_BASE, 1, 255).unwrap_err(),
    ] {
        assert!(e.kind.is_host_stop());
    }
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa400_0000);
}

#[test]
fn scsi_irq_input_is_level_sensitive_and_gated_to_ncr_level_four() {
    let mut b = SystemBus::default();
    b.install_sun4m(Nvram::default()).unwrap();
    b.write32(SYSTEM_INTERRUPT_BASE + 8, (1 << 31) | (1 << 18))
        .unwrap();
    b.write8(ESP_BASE + 12, 0).unwrap();
    b.write8(ESP_BASE + 12, 0x45).unwrap();
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa000_0001);
    assert_eq!(b.interrupt_levels(), 0);
    b.write32(SCSI_DMA_BASE, 0x10).unwrap();
    assert_eq!(b.interrupt_levels(), 1 << 4);
    b.write32(SCSI_DMA_BASE, 0x30).unwrap();
    assert_eq!(b.interrupt_levels(), 1 << 4);
    assert_eq!(b.read8(ESP_BASE + 20).unwrap(), 8);
    assert_eq!(b.interrupt_levels(), 0);
    b.reset_devices();
    assert_eq!(b.read32(SCSI_DMA_BASE).unwrap(), 0xa000_0000);
}
