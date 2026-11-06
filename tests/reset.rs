use helios::bus::{Bus, BusErrorKind, MapError};
use helios::cpu::{Step, PSR_ET, PSR_PS, PSR_S};
use helios::devices::interrupt::MASK_BITS;
use helios::devices::nvram::Nvram;
use helios::devices::reset::ResetControl;
use helios::devices::serial::Channel;
use helios::machine::Machine;
use helios::mmu::{Srmmu, Width, PCR_BM, PCR_EN};
use helios::system_bus::{
    SystemBus, CPU_INTERRUPT_BASE, NVRAM_BASE, RESET_BASE, SERIAL_BASE, SYSTEM_INTERRUPT_BASE,
    SYSTEM_TIMER_BASE,
};

fn machine(words: &[u32]) -> Machine {
    let prom: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    Machine::sun4m(0x4000, &prom).unwrap()
}

#[test]
fn reset_causes_are_read_clear_and_software_request_is_sticky() {
    let mut r = ResetControl::default();
    assert_eq!(r.read(), 0);
    r.write(0xffff_fffe); // ones cannot manufacture either status bit
    assert_eq!(r.read(), 0);
    assert!(!r.requested());
    r.latch_watchdog();
    assert_eq!(r.read(), 0x10);
    r.write(0x10);
    assert_eq!(r.read(), 0x10);
    r.write(0);
    assert_eq!(r.read(), 0);
    r.write(1);
    r.write(0); // request cannot be retracted
    assert!(r.requested());
    assert_eq!(r.read(), 0); // SR is write-only
    r.software_reset();
    assert_eq!(r.read(), 2);
    assert!(!r.requested());
    r.latch_watchdog();
    assert_eq!(r.read(), 0x12); // watchdog does not clear earlier software cause
    r.write(2);
    assert_eq!(r.read(), 2);
    r.write(0x10);
    assert_eq!(r.read(), 0); // clear RS, cannot set WD
    r.latch_watchdog();
    r.software_reset();
    assert_eq!(r.read(), 2); // software reset clears watchdog
    r.power_on_reset();
    assert_eq!(r.read(), 0);
}

#[test]
fn reset_mmio_checks_all_widths_and_mapping_conflicts_before_side_effects() {
    let mut m = machine(&[0x0100_0000]);
    m.bus.devices_mut().unwrap().reset.latch_watchdog();
    for error in [
        m.bus.read8(RESET_BASE).unwrap_err(),
        m.bus.read16(RESET_BASE).unwrap_err(),
        m.bus.read_doubleword(RESET_BASE).unwrap_err(),
        m.bus.write8(RESET_BASE + 3, 1).unwrap_err(),
        m.bus.write16(RESET_BASE + 2, 1).unwrap_err(),
        m.bus.write_doubleword(RESET_BASE, [1, 0]).unwrap_err(),
        m.bus.write32_masked(RESET_BASE, 1, 0xff).unwrap_err(),
        m.bus.swap32(RESET_BASE, 1).unwrap_err(),
        m.bus.ldstub8(RESET_BASE).unwrap_err(),
        m.bus.load_image(RESET_BASE, &[0; 4]).unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop(), "{error:?}");
    }
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0x10);
    assert!(!m.bus.devices().unwrap().reset.requested());
    assert_eq!(
        m.bus.read32(RESET_BASE + 1).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(
        m.bus.read32(RESET_BASE + 4).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    assert_eq!(m.bus.map_ram(RESET_BASE, 4), Err(MapError::Overlap));
    assert_eq!(m.bus.map_rom(RESET_BASE, &[0; 4]), Err(MapError::Overlap));
    let mut bus = SystemBus::default();
    bus.map_ram(RESET_BASE, 4).unwrap();
    assert_eq!(bus.install_sun4m(Nvram::default()), Err(MapError::Overlap));
}

#[test]
fn guest_reset_in_branch_delay_slot_restarts_prom_and_resets_io_once() {
    let mut m = machine(&[
        0x1080_0004, // ba 0x10
        0xc4a0_4400, // sta g2, [g1] ASI 0x20 (delay slot)
        0x0100_0000,
        0x0100_0000,
        0x86a0_e001, // subcc g3, 1, g3 -- must not execute
    ]);
    m.cpu.write_reg(1, RESET_BASE as u32);
    m.cpu.write_reg(2, 1);
    m.cpu.write_reg(3, 42);
    m.bus.write32(0x100, 0x1234_5678).unwrap();
    m.bus.write8(NVRAM_BASE + 0x123, 0xab).unwrap();
    // Enable async transmit and receive, then leave one RX byte pending.
    for (reg, value) in [(14, 3), (3, 0xc1), (5, 0x68)] {
        m.bus.write8(SERIAL_BASE + 4, reg).unwrap();
        m.bus.write8(SERIAL_BASE + 4, value).unwrap();
    }
    assert!(m
        .bus
        .devices_mut()
        .unwrap()
        .serial
        .receive(Channel::A, b'R'));
    m.bus.write8(SERIAL_BASE + 6, b'T').unwrap();
    m.bus.write32(SYSTEM_TIMER_BASE, 1024).unwrap();
    m.bus.write32(CPU_INTERRUPT_BASE + 8, 1 << 22).unwrap();
    m.bus.write32(SYSTEM_INTERRUPT_BASE + 8, u32::MAX).unwrap();
    m.advance_ns(999_999_800);
    m.step().unwrap();
    assert_eq!(
        m.step().unwrap(),
        Step::SoftwareReset {
            pc: 4,
            npc: 16,
            instruction: Some(0xc4a0_4400)
        }
    );
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0, 4));
    assert_eq!(m.cpu.mmu.control() & (PCR_EN | PCR_BM), PCR_BM);
    assert_eq!(m.cpu.psr() & (PSR_ET | PSR_S), PSR_S);
    assert_eq!(m.cpu.read_reg(3), 42);
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 2);
    assert_eq!(m.bus.read32(0x100).unwrap(), 0x1234_5678);
    assert_eq!(m.bus.read8(NVRAM_BASE + 0x123).unwrap(), 0xab);
    assert_eq!(m.bus.read8(NVRAM_BASE + 0x1ff9).unwrap(), 1);
    assert_eq!(m.bus.read32(CPU_INTERRUPT_BASE).unwrap(), 0);
    assert_eq!(m.bus.read32(SYSTEM_INTERRUPT_BASE + 4).unwrap(), MASK_BITS);
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 4).unwrap(), 0);
    assert_eq!(m.bus.interrupt_levels(), 0);
    assert_eq!(m.bus.read8(SERIAL_BASE + 4).unwrap() & 1, 0);
    assert!(!m
        .bus
        .devices_mut()
        .unwrap()
        .serial
        .receive(Channel::A, b'R'));
    assert_eq!(
        m.bus.devices_mut().unwrap().serial.take_output(Channel::A),
        b"T"
    );
    assert!(matches!(m.step().unwrap(), Step::Executed { pc: 0, .. }));
    m.reset();
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0);
}

#[test]
fn software_reset_retains_windows_ps_and_tt() {
    let mut m = machine(&[
        0x8188_6023, // wr g1, 0x23, psr (ET,CWP=3; S=PS=0)
        0x91d0_2007, // ta 7: window 2, TT=0x87
    ]);
    m.step().unwrap();
    m.cpu.write_reg(16, 0x1234); // window 3, retained across reset and re-entry
    m.step().unwrap();
    assert_eq!(m.cpu.current_window(), 2);
    m.cpu.write_reg(16, 0x5678);
    let saved = (m.cpu.read_reg(17), m.cpu.read_reg(18));
    let ps = m.cpu.psr() & PSR_PS;
    assert!(m.step().is_err()); // padded PROM is unsupported; no fake watchdog
    m.bus.write32(RESET_BASE, 1).unwrap();
    assert!(matches!(
        m.step().unwrap(),
        Step::SoftwareReset {
            instruction: None,
            ..
        }
    ));
    assert_eq!(m.cpu.current_window(), 2);
    assert_eq!(m.cpu.psr() & PSR_PS, ps);
    assert_eq!(m.cpu.tbr(), 0x870);
    assert_eq!(m.cpu.read_reg(16), 0x5678);
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), saved);
    assert!(m.cpu.error_state().is_none());
    m.step().unwrap(); // same wrpsr returns to window 3
    assert_eq!(m.cpu.read_reg(16), 0x1234);
}

#[test]
fn cpu_error_latches_watchdog_without_resetting_devices_or_repeating_the_event() {
    let mut m = machine(&[0]); // UNIMP with ET=0
    m.bus.write32(CPU_INTERRUPT_BASE + 8, 1 << 22).unwrap();
    m.advance_ns(1250);
    let fault = m.step().unwrap_err();
    assert!(m.cpu.error_state().is_some());
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0x10);
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 4).unwrap(), 1024);
    assert_eq!(m.bus.read32(CPU_INTERRUPT_BASE).unwrap(), 1 << 22);
    m.bus.write32(RESET_BASE, 0).unwrap();
    assert_eq!(m.step().unwrap_err(), fault);
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0); // no new IU_ERROR edge
    m.bus.write32(RESET_BASE, 1).unwrap();
    assert!(matches!(
        m.step().unwrap(),
        Step::SoftwareReset {
            instruction: None,
            ..
        }
    ));
    assert!(m.cpu.error_state().is_none());
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 2);
    m.step().unwrap_err();
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0x12);

    let mut unsupported = machine(&[0xffff_ffff]);
    unsupported.step().unwrap_err();
    assert!(unsupported.cpu.error_state().is_none());
    assert_eq!(unsupported.bus.read32(RESET_BASE).unwrap(), 0);
}

#[test]
fn translated_read_only_mapping_cannot_request_a_reset() {
    let mut m = machine(&[0x0100_0000]);
    m.bus.write32(0x1000, 0x201).unwrap(); // context table -> L1 at 0x2000
    m.bus.write32(0x2000, 0x301).unwrap(); // L1 -> L2 at 0x3000
    m.bus.write32(0x3000, 0x071f_0002).unwrap(); // level-2 PTE, ACC=0 read-only
    let mut mmu = Srmmu::default();
    mmu.store(&mut m.bus, 0x100, 4, Width::Word, 0x100).unwrap();
    mmu.store(&mut m.bus, 0, 4, Width::Word, PCR_EN).unwrap();
    assert_eq!(mmu.load(&mut m.bus, 0, 0xb, Width::Word).unwrap(), 0);
    assert!(mmu.store(&mut m.bus, 0, 0xb, Width::Word, 1).is_err());
    assert!(!m.bus.devices().unwrap().reset.requested());
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0);
}
