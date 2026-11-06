use helios::bus::{Bus, BusErrorKind};
use helios::devices::{floppy::Floppy, nvram::Nvram};
use helios::system_bus::{SystemBus, FLOPPY_BASE, SYSTEM_INTERRUPT_BASE};

fn send(f: &mut Floppy, bytes: &[u8]) {
    for &byte in bytes {
        assert_eq!(f.read(4).unwrap() & 0xc0, 0x80);
        f.write(5, byte).unwrap();
    }
}
fn result(f: &mut Floppy, n: usize) -> Vec<u8> {
    (0..n)
        .map(|_| {
            assert_eq!(f.read(4).unwrap() & 0xf0, 0xd0);
            f.read(5).unwrap()
        })
        .collect()
}

#[test]
fn floppy_reset_poll_is_delayed_maskable_and_has_four_sense_results() {
    let mut f = Floppy::default();
    assert_eq!(f.read(2).unwrap(), 0);
    assert_eq!(f.read(4).unwrap(), 0);
    assert_eq!(f.write(5, 0x10), Err(BusErrorKind::WouldBlock));
    f.write(2, 4).unwrap(); // released, interrupt gate still closed
    f.advance_ns(499_999);
    assert!(!f.irq());
    send(&mut f, &[8]);
    assert_eq!(result(&mut f, 1), [0x80]); // no interrupt before deadline
    f.advance_ns(1);
    assert!(!f.irq());
    f.write(2, 12).unwrap();
    assert!(f.irq());
    for drive in 0..4 {
        send(&mut f, &[8]);
        assert!(!f.irq());
        assert_eq!(result(&mut f, 2), [0xc0 | drive, 0]);
        assert_eq!(f.read(4).unwrap(), 0x80);
    }
    send(&mut f, &[8]);
    assert_eq!(result(&mut f, 1), [0x80]);
    f.advance_ns(u64::MAX);
    assert!(!f.irq()); // polling notification is not periodic
}

#[test]
fn floppy_configuration_cancels_pending_poll_and_result_phase_blocks_writes() {
    let mut f = Floppy::default();
    f.write(2, 0xff).unwrap();
    assert_eq!(f.read(2).unwrap(), 0xdd);
    send(&mut f, &[0x13]);
    assert_eq!(f.read(4).unwrap(), 0x90);
    f.advance_ns(2_000_000); // first CONFIGURE byte pauses reset polling
    assert!(!f.irq());
    send(&mut f, &[0, 0x5f, 42]);
    assert_eq!(f.read(4).unwrap(), 0x80);
    f.advance_ns(u64::MAX);
    assert!(!f.irq());
    send(&mut f, &[3, 0xab, 0x17]);
    send(&mut f, &[0x10]);
    let before = f.clone();
    assert_eq!(f.write(5, 8), Err(BusErrorKind::WouldBlock));
    assert_eq!(f, before);
    assert_eq!(result(&mut f, 1), [0x90]);
    send(&mut f, &[0x0e]);
    assert_eq!(result(&mut f, 10), [0, 0, 0, 0, 0xab, 0x17, 0, 0, 0x5f, 42]);
    assert_eq!(f.read(5), Err(BusErrorKind::WouldBlock));
}

#[test]
fn floppy_lock_and_reset_preserve_documented_configuration_and_specify() {
    let mut f = Floppy::default();
    f.write(2, 12).unwrap();
    send(&mut f, &[3, 0xa5, 0x21]);
    send(&mut f, &[0x13, 0, 0x5f, 99]);
    send(&mut f, &[0x12, 0xbd]);
    send(&mut f, &[0x12, 2]); // OW=0 preserves drive bits
    send(&mut f, &[0x94]);
    assert_eq!(result(&mut f, 1), [0x10]);
    f.write(4, 0x80).unwrap();
    send(&mut f, &[0x0e]);
    assert_eq!(
        result(&mut f, 10),
        [0, 0, 0, 0, 0xa5, 0x21, 0, 0xbc, 0x5f, 99]
    );
    send(&mut f, &[0x14]);
    assert_eq!(result(&mut f, 1), [0]);
    f.write(2, 0).unwrap();
    f.write(2, 12).unwrap();
    send(&mut f, &[0x0e]);
    assert_eq!(
        result(&mut f, 10),
        [0, 0, 0, 0, 0xa5, 0x21, 0, 0x3c, 0x70, 0]
    );
    f.reset();
    assert_eq!(f.read(2).unwrap(), 0);
    f.write(2, 12).unwrap();
    send(&mut f, &[0x0e]);
    assert_eq!(result(&mut f, 10), [0, 0, 0, 0, 0xa5, 0x21, 0, 0, 0x20, 0]);
}

#[test]
fn floppy_unimplemented_operations_leave_state_unchanged() {
    let mut f = Floppy::default();
    f.write(2, 12).unwrap();
    let before = f.clone();
    for code in [0x06, 0x0a, 0xff] {
        assert!(f.write(5, code).unwrap_err().is_host_stop());
        assert_eq!(f, before);
    }
    assert!(f.write(4, 0xc3).unwrap_err().is_host_stop());
    assert_eq!(f, before);
    send(&mut f, &[4, 5]);
    assert_eq!(result(&mut f, 1), [0x2d]); // ST3 fixed 5/3, head and unit
    assert_eq!(f.read(7).unwrap(), 0); // disconnected disk-change input
}

#[test]
fn floppy_reset_interrupt_reaches_ncr_level_eleven_and_acknowledges_through_fifo() {
    let mut b = SystemBus::default();
    b.install_sun4m(Nvram::default()).unwrap();
    b.write32(SYSTEM_INTERRUPT_BASE + 8, (1 << 31) | (1 << 22))
        .unwrap();
    b.write8(FLOPPY_BASE + 2, 12).unwrap();
    b.advance_ns(500_000);
    assert_eq!(b.devices().unwrap().interrupts.levels(), 1 << 11);
    assert_eq!(
        b.read32(SYSTEM_INTERRUPT_BASE).unwrap() & (1 << 22),
        1 << 22
    );
    b.write8(FLOPPY_BASE + 5, 8).unwrap();
    assert_eq!(b.devices().unwrap().interrupts.levels(), 0);
    assert_eq!(b.read8(FLOPPY_BASE + 5).unwrap(), 0xc0);
    assert_eq!(b.read8(FLOPPY_BASE + 5).unwrap(), 0);
    let before = b.devices().unwrap().floppy.clone();
    assert_eq!(
        b.read32(FLOPPY_BASE + 4).unwrap_err().kind,
        BusErrorKind::SBusError
    );
    assert_eq!(
        b.write16(FLOPPY_BASE + 2, 0).unwrap_err().kind,
        BusErrorKind::SBusError
    );
    assert_eq!(b.devices().unwrap().floppy, before);
    b.write8(FLOPPY_BASE + 2, 0).unwrap();
    assert_eq!(b.devices().unwrap().interrupts.levels(), 0);
}

#[test]
fn disconnected_recalibration_times_out_and_seek_tracks_internal_cylinder() {
    let mut f = Floppy::default();
    f.write(2, 12).unwrap();
    send(&mut f, &[0x13, 0, 0x10, 0]); // disable reset polling
    send(&mut f, &[3, 0xc0, 1]); // four milliseconds per step at 500 kbit/s
    send(&mut f, &[7, 0]);
    assert_eq!(f.read(4).unwrap(), 0x81); // command-ready but drive busy
    f.advance_ns(315_999_999);
    assert!(!f.irq());
    f.advance_ns(1);
    assert!(f.irq());
    assert_eq!(f.read(4).unwrap(), 0x81); // busy until SENSE INTERRUPT
    send(&mut f, &[8]);
    assert!(!f.irq());
    assert_eq!(result(&mut f, 2), [0x70, 0]);
    assert_eq!(f.read(4).unwrap(), 0x80);
    send(&mut f, &[0x0f, 0, 3]);
    f.advance_ns(11_999_999);
    assert!(!f.irq());
    f.advance_ns(1);
    assert!(f.irq());
    send(&mut f, &[8]);
    assert_eq!(result(&mut f, 2), [0x20, 3]);
    send(&mut f, &[0x0e]);
    assert_eq!(result(&mut f, 10)[0], 3);
    send(&mut f, &[7, 0]);
    f.write(2, 0).unwrap();
    f.advance_ns(u64::MAX);
    assert!(!f.irq()); // reset aborts outstanding movement
}
