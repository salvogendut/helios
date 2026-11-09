use helios::bus::{Bus, BusErrorKind};
use helios::cpu::Step;
use helios::dma::{DmaError, DmaFault};
use helios::machine::Machine;
use helios::mmu::{Width, PCR_EN, PCR_NF};
use helios::system_bus::{IO_CONTROL_BASE as IO, RESET_BASE, SYSTEM_INTERRUPT_BASE};

fn fixture() -> Machine {
    let mut m = Machine::sun4m(0x20000, &0x0100_0000u32.to_be_bytes()).unwrap();
    m.bus.write32(IO, 1).unwrap();
    m.bus.write32(IO + 4, 0x1000).unwrap(); // 16 KiB table at PA 0x10000
    m.bus.write32(IO + 0x2000, 0x003f_0000).unwrap();
    m.bus.write32(0x10000, 0x806).unwrap(); // ff000000 -> 8000, writable
    m.bus.write32(0x10004, 0x902).unwrap(); // ff001000 -> 9000, read-only
    m.bus.write32(0x8000, 0x1234_5678).unwrap();
    m.bus.write32(0x9000, 0xabcd_ef01).unwrap();
    m
}

fn put(m: &mut Machine, address: u32, asi: u8, value: u32) {
    m.cpu
        .mmu
        .store(&mut m.bus, address, asi, Width::Word, value)
        .unwrap();
}
fn get(m: &mut Machine, address: u32, asi: u8) -> u32 {
    m.cpu
        .mmu
        .load(&mut m.bus, address, asi, Width::Word)
        .unwrap()
}
fn fault(address: u32, kind: DmaFault) -> DmaError {
    DmaError::Fault { address, kind }
}

#[test]
fn translated_dma_preserves_word_patterns_byte_lanes_and_memory_ioptes() {
    let mut m = fixture();
    let cpu = m.cpu.clone();
    let timers = m.bus.devices().unwrap().timers.clone();
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
    assert_eq!(m.dma_read(5, 0xff00_0001, Width::Byte), Ok(0x34));
    assert_eq!(m.dma_read(5, 0xff00_0002, Width::Half), Ok(0x5678));
    m.dma_write(5, 0xff00_0001, Width::Byte, 0xaa).unwrap();
    m.dma_write(5, 0xff00_0002, Width::Half, 0xbbcc).unwrap();
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0x12aa_bbcc);
    m.dma_write(5, 0xff00_0ffc, Width::Word, 0xdead_beef)
        .unwrap();
    assert_eq!(m.bus.read32(0x8ffc).unwrap(), 0xdead_beef);
    assert_eq!(m.bus.read32(0x10000).unwrap(), 0x806); // no CPU R/M writeback
    assert_eq!(
        (m.cpu.pc(), m.cpu.npc(), m.cpu.psr()),
        (cpu.pc(), cpu.npc(), cpu.psr())
    );
    assert_eq!(m.bus.devices().unwrap().timers, timers);
    assert_eq!(get(&mut m, 0, 6), 0xe000_0806);
    assert_eq!(get(&mut m, 0x100, 6), 4); // IO tag
    assert_eq!(get(&mut m, 0x300, 6), 0xff00_0008); // context-independent 4 KiB tag
}

#[test]
fn all_eight_ranges_check_the_lower_boundary_and_align_the_table_base() {
    // Expected table addresses derived from the hardware concatenation, using
    // deliberately misaligned IBAR=0x3fc00 (PA=0x3fc000) for the larger tables.
    for (range, start, first_pte, last_pte) in [
        (0, 0xff00_0000, 0x3fc000, 0x3ffffc),
        (1, 0xfe00_0000, 0x3f8000, 0x3ffffc),
        (2, 0xfc00_0000, 0x3f0000, 0x3ffffc),
        (3, 0xf800_0000, 0x3e0000, 0x3ffffc),
        (4, 0xf000_0000, 0x3c0000, 0x3ffffc),
        (5, 0xe000_0000, 0x380000, 0x3ffffc),
        (6, 0xc000_0000, 0x300000, 0x3ffffc),
        (7, 0x8000_0000, 0x200000, 0x3ffffc),
    ] {
        let mut m = Machine::sun4m(0x400000, &[1, 0, 0, 0]).unwrap();
        m.bus.write32(IO, 1 | range << 2).unwrap();
        m.bus.write32(IO + 4, 0x3fc00).unwrap();
        m.bus.write32(IO + 0x2000, 1 << 21).unwrap();
        m.bus.write32(first_pte, 0x806).unwrap();
        m.bus.write32(last_pte, 0x906).unwrap();
        m.bus.write32(0x8000, 0x1122_3344).unwrap();
        m.bus.write32(0x9ffc, 0xaabb_ccdd).unwrap();
        assert_eq!(m.dma_read(5, start, Width::Word), Ok(0x1122_3344));
        assert_eq!(m.dma_read(5, u32::MAX, Width::Byte), Ok(0xdd));
        assert_eq!(
            m.dma_read(5, start - 4, Width::Word),
            Err(fault(start - 4, DmaFault::OutOfRange))
        );
    }
}

#[test]
fn denied_dma_returns_to_the_master_without_cpu_faults_interrupts_or_ram_changes() {
    let mut m = fixture();
    put(&mut m, 0, 4, PCR_NF); // CPU no-fault mode cannot suppress a DMA error
    m.bus.write32(SYSTEM_INTERRUPT_BASE + 8, u32::MAX).unwrap();
    for (address, kind) in [
        (0xfeff_fffc, DmaFault::OutOfRange),
        (0xff00_1000, DmaFault::WriteProtected),
        (0xff00_2000, DmaFault::InvalidPte),
        (0xff00_0001, DmaFault::Misaligned),
    ] {
        assert_eq!(
            m.dma_write(5, address, Width::Word, 0),
            Err(fault(address, kind))
        );
    }
    assert_eq!(m.dma_read(5, 0xff00_1000, Width::Word), Ok(0xabcd_ef01));
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0x1234_5678);
    assert_eq!(m.bus.read32(0x9000).unwrap(), 0xabcd_ef01);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert_eq!(m.cpu.mmu.fault_address(), 0);
    assert!(m.cpu.error_state().is_none());
    assert_eq!(m.bus.interrupt_levels(), 0);
    assert_eq!(m.bus.read32(SYSTEM_INTERRUPT_BASE).unwrap(), 0);
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0, 4));
}

#[test]
fn all_six_arbitration_enables_gate_dma_and_slots_four_and_five_cannot_bypass() {
    let mut m = fixture();
    for master in 0..6 {
        m.bus.write32(IO + 0x2000, 1 << (16 + master)).unwrap();
        for candidate in 0..6 {
            let result = m.dma_read(candidate, 0xff00_0000, Width::Word);
            if candidate == master {
                assert_eq!(result, Ok(0x1234_5678));
            } else {
                assert_eq!(result, Err(DmaError::NotGranted { master: candidate }));
            }
        }
    }
    m.bus.write32(IO + 0x2000, 0x003f_0000).unwrap();
    for slot in 0..5 {
        m.bus.write32(IO + 0x1010 + slot * 4, 1).unwrap();
    }
    for master in 0..4 {
        assert_eq!(m.dma_read(master, 0x8000, Width::Word), Ok(0x1234_5678));
    }
    for master in [4, 5] {
        assert_eq!(
            m.dma_read(master, 0x8000, Width::Word),
            Err(fault(0x8000, DmaFault::OutOfRange))
        );
    }
    assert!(matches!(
        m.dma_read(6, 0x8000, Width::Word),
        Err(DmaError::Unsupported(_))
    ));
}

#[test]
fn bypass_is_slot_specific_low_address_only_and_restricted_to_main_memory() {
    let mut m = fixture();
    m.bus.write32(IO + 0x1010, 1).unwrap();
    m.dma_write(0, 0x8000, Width::Word, 0x2468_ace0).unwrap();
    assert_eq!(m.dma_read(0, 0xff00_0000, Width::Word), Ok(0x2468_ace0)); // high VA still translates
    m.bus.write32(IO, 0).unwrap();
    assert_eq!(m.dma_read(0, 0x8000, Width::Word), Ok(0x2468_ace0));
    assert!(matches!(
        m.dma_read(1, 0x8000, Width::Word),
        Err(DmaError::Unsupported(_))
    ));
    assert!(matches!(
        m.dma_read(0, 0x4000_8000, Width::Word),
        Err(DmaError::Unsupported(_))
    ));
    assert_eq!(
        m.dma_read(0, 0x1000_0000, Width::Word),
        Err(fault(0x1000_0000, DmaFault::InvalidBypassAddress))
    );
    m.bus.write32(IO + 0x1010, 0x10001).unwrap();
    assert_eq!(
        m.dma_read(0, 0x8000, Width::Word),
        Err(fault(0x8000, DmaFault::InvalidBypassAddress))
    );
}

#[test]
fn io_flushes_preserve_cpu_entries_and_cpu_entire_flush_clears_the_shared_tlb() {
    let mut m = fixture();
    put(&mut m, 0, 4, PCR_EN);
    // CPU and I/O can map the same VA to different physical pages.
    put(&mut m, 0xfc, 6, 0x602);
    put(&mut m, 0x1fc, 6, 0x120);
    put(&mut m, 0x3fc, 6, 0xff00_0008);
    m.bus.write32(0x6000, 0x8765_4321).unwrap();
    assert_eq!(get(&mut m, 0xff00_0000, 11), 0x8765_4321);
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
    assert_eq!(m.dma_read(5, 0xff00_1000, Width::Word), Ok(0xabcd_ef01));
    m.bus.write32(0x10000, 0x906).unwrap();
    m.bus.write32(0x10004, 0x806).unwrap();
    // Neither RAM page-table edits nor context changes flush I/O entries.
    put(&mut m, 0x200, 4, 7);
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
    put(&mut m, 0x200, 4, 0);
    put(&mut m, (IO + 0x18) as u32, 0x20, 0xff00_0123);
    assert_eq!(get(&mut m, 0x300, 6), 0xff00_0000); // invalidated before diagnostic read
    assert_eq!(get(&mut m, 0x304, 6), 0xff00_1008);
    assert_eq!(get(&mut m, 0xff00_0000, 11), 0x8765_4321);
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0xabcd_ef01));
    assert_eq!(m.dma_read(5, 0xff00_1000, Width::Word), Ok(0xabcd_ef01));
    m.bus.write32(IO + 0x14, 123).unwrap();
    assert_eq!(get(&mut m, 0x304, 6) & 8, 0);
    assert_eq!(get(&mut m, 0xff00_0000, 11), 0x8765_4321);
    assert_eq!(m.dma_read(5, 0xff00_1000, Width::Word), Ok(0x1234_5678));
    put(&mut m, 0x400, 3, 0);
    assert_eq!(get(&mut m, 0x3fc, 6) & 8, 0);
    assert_eq!(get(&mut m, 0x30c, 6) & 8, 0);
    m.bus.write32(0x10004, 0).unwrap();
    assert_eq!(
        m.dma_read(5, 0xff00_1000, Width::Word),
        Err(fault(0xff00_1000, DmaFault::InvalidPte))
    );
}

#[test]
fn base_and_enable_writes_retain_cached_io_mappings_until_an_explicit_flush() {
    let mut m = fixture();
    m.bus.write32(0x10000, 0xf800_08ff).unwrap(); // reserved IOPTE bits ignored
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
    assert_eq!(get(&mut m, 0, 6), 0xe000_0806);
    assert_eq!(m.bus.read32(0x10000).unwrap(), 0xf800_08ff);
    m.bus.write32(IO + 4, 0x1400).unwrap();
    m.bus.write32(0x14000, 0x906).unwrap();
    m.bus.write32(IO, 0).unwrap();
    assert!(matches!(
        m.dma_read(5, 0xff00_0000, Width::Word),
        Err(DmaError::Unsupported(_))
    ));
    m.bus.write32(IO, 1).unwrap();
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
    m.bus.write32(IO + 0x18, 0xff00_0000).unwrap();
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0xabcd_ef01));
}

#[test]
fn cpu_and_io_fills_use_separate_partitions_with_the_same_frozen_counter() {
    let mut m = fixture();
    for (address, value) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4010, 0x60e),
        (0x6000, 0x2468_ace0),
    ] {
        m.bus.write32(address, value).unwrap();
    }
    put(&mut m, 0x100, 4, 0x100);
    put(&mut m, 0, 4, PCR_EN);
    let trcr = (47 << 7) | (1 << 13) | (1 << 6) | 5;
    put(&mut m, 0x1000, 4, trcr);
    assert_eq!(get(&mut m, 0x4000, 11), 0x2468_ace0);
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
    assert_eq!(get(&mut m, 0x314, 6), 0x4008); // CPU slot 5
    assert_eq!(get(&mut m, 0x3d4, 6), 0xff00_0008); // I/O slot 53
    assert_eq!(get(&mut m, 0x1000, 4), trcr);
    m.bus.write32(0x4010, 0).unwrap();
    m.bus.write32(0x10000, 0).unwrap();
    assert_eq!(get(&mut m, 0x4000, 11), 0x2468_ace0);
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
}

#[test]
fn io_partition_and_software_injected_ioptes_use_the_live_shared_tlb() {
    let mut m = fixture();
    put(&mut m, 0x1000, 4, (47 << 7) | (1 << 13)); // IL: I/O fills in 48..63
    for page in 0..16 {
        m.bus.write32(0x10000 + page * 4, 0x806).unwrap();
        assert_eq!(
            m.dma_read(5, 0xff00_0000 + page as u32 * 4096, Width::Word),
            Ok(0x1234_5678)
        );
    }
    for slot in 0..48 {
        assert_eq!(get(&mut m, 0x300 + slot * 4, 6), 0);
    }
    for slot in 48..64 {
        assert_eq!(
            get(&mut m, 0x300 + slot * 4, 6),
            0xff00_0008 + (slot - 48) * 4096
        );
    }
    // PTE W controls I/O writes; CPU context and CPU protection bits are irrelevant.
    put(&mut m, 0xc0, 6, 0x902);
    put(&mut m, 0x1c0, 6, 4);
    put(&mut m, 0x3c0, 6, 0xff00_0ab8);
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0xabcd_ef01));
    assert_eq!(
        m.dma_write(5, 0xff00_0000, Width::Word, 0),
        Err(fault(0xff00_0000, DmaFault::WriteProtected))
    );
    put(&mut m, 0xc0, 6, 0x906);
    m.dma_write(5, 0xff00_0000, Width::Word, 0xcafe_babe)
        .unwrap();
    assert_eq!(m.bus.read32(0x9000).unwrap(), 0xcafe_babe);
}

#[test]
fn physical_errors_and_unimplemented_targets_remain_explicit() {
    let mut m = fixture();
    m.bus.write32(IO + 4, 0x8000).unwrap(); // unmapped table PA=0x80000 in strict harness
    assert!(
        matches!(m.dma_read(5, 0xff00_0000, Width::Word), Err(DmaError::Bus(e)) if e.address == 0x80000 && e.kind == BusErrorKind::Unmapped)
    );
    m.bus.write32(IO + 4, 0x1000).unwrap();
    m.bus.write32(0x10000, 0x0000_8006).unwrap(); // valid PTE, unmapped target
    assert!(
        matches!(m.dma_write(5, 0xff00_0000, Width::Word, 0), Err(DmaError::Bus(e)) if e.address == 0x80000 && e.kind == BusErrorKind::Unmapped)
    );
    m.bus.write32(IO + 0x14, 0).unwrap();
    m.bus.write32(0x10000, 0x0711_0006).unwrap(); // serial device cannot be consumed by DMA yet
    let serial = m.bus.devices().unwrap().serial.clone();
    assert_eq!(
        m.dma_read(5, 0xff00_0000, Width::Byte),
        Err(DmaError::Unsupported("non-DRAM DVMA target"))
    );
    assert_eq!(m.bus.devices().unwrap().serial, serial);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}

#[test]
fn reset_discards_io_mappings_configuration_and_pending_flush_commands() {
    for software in [false, true] {
        let mut m = fixture();
        m.dma_read(5, 0xff00_0000, Width::Word).unwrap();
        m.bus.write32(IO + 0x18, 0xff00_1000).unwrap();
        if software {
            m.bus.write32(RESET_BASE, 1).unwrap();
            assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
        } else {
            m.reset();
        }
        assert_eq!(get(&mut m, 0x300, 6), 0);
        assert_eq!(m.bus.take_io_tlb_flush(), None);
        assert_eq!(m.bus.read32(IO).unwrap(), 0x0400_0000);
        assert_eq!(
            m.dma_read(5, 0xff00_0000, Width::Word),
            Err(DmaError::NotGranted { master: 5 })
        );
        assert_eq!(m.bus.read32(0x8000).unwrap(), 0x1234_5678);
    }
}

#[test]
fn flush_ports_are_write_only_and_wrong_widths_do_not_enqueue_work() {
    let mut m = fixture();
    for offset in [0x14, 0x18] {
        assert!(m.bus.read32(IO + offset).unwrap_err().kind.is_host_stop());
        assert!(m
            .bus
            .write8(IO + offset, 0)
            .unwrap_err()
            .kind
            .is_host_stop());
        assert!(m
            .bus
            .swap32(IO + offset, 0)
            .unwrap_err()
            .kind
            .is_host_stop());
        assert_eq!(m.bus.take_io_tlb_flush(), None);
    }
}

#[test]
fn ss20_iommu_uses_iopte_physical_bits_not_cpu_pte_type_bits() {
    let io = helios::machine::MachineProfile::Ss20.layout().io_control;
    let mut m = Machine::ss20(16 * 1024 * 1024, &0x0100_0000u32.to_be_bytes()).unwrap();
    m.bus.write32(io, 1).unwrap();
    m.bus.write32(io + 4, 0x1000).unwrap(); // 16 KiB table at PA 0x10000
    m.bus.write32(io + 0x1008, 1 << 20).unwrap(); // onboard SBus master 5
    m.bus.write32(0x10000, 0x806).unwrap(); // ff000000 -> PA 0x8000
    m.bus.write32(0x8000, 0x1234_5678).unwrap();
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1234_5678));
}
