use helios::bus::{AccessContext, Bus, BusErrorKind, MapError};
use helios::cpu::{Cpu, FaultKind, Step, PSR_ET, PSR_S};
use helios::devices::io_control::IoControl;
use helios::machine::Machine;
use helios::mmu::{MemoryError, Width, PCR_EN, PCR_NF};
use helios::system_bus::{
    CPU_INTERRUPT_BASE as CPU_IRQ, FLOPPY_BASE, IO_CONTROL_BASE as IO, RESET_BASE,
    SYSTEM_INTERRUPT_BASE as IRQ,
};

const AFSR: u64 = IO + 0x1000;
const AFAR: u64 = IO + 0x1004;
const MFSR: u64 = IO + 0x1050;
const MFAR: u64 = IO + 0x1054;
const EMPTY: u32 = 0x7cc0_0000;
const ME: u32 = 1 << 30;

fn fixture() -> Machine {
    Machine::sun4m(0x20000, &0x0100_0000u32.to_be_bytes()).unwrap()
}
fn put(m: &mut Machine, address: u32, asi: u8, value: u32) {
    m.cpu
        .mmu
        .store(&mut m.bus, address, asi, Width::Word, value)
        .unwrap();
}
fn get(m: &mut Machine, address: u32, asi: u8) -> Result<u32, MemoryError> {
    m.cpu.mmu.load(&mut m.bus, address, asi, Width::Word)
}
fn mapping(m: &mut Machine, pte: u32) {
    for (address, value) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4010, pte),
    ] {
        m.bus.write32(address, value).unwrap();
    }
    put(m, 0x100, 4, 0x100);
    put(m, 0, 4, PCR_EN);
}

#[test]
fn sbus_slot_holes_time_out_but_ram_control_and_graphics_holes_remain_distinct() {
    let mut m = fixture();
    for address in [
        0x3000_0100,
        0x4000_0100,
        0x5000_0100,
        0x6000_0100,
        EMPTY as u64,
    ] {
        assert_eq!(
            m.bus.read8(address).unwrap_err().kind,
            BusErrorKind::SBusTimeout
        );
        assert_eq!(
            m.bus.read16(address).unwrap_err().kind,
            BusErrorKind::SBusTimeout
        );
        assert_eq!(
            m.bus.read32(address).unwrap_err().kind,
            BusErrorKind::SBusTimeout
        );
        assert_eq!(
            m.bus.read_doubleword(address).unwrap_err().kind,
            BusErrorKind::SBusTimeout
        );
        assert_eq!(
            m.bus.write32(address, 123).unwrap_err().kind,
            BusErrorKind::SBusTimeout
        );
    }
    for address in [0x20000, IO + 0x8000, 0x2000_0100, 0x8000_0000] {
        assert_eq!(
            m.bus.read32(address).unwrap_err().kind,
            BusErrorKind::Unmapped
        );
    }
    let mut bare = Machine::bare_metal(4096, 0).unwrap();
    assert_eq!(
        bare.bus.read32(EMPTY.into()).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
    m.bus.map_ram(0x4000_0100, 4).unwrap();
    m.bus.write32(0x4000_0100, 0x8765_4321).unwrap();
    assert_eq!(m.bus.read32(0x4000_0100).unwrap(), 0x8765_4321);
    assert_eq!(
        m.bus.read32(EMPTY as u64 + 1).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    // Raw/debugger bus failures do not masquerade as CPU-issued requests.
    assert_eq!(m.bus.read32(AFSR).unwrap(), 0x0080_0000);
    assert_eq!(m.bus.read32(IRQ).unwrap(), 0);
}

#[test]
fn ncr_rejects_wide_floppy_accesses_but_byte_emulation_remains_a_host_stop() {
    let mut m = fixture();
    for address in [
        0x7150_0000,
        0x717f_fffc,
        0x71c0_0000,
        CPU_IRQ + 0x10,
        IRQ + 0x14,
        RESET_BASE + 4,
    ] {
        assert_eq!(
            m.bus.read32(address).unwrap_err().kind,
            BusErrorKind::SBusError
        );
    }
    for address in [FLOPPY_BASE, FLOPPY_BASE + 0x300, FLOPPY_BASE + 0xffff8] {
        assert_eq!(
            m.bus.read16(address).unwrap_err().kind,
            BusErrorKind::SBusError
        );
        assert_eq!(
            m.bus.read32(address).unwrap_err().kind,
            BusErrorKind::SBusError
        );
        assert_eq!(
            m.bus.write32(address, 0).unwrap_err().kind,
            BusErrorKind::SBusError
        );
        assert!(m.bus.read8(address).unwrap_err().kind.is_host_stop());
        assert!(m.bus.write8(address, 0).unwrap_err().kind.is_host_stop());
        assert!(m.bus.swap32(address, 0).unwrap_err().kind.is_host_stop());
        assert!(m
            .bus
            .write32_masked(address, 0, 0xff)
            .unwrap_err()
            .kind
            .is_host_stop());
        assert_eq!(m.bus.map_ram(address, 4), Err(MapError::Overlap));
    }
    assert_eq!(
        m.bus.read32(FLOPPY_BASE + 1).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
}

#[test]
fn synchronous_status_distinguishes_timeout_from_slave_error_and_retains_virtual_address() {
    let mut m = fixture();
    mapping(&mut m, 0x07cc_000e); // VA 4000 -> PA 7cc00000, user/supervisor RWX
    assert!(
        matches!(get(&mut m, 0x4000, 11), Err(MemoryError::Bus(e)) if e.kind == BusErrorKind::SBusTimeout && e.address == u64::from(EMPTY))
    );
    assert_eq!(m.cpu.mmu.fault_status(), 0x836); // TO, supervisor data read, FT5, FAV
    assert_eq!(m.cpu.mmu.fault_address(), 0x4000);
    assert_eq!(get(&mut m, 0x300, 4).unwrap(), 0x836); // destructive status read
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert!(
        matches!(get(&mut m, (FLOPPY_BASE + 0x300) as u32, 0x20), Err(MemoryError::Bus(e)) if e.kind == BusErrorKind::SBusError)
    );
    assert_eq!(m.cpu.mmu.fault_status(), 0x436);
    assert_eq!(m.cpu.mmu.fault_address(), (FLOPPY_BASE + 0x300) as u32);
    assert_eq!(m.bus.read32(AFSR).unwrap(), 0x0080_0000);
    assert_eq!(m.bus.read32(IRQ).unwrap(), 0);
}

#[test]
fn no_fault_and_priority_rules_do_not_lose_or_attach_timeout_bits_to_the_wrong_fault() {
    let mut m = fixture();
    put(&mut m, 0, 4, PCR_NF);
    assert_eq!(get(&mut m, EMPTY, 11), Ok(0));
    assert_eq!(m.cpu.mmu.fault_status(), 0x836);
    assert_eq!(get(&mut m, FLOPPY_BASE as u32, 11), Ok(0));
    assert_eq!(m.cpu.mmu.fault_status(), 0x437); // replacement cause, overwrite bit
    put(&mut m, 0x1300, 4, 0x132); // higher-priority level1 table-walk fault
    put(&mut m, 0x1400, 4, 0x1234_5678);
    assert_eq!(get(&mut m, EMPTY, 11), Ok(0));
    assert_eq!(m.cpu.mmu.fault_status(), 0x132);
    assert_eq!(m.cpu.mmu.fault_address(), 0x1234_5678);
    // Missing FDC implementation is not converted to a timeout, even under NF.
    let before = m.cpu.mmu.clone();
    assert!(
        matches!(m.cpu.mmu.load(&mut m.bus, FLOPPY_BASE as u32, 11, Width::Byte), Err(MemoryError::Bus(e)) if e.kind.is_host_stop())
    );
    assert_eq!(m.cpu.mmu, before);
}

#[test]
fn cpu_read_error_is_precise_with_traps_disabled_and_sets_watchdog_status() {
    let mut m = Machine::sun4m(4096, &0xc400_6000u32.to_be_bytes()).unwrap(); // ld [g1],g2
    m.cpu.write_reg(1, EMPTY);
    m.cpu.write_reg(2, 0x1234_5678);
    let error = m.step().unwrap_err();
    assert!(matches!(error.kind, FaultKind::DataAccess(e) if e.kind == BusErrorKind::SBusTimeout));
    assert_eq!(m.cpu.read_reg(2), 0x1234_5678);
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0, 4));
    assert_eq!(m.cpu.mmu.fault_status(), 0x836);
    assert_eq!(m.step().unwrap_err(), error);
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0x10);
}

#[test]
fn cpu_store_errors_complete_and_report_physical_address_size_and_privilege_asynchronously() {
    for (width, size) in [(Width::Byte, 1), (Width::Half, 2), (Width::Word, 0)] {
        for supervisor in [false, true] {
            let mut m = fixture();
            mapping(&mut m, 0x07cc_000e);
            m.bus.set_cpu_supervisor(supervisor);
            m.cpu
                .mmu
                .store(
                    &mut m.bus,
                    0x4000,
                    if supervisor { 11 } else { 10 },
                    width,
                    0x1234,
                )
                .unwrap();
            assert_eq!(m.cpu.mmu.fault_status(), 0);
            assert_eq!(m.cpu.mmu.fault_address(), 0);
            assert_eq!(m.bus.read32(AFAR).unwrap(), EMPTY);
            assert_eq!(
                m.bus.read32(AFSR).unwrap(),
                0xa082_0000 | (size << 25) | (u32::from(supervisor) << 24)
            );
            assert_eq!(m.bus.read32(IRQ).unwrap(), ME);
        }
    }
    let mut m = fixture();
    put(&mut m, FLOPPY_BASE as u32, 0x20, 1);
    assert_eq!(m.bus.read32(AFSR).unwrap(), 0x9182_0000); // BE, not TO
    assert_eq!(m.bus.read32(AFAR).unwrap(), FLOPPY_BASE as u32);
}

#[test]
fn asynchronous_capture_locks_first_fault_until_status_read_and_reports_multiple_errors() {
    let mut m = fixture();
    put(&mut m, EMPTY, 0x20, 1);
    assert_eq!(m.bus.read32(AFAR).unwrap(), EMPTY); // address read does not unlock
    put(&mut m, FLOPPY_BASE as u32, 0x20, 2);
    assert_eq!(m.bus.read32(AFAR).unwrap(), EMPTY);
    assert_eq!(m.bus.read32(AFSR).unwrap(), 0xa18a_0000); // first TO + ME
    put(&mut m, FLOPPY_BASE as u32 + 4, 0x20, 3);
    assert_eq!(m.bus.read32(AFAR).unwrap(), FLOPPY_BASE as u32 + 4);
    assert_eq!(m.bus.read32(AFSR).unwrap(), 0x9182_0000); // new BE, ME clear
    m.bus.write32(AFSR, 0).unwrap();
    assert_eq!(m.bus.read32(IRQ).unwrap(), ME); // capture acknowledgement is separate from NCR IC
}

#[test]
fn module_interrupt_masking_and_ic_acknowledgement_preserve_software_interrupts() {
    let mut m = fixture();
    put(&mut m, EMPTY, 0x20, 0);
    assert_eq!(m.bus.interrupt_levels(), 0); // all reset masks set
    m.bus.write32(IRQ + 8, 1 << 31).unwrap();
    assert_eq!(m.bus.interrupt_levels(), 0); // source still masked
    m.bus.write32(IRQ + 8, ME).unwrap();
    assert_eq!(m.bus.interrupt_levels(), 1 << 15);
    m.bus.write32(CPU_IRQ + 8, 1 << 31).unwrap(); // independent software level15
    m.bus.write32(CPU_IRQ + 4, 1 << 15).unwrap();
    assert_eq!(m.bus.read32(IRQ).unwrap(), 0);
    assert_eq!(m.bus.interrupt_levels(), 1 << 15);
    m.bus.write32(CPU_IRQ + 4, 1 << 31).unwrap();
    assert_eq!(m.bus.interrupt_levels(), 0);
    put(&mut m, EMPTY, 0x20, 0);
    m.bus.write32(IRQ + 12, ME).unwrap(); // source-mask acknowledgement
    m.bus.write32(IRQ + 8, ME).unwrap();
    assert_eq!(m.bus.interrupt_levels(), 0);
}

#[test]
fn guest_store_completes_before_module_interrupt_and_level_15_bypasses_pil() {
    let words = [
        0x8198_6000u32,
        0x8188_a000,
        0x0100_0000,
        0x0100_0000,
        0x0100_0000,
        0xc620_a000,
        0x0100_0000,
    ];
    let prom: Vec<_> = words.into_iter().flat_map(u32::to_be_bytes).collect();
    let mut m = Machine::sun4m(0x2000, &prom).unwrap();
    m.cpu.write_reg(1, 0x1000);
    m.cpu.write_reg(2, PSR_S | PSR_ET | 0xf00);
    for _ in 0..5 {
        m.step().unwrap();
    }
    m.cpu.write_reg(2, EMPTY);
    m.cpu.write_reg(3, 0x1234);
    m.bus.write32(IRQ + 8, ME | (1 << 31)).unwrap();
    assert!(matches!(m.step().unwrap(), Step::Executed { pc: 20, .. }));
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (24, 28));
    assert!(m.cpu.error_state().is_none());
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert_eq!(
        m.step().unwrap(),
        Step::InterruptTaken {
            pc: 24,
            npc: 28,
            level: 15,
            vector: 0x11f0
        }
    );
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (24, 28));
}

#[test]
fn fault_register_masks_read_unlocks_and_memory_requests_follow_the_manual() {
    let mut m = fixture();
    for (address, expected) in [
        (AFSR, 0xff8e_0000),
        (AFAR, 0x7fff_ffff),
        (MFSR, 0x8188_f8f0),
        (MFAR, 0x7fff_ffff),
    ] {
        m.bus.write32(address, u32::MAX).unwrap();
        assert_eq!(m.bus.read32(address).unwrap(), expected);
    }
    assert_eq!(m.bus.read32(IRQ).unwrap(), 0); // diagnostic writes do not create hardware errors
    for (width, address, expected) in [
        (Width::Byte, 0x8001, 0x0180_1090),
        (Width::Half, 0x8002, 0x0180_10a0),
        (Width::Word, 0x8004, 0x0180_10b0),
    ] {
        m.cpu
            .mmu
            .store(&mut m.bus, address, 0x20, width, 123)
            .unwrap();
        assert_eq!(m.bus.read32(MFSR).unwrap(), expected);
        assert_eq!(m.bus.read32(MFAR).unwrap(), address);
    }
    m.cpu
        .mmu
        .store_doubleword(&mut m.bus, 0x8010, 0x20, [1, 2])
        .unwrap();
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0180_10c0);
    m.cpu
        .mmu
        .load(&mut m.bus, 0x8017, 0x20, Width::Byte)
        .unwrap();
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0180_1010); // underlying DRAM RD64
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x8017);
    m.bus.write32(0x9000, 99).unwrap(); // host image/debug writes do not emulate CPU requests
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x8017);
}

#[test]
fn memory_capture_status_and_address_unlock_independently_without_parity_invention() {
    let mut m = fixture();
    m.bus.write32(MFSR, 0x8180_2090).unwrap(); // diagnostic parity fault, lock both captures
    m.bus.write32(MFAR, 0x400).unwrap();
    put(&mut m, 0x8000, 0x20, 123);
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x8180_2090); // unlock status only
    put(&mut m, 0x8004, 0x20, 456);
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0180_10b0);
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x400); // unlock address
    get(&mut m, 0x8004, 0x20).unwrap();
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x8004);
    assert_eq!(m.bus.read32(IRQ).unwrap(), 0);
}

#[test]
fn cacheability_and_actual_cpu_mode_are_captured_independently_of_alternate_space_permissions() {
    let mut m = fixture();
    mapping(&mut m, 0x88e); // VA4000 -> PA8000, cacheable, RWX
    m.bus.set_cpu_supervisor(false);
    get(&mut m, 0x4000, 10).unwrap();
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0080_0810);
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x8000);
    // In a real CPU instruction, PSR.S drives S even for user ASI0a.
    m.cpu = Cpu::new(0);
    m.bus.write32(0, 0xc480_4140).unwrap(); // lda [g1] ASI0a,g2
    m.cpu.write_reg(1, 0x8000);
    m.step().unwrap();
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0180_0010);
}

#[test]
fn table_walks_and_dma_requests_update_memory_capture_with_their_physical_addresses() {
    let mut m = fixture();
    mapping(&mut m, 0); // invalid L3 PTE
    assert!(matches!(
        get(&mut m, 0x4000, 11),
        Err(MemoryError::Translation(_))
    ));
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0180_0010);
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x4010);
    m.bus.write32(IO, 1).unwrap();
    m.bus.write32(IO + 4, 0x1000).unwrap();
    m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
    m.bus.write32(0x10000, 0).unwrap();
    assert!(m.dma_read(5, 0xff00_0000, Width::Word).is_err());
    // Integrated microSPARC-II DMA keeps the CPU S pin and current PCR.BM
    // state. This fixture selected PCR_EN without BM.
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0100_0010);
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x10000);
    m.bus.write32(0x10000, 0x806).unwrap();
    m.dma_write(5, 0xff00_0002, Width::Half, 0x1234).unwrap();
    assert_eq!(m.bus.read32(MFSR).unwrap(), 0x0100_00a0);
    assert_eq!(m.bus.read32(MFAR).unwrap(), 0x8002);
}

#[test]
fn unsupported_operations_and_register_widths_do_not_clear_fault_captures() {
    let mut m = fixture();
    put(&mut m, EMPTY, 0x20, 0);
    let before = m.bus.devices().unwrap().io_control.clone();
    for error in [
        m.bus.read8(AFSR).unwrap_err(),
        m.bus.read_doubleword(AFSR).unwrap_err(),
        m.bus.swap32(MFSR, 0).unwrap_err(),
        m.bus.write16(MFAR, 0).unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop());
    }
    assert_eq!(m.bus.devices().unwrap().io_control, before);
    assert!(
        matches!(m.cpu.mmu.store_doubleword(&mut m.bus, EMPTY, 0x20, [1,2]), Err(MemoryError::Bus(e)) if e.kind.is_host_stop())
    );
    assert_eq!(m.bus.devices().unwrap().io_control, before);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}

#[test]
fn machine_resets_clear_fault_captures_and_module_error_latches_but_retain_ram() {
    for software in [false, true] {
        let mut m = fixture();
        put(&mut m, 0x8000, 0x20, 0x1234);
        put(&mut m, EMPTY, 0x20, 0);
        if software {
            m.bus.write32(RESET_BASE, 1).unwrap();
            assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
        } else {
            m.reset();
        }
        assert_eq!(m.bus.devices().unwrap().io_control, IoControl::default());
        assert_eq!(m.bus.read32(IRQ).unwrap(), 0);
        assert_eq!(m.bus.read32(AFSR).unwrap(), 0x0080_0000);
        assert_eq!(m.bus.read32(0x8000).unwrap(), 0x1234);
    }
}

#[test]
fn table_walk_and_probe_failures_keep_the_sbus_cause_and_virtual_fault_address() {
    for (ptp, cause, expected) in [
        (0x07cc_0001, BusErrorKind::SBusTimeout, 0x932),
        (0x0714_0001, BusErrorKind::SBusError, 0x532),
    ] {
        let mut m = fixture();
        mapping(&mut m, 0x80e);
        m.bus.write32(0x1000, ptp).unwrap();
        assert!(
            matches!(get(&mut m, 0x4000, 11), Err(MemoryError::Translation(f)) if f.level == 1 && f.bus_error == Some(cause))
        );
        assert_eq!(m.cpu.mmu.fault_status(), expected);
        assert_eq!(m.cpu.mmu.fault_address(), 0x4000);
        get(&mut m, 0x300, 4).unwrap();
        assert_eq!(get(&mut m, 0x4400, 3), Ok(0));
        assert_eq!(m.cpu.mmu.fault_status(), expected);
        assert_eq!(m.bus.read32(AFSR).unwrap(), 0x0080_0000);
        assert_eq!(m.bus.read32(IRQ).unwrap(), 0);
    }
}

#[test]
fn ss5_dma_capture_retains_cpu_supervisor_and_boot_mode() {
    for boot_mode in [false, true] {
        let mut m = fixture();
        assert!(m.cpu.mmu.boot_mode());
        if !boot_mode {
            put(&mut m, 0, 4, PCR_EN);
        }
        m.bus.write32(IO, 1).unwrap();
        m.bus.write32(IO + 4, 0x1000).unwrap();
        m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
        m.bus.write32(0x10000, 0x806).unwrap();
        m.bus.write32(0x8000, 0x1234_5678).unwrap();
        let baseline = AccessContext::host();
        m.bus.set_access_context(baseline);
        assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
        eprintln!(
            "after success boot={} psr={:08x}",
            m.cpu.mmu.boot_mode(),
            m.cpu.psr()
        );
        assert_eq!(m.bus.access_context(), baseline);
        assert_eq!(
            m.bus.read32(MFSR).unwrap() & 0x0180_1810,
            0x0100_0010 | (u32::from(boot_mode) << 12)
        );

        eprintln!(
            "before invalid boot={} psr={:08x} status={:08x}",
            m.cpu.mmu.boot_mode(),
            m.cpu.psr(),
            m.bus.read32(MFSR).unwrap()
        );
        m.bus.write32(IO + 0x18, 0xff00_0000).unwrap();
        m.bus.write32(0x10000, 0).unwrap();
        assert!(matches!(
            m.dma_read(5, 0xff00_0000, Width::Word),
            Err(helios::dma::DmaError::Fault { .. })
        ));
        assert_eq!(m.bus.access_context(), baseline);
        assert_eq!(
            m.bus.read32(MFSR).unwrap() & 0x0180_1810,
            0x0100_0010 | (u32::from(boot_mode) << 12)
        );
    }
}

#[test]
fn ss5_dma_capture_covers_write_protected_and_bypass_paths() {
    let mut m = fixture();
    assert!(m.cpu.mmu.boot_mode());
    m.bus.write32(IO, 1).unwrap();
    m.bus.write32(IO + 4, 0x1000).unwrap();
    m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
    m.bus.write32(0x10000, 0x902).unwrap();
    m.bus.write32(0x9000, 0xaaaa_bbbb).unwrap();
    assert!(matches!(
        m.dma_write(5, 0xff00_0000, Width::Word, 0),
        Err(helios::dma::DmaError::Fault { .. })
    ));
    assert_eq!(m.bus.read32(MFSR).unwrap() & 0x0180_1810, 0x0100_1010);

    m.bus.write32(IO + 0x1010, 1).unwrap(); // slot 0 bypass
    m.bus.write32(IO + 0x2000, 1 << 16).unwrap();
    let baseline = AccessContext::host();
    m.bus.set_access_context(baseline);
    assert_eq!(m.dma_read(0, 0x9000, Width::Word), Ok(0xaaaa_bbbb));
    assert_eq!(m.bus.access_context(), baseline);
    assert_eq!(m.bus.read32(MFSR).unwrap() & 0x0180_1810, 0x0100_1010);
}
