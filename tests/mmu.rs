use helios::bus::{Bus, BusErrorKind, MemoryBus};
use helios::mmu::{FaultType, MemoryError, Srmmu, Width, PCR_BM, PCR_EN, PCR_NF, PROM_BASE};

const VA: u32 = 0x1234_5000;
const LEAF: u64 = 0x4014;
const PTE: u32 = 0x0000_088e; // PA=0x8000, C=1, ACC=3 (RWX for both), ET=2

fn write_reg(mmu: &mut Srmmu, bus: &mut MemoryBus, reg: u32, value: u32) {
    mmu.store(bus, reg << 8, 4, Width::Word, value).unwrap();
}

fn fixture() -> (Srmmu, MemoryBus) {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x10000).unwrap();
    // VA indices: region 0x12, segment 0x0d, page 5, offset 0.
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (LEAF, PTE),
    ] {
        bus.write32(address, word).unwrap();
    }
    let mut mmu = Srmmu::default();
    write_reg(&mut mmu, &mut bus, 1, 0x100);
    write_reg(&mut mmu, &mut bus, 0, PCR_EN);
    (mmu, bus)
}

fn fault(result: Result<u32, MemoryError>, kind: FaultType, level: u8) {
    match result.unwrap_err() {
        MemoryError::Translation(error) => assert_eq!((error.kind, error.level), (kind, level)),
        other => panic!("expected MMU fault, got {other:?}"),
    }
}

#[test]
fn walk_translates_big_endian_lanes_and_updates_referenced_modified() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x8000, 0x89ab_cdef).unwrap();
    assert_eq!(mmu.load(&mut bus, VA + 1, 10, Width::Byte).unwrap(), 0xab);
    assert_eq!(bus.read32(LEAF).unwrap(), PTE | 0x20);
    assert_eq!(mmu.load(&mut bus, VA + 2, 9, Width::Half).unwrap(), 0xcdef);
    mmu.store(&mut bus, VA + 2, 11, Width::Half, 0x1234)
        .unwrap();
    assert_eq!(bus.read32(0x8000).unwrap(), 0x89ab_1234);
    assert_eq!(bus.read32(LEAF).unwrap(), PTE | 0x60);
    mmu.store(&mut bus, VA + 1, 10, Width::Byte, 0x56).unwrap();
    assert_eq!(bus.read32(0x8000).unwrap(), 0x8956_1234);
}

#[test]
fn every_acc_and_access_type_uses_the_architectural_permission_matrix() {
    // microSPARC-II table 9, independently expressed as user/supervisor rights.
    let rights = [
        ("r", "r"),
        ("rw", "rw"),
        ("rx", "rx"),
        ("rwx", "rwx"),
        ("x", "x"),
        ("r", "rw"),
        ("", "rx"),
        ("", "rwx"),
    ];
    for (acc, (user, supervisor)) in rights.into_iter().enumerate() {
        for (asi, write) in [
            (10, false),
            (11, false),
            (8, false),
            (9, false),
            (10, true),
            (11, true),
            (8, true),
            (9, true),
        ] {
            let (mut mmu, mut bus) = fixture();
            let pte = (PTE & !0x1c) | ((acc as u32) << 2);
            bus.write32(LEAF, pte).unwrap();
            bus.write32(0x8000, 0x1122_3344).unwrap();
            let mode = if asi & 1 != 0 { supervisor } else { user };
            let required = if asi < 10 {
                if write {
                    "xw"
                } else {
                    "x"
                }
            } else if write {
                "w"
            } else {
                "r"
            };
            let allowed = required.chars().all(|right| mode.contains(right));
            let result = if write {
                mmu.store(&mut bus, VA, asi, Width::Word, 0xfeed_face)
                    .map(|()| 0)
            } else {
                mmu.load(&mut bus, VA, asi, Width::Word)
            };
            assert_eq!(
                result.is_ok(),
                allowed,
                "ACC={acc}, ASI={asi}, write={write}"
            );
            if !allowed {
                fault(
                    result,
                    if mode.is_empty() {
                        FaultType::Privilege
                    } else {
                        FaultType::Protection
                    },
                    3,
                );
                assert_eq!(bus.read32(LEAF).unwrap(), pte);
                assert_eq!(bus.read32(0x8000).unwrap(), 0x1122_3344);
                assert_eq!(
                    (mmu.fault_status() >> 5) & 7,
                    u32::from(asi & 1)
                        | (if asi < 10 { 2 } else { 0 })
                        | (if write { 4 } else { 0 })
                );
            }
        }
    }
}

#[test]
fn region_segment_and_page_leaves_preserve_their_offsets() {
    for (leaf, pte, physical) in [
        (0x2048, 0x0020_008e, 0x0234_5024),
        (0x3034, 0x0001_008e, 0x0010_5024),
        (LEAF, PTE, 0x8024),
    ] {
        let (mut mmu, mut bus) = fixture();
        bus.write32(leaf, pte).unwrap();
        if physical >= 0x10000 {
            bus.map_ram(physical, 4).unwrap();
        }
        bus.write32(physical, 0x1020_3040).unwrap();
        assert_eq!(
            mmu.load(&mut bus, VA + 0x24, 11, Width::Word).unwrap(),
            0x1020_3040
        );
        assert_eq!(bus.read32(leaf).unwrap(), pte | 0x20);
    }
}

#[test]
fn disabled_mmu_and_bypass_drive_31_physical_bits() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x8000, 0x1122_3344).unwrap();
    assert_eq!(
        mmu.load(&mut bus, 0x8000_8000, 0x20, Width::Word).unwrap(),
        0x1122_3344
    );
    assert_eq!(bus.read32(LEAF).unwrap(), PTE);
    write_reg(&mut mmu, &mut bus, 0, 0);
    assert_eq!(
        mmu.load(&mut bus, 0x8000_8000, 9, Width::Word).unwrap(),
        0x1122_3344
    );
    assert!(matches!(
        mmu.load(&mut bus, 0x8000, 0x21, Width::Word),
        Err(MemoryError::UnsupportedAsi { .. })
    ));
}

#[test]
fn boot_mapping_applies_only_to_instruction_spaces_and_overrides_enable() {
    let (mut mmu, mut bus) = fixture();
    bus.map_rom(PROM_BASE, &0xaabb_ccddu32.to_be_bytes())
        .unwrap();
    bus.map_rom(0x7234_5000, &0xdead_beefu32.to_be_bytes())
        .unwrap();
    for control in [PCR_BM, PCR_BM | PCR_EN] {
        write_reg(&mut mmu, &mut bus, 0, control);
        assert_eq!(mmu.load(&mut bus, 0, 9, Width::Word).unwrap(), 0xaabb_ccdd);
        assert_eq!(mmu.load(&mut bus, VA, 8, Width::Word).unwrap(), 0xdead_beef);
    }
    // Data uses translation even while instruction accesses use boot mapping.
    bus.write32(0x8000, 7).unwrap();
    assert_eq!(mmu.load(&mut bus, VA, 11, Width::Word).unwrap(), 7);
    assert_eq!(mmu.load(&mut bus, 0, 0x20, Width::Word).unwrap(), 0);
}

#[test]
fn invalid_reserved_and_terminal_pointer_faults_report_the_walk_level() {
    for (address, level) in [(0x1000, 0), (0x2048, 1), (0x3034, 2), (LEAF, 3)] {
        for word in [0, 3] {
            let (mut mmu, mut bus) = fixture();
            bus.write32(address, word).unwrap();
            fault(
                mmu.load(&mut bus, VA, 11, Width::Word),
                if word == 0 {
                    FaultType::Invalid
                } else {
                    FaultType::Translation
                },
                level,
            );
            assert_eq!(mmu.fault_address(), VA);
        }
    }
    let (mut mmu, mut bus) = fixture();
    bus.write32(LEAF, 0x501).unwrap();
    fault(
        mmu.load(&mut bus, VA, 11, Width::Word),
        FaultType::Translation,
        3,
    );
}

#[test]
fn page_table_bus_failures_and_failed_rm_updates_are_translation_errors() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x2048, 0x2001).unwrap(); // L2 at unmapped 0x20000
    fault(
        mmu.load(&mut bus, VA, 11, Width::Word),
        FaultType::Translation,
        2,
    );
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x3034, 0x2001).unwrap();
    bus.map_rom(0x20014, &PTE.to_be_bytes()).unwrap();
    fault(
        mmu.load(&mut bus, VA, 11, Width::Word),
        FaultType::Translation,
        3,
    );
    assert_eq!(bus.read32(0x20014).unwrap(), PTE);
    assert_eq!(bus.read32(0x8000).unwrap(), 0);
}

#[test]
fn page_table_pointers_can_address_memory_above_256_mib() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x3034, 0x0300_0001).unwrap(); // L3 at 0x30000000
    bus.map_ram(0x3000_0000, 256).unwrap();
    bus.write32(0x3000_0014, PTE).unwrap();
    bus.write32(0x8000, 42).unwrap();
    assert_eq!(mmu.load(&mut bus, VA, 11, Width::Word).unwrap(), 42);
}

#[test]
fn fault_register_priority_and_destructive_read_aliases() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(LEAF, 0).unwrap();
    fault(
        mmu.load(&mut bus, VA, 9, Width::Word),
        FaultType::Invalid,
        3,
    );
    assert_eq!(mmu.fault_status(), 0x364); // instruction: SFAR not valid
    fault(
        mmu.load(&mut bus, VA, 11, Width::Word),
        FaultType::Invalid,
        3,
    );
    assert_eq!(mmu.fault_status(), 0x326); // higher class clears OW
    fault(
        mmu.load(&mut bus, VA + 4, 9, Width::Word),
        FaultType::Invalid,
        3,
    );
    assert_eq!((mmu.fault_status(), mmu.fault_address()), (0x326, VA));
    fault(
        mmu.load(&mut bus, VA + 8, 11, Width::Word),
        FaultType::Invalid,
        3,
    );
    assert_eq!((mmu.fault_status(), mmu.fault_address()), (0x327, VA + 8));
    bus.write32(0x3034, 3).unwrap();
    fault(
        mmu.load(&mut bus, VA + 12, 9, Width::Word),
        FaultType::Translation,
        2,
    );
    assert_eq!((mmu.fault_status(), mmu.fault_address()), (0x272, VA + 12));
    bus.write32(0x3034, 0x401).unwrap();
    fault(
        mmu.load(&mut bus, VA + 16, 11, Width::Word),
        FaultType::Invalid,
        3,
    );
    assert_eq!((mmu.fault_status(), mmu.fault_address()), (0x272, VA + 12));
    assert_eq!(mmu.load(&mut bus, 0x1400, 4, Width::Word).unwrap(), VA + 12);
    assert_eq!(mmu.load(&mut bus, 0x400, 4, Width::Word).unwrap(), VA + 12);
    assert_eq!(mmu.fault_address(), 0);
    assert_eq!(mmu.load(&mut bus, 0x1300, 4, Width::Word).unwrap(), 0x272);
    assert_eq!(mmu.load(&mut bus, 0x300, 4, Width::Word).unwrap(), 0x272);
    assert_eq!(mmu.fault_status(), 0);
}

#[test]
fn no_fault_suppresses_data_failures_without_bypassing_permissions() {
    let (mut mmu, mut bus) = fixture();
    write_reg(&mut mmu, &mut bus, 0, PCR_EN | PCR_NF);
    bus.write32(LEAF, 0x89a).unwrap(); // supervisor RX
    bus.write32(0x8000, 0x1234_5678).unwrap();
    assert_eq!(mmu.load(&mut bus, VA, 10, Width::Word).unwrap(), 0);
    mmu.store(&mut bus, VA, 11, Width::Word, 99).unwrap();
    assert_eq!(bus.read32(0x8000).unwrap(), 0x1234_5678);
    assert_eq!(bus.read32(LEAF).unwrap(), 0x89a);
    assert_ne!(mmu.fault_status(), 0);
    for asi in [8, 9] {
        bus.write32(LEAF, 0).unwrap();
        fault(
            mmu.load(&mut bus, VA, asi, Width::Word),
            FaultType::Invalid,
            3,
        );
    }
    assert_eq!(mmu.load(&mut bus, 0x50000, 0x20, Width::Word).unwrap(), 0);
}

#[test]
fn misalignment_precedes_walk_and_does_not_update_fault_or_pte_state() {
    let (mut mmu, mut bus) = fixture();
    write_reg(&mut mmu, &mut bus, 0, PCR_EN | PCR_NF);
    assert!(
        matches!(mmu.load(&mut bus, VA + 1, 11, Width::Word), Err(MemoryError::Bus(error)) if error.kind == BusErrorKind::Misaligned)
    );
    assert_eq!(mmu.fault_status(), 0);
    assert_eq!(bus.read32(LEAF).unwrap(), PTE);
}

#[test]
fn tlb_retains_translation_until_mmu_flush_and_cache_flush_is_separate() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x8000, 1).unwrap();
    bus.write32(0x9000, 2).unwrap();
    assert_eq!(mmu.load(&mut bus, VA, 11, Width::Word).unwrap(), 1);
    bus.write32(LEAF, 0x98e).unwrap();
    for (asi, address) in [(0x10, VA), (0x14, 0), (3, VA | 0xf00)] {
        mmu.store(&mut bus, address, asi, Width::Word, 0).unwrap();
        assert_eq!(mmu.load(&mut bus, VA, 11, Width::Word).unwrap(), 1);
    }
    for kind in 0..=4 {
        mmu.store(&mut bus, VA | (kind << 8), 3, Width::Word, 0)
            .unwrap();
        assert_eq!(mmu.load(&mut bus, VA, 11, Width::Word).unwrap(), 2);
    }
}

#[test]
fn contexts_are_tagged_and_supervisor_pages_can_be_global() {
    for acc in [3, 7] {
        let (mut mmu, mut bus) = fixture();
        bus.write32(LEAF, 0x882 | (acc << 2)).unwrap();
        for (address, word) in [
            (0x1004, 0x501),
            (0x5048, 0x601),
            (0x6034, 0x701),
            (0x7014, 0x98e),
            (0x8000, 1),
            (0x9000, 2),
        ] {
            bus.write32(address, word).unwrap();
        }
        assert_eq!(mmu.load(&mut bus, VA, 11, Width::Word).unwrap(), 1);
        write_reg(&mut mmu, &mut bus, 2, 1);
        assert_eq!(
            mmu.load(&mut bus, VA, 11, Width::Word).unwrap(),
            if acc == 7 { 1 } else { 2 }
        );
        assert_eq!(mmu.load(&mut bus, VA, 10, Width::Word).unwrap(), 2);
        write_reg(&mut mmu, &mut bus, 2, 0);
        if acc == 7 {
            fault(
                mmu.load(&mut bus, VA, 10, Width::Word),
                FaultType::Privilege,
                3,
            );
        } else {
            assert_eq!(mmu.load(&mut bus, VA, 10, Width::Word).unwrap(), 1);
        }
    }
}

#[test]
fn probes_return_requested_level_and_entire_probe_sets_r_without_checking_acc() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(LEAF, 0x89a).unwrap();
    for (kind, expected) in [(0, 0x89a), (1, 0x401), (2, 0x301), (3, 0x201), (4, 0x8ba)] {
        assert_eq!(
            mmu.load(&mut bus, VA | (kind << 8), 3, Width::Word)
                .unwrap(),
            expected
        );
    }
    assert_eq!(bus.read32(LEAF).unwrap(), 0x8ba);
    // A page probe must not return a segment leaf found earlier in the walk.
    bus.write32(0x3034, 0x8e).unwrap();
    assert_eq!(mmu.load(&mut bus, VA, 3, Width::Word).unwrap(), 0);
    assert_eq!(
        mmu.load(&mut bus, VA | 0x100, 3, Width::Word).unwrap(),
        0x8e
    );
    assert_eq!(mmu.load(&mut bus, VA | 0xf00, 3, Width::Word).unwrap(), 0);
    assert_eq!(mmu.fault_status(), 0);
}

#[test]
fn failed_probe_records_table_bus_error_without_delivering_exception() {
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x3034, 0x2001).unwrap();
    assert_eq!(mmu.load(&mut bus, VA | 0x400, 3, Width::Word).unwrap(), 0);
    assert_eq!(mmu.fault_status(), 0x332);
    assert_eq!(mmu.fault_address(), VA);
}

#[test]
fn register_masks_diagnostic_aliases_and_unimplemented_features() {
    let (mut mmu, mut bus) = fixture();
    write_reg(&mut mmu, &mut bus, 1, u32::MAX);
    write_reg(&mut mmu, &mut bus, 2, 0x123);
    assert_eq!(
        mmu.load(&mut bus, 0x100, 4, Width::Word).unwrap(),
        0x00ff_ffc0
    );
    assert_eq!(mmu.load(&mut bus, 0x200, 4, Width::Word).unwrap(), 0x23);
    assert_eq!(mmu.control(), 0x0400_0001);
    write_reg(&mut mmu, &mut bus, 0x13, u32::MAX);
    write_reg(&mut mmu, &mut bus, 0x14, VA);
    write_reg(&mut mmu, &mut bus, 3, 0);
    write_reg(&mut mmu, &mut bus, 4, 0);
    assert_eq!((mmu.fault_status(), mmu.fault_address()), (0x16fff, VA));
    let before = mmu.clone();
    assert!(matches!(
        mmu.store(&mut bus, 0, 4, Width::Word, 0x0080_0000),
        Err(MemoryError::UnsupportedControl(_))
    ));
    assert_eq!(mmu, before);
    assert!(matches!(
        mmu.load(&mut bus, 0x1100, 4, Width::Word),
        Err(MemoryError::UnsupportedRegister(_))
    ));
    assert!(matches!(
        mmu.load(&mut bus, 0, 7, Width::Word),
        Err(MemoryError::UnsupportedAsi { .. })
    ));
    assert!(matches!(
        mmu.load(&mut bus, 0, 4, Width::Byte),
        Err(MemoryError::UnsupportedAsi { .. })
    ));
    let (mut mmu, mut bus) = fixture();
    bus.write32(0x1000, PTE).unwrap();
    assert_eq!(
        mmu.load(&mut bus, VA, 11, Width::Word),
        Err(MemoryError::UnsupportedRootPte)
    );
}

#[test]
fn mxcc_stream_registers_copy_four_doublewords() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x4000).unwrap();
    for (index, address) in (0x1000u64..0x1020).step_by(8).enumerate() {
        bus.write_doubleword(
            address,
            [0x1000_0000 + index as u32, 0x2000_0000 + index as u32],
        )
        .unwrap();
    }
    let mut mmu = Srmmu::default();
    mmu.store_doubleword(&mut bus, 0x01c0_0100, 2, [0, 0x1000])
        .unwrap();
    for (index, address) in (0x01c0_0000u32..0x01c0_0020).step_by(8).enumerate() {
        assert_eq!(
            mmu.load_doubleword(&mut bus, address, 2).unwrap(),
            [0x1000_0000 + index as u32, 0x2000_0000 + index as u32]
        );
    }
    mmu.store_doubleword(&mut bus, 0x01c0_0200, 2, [0, 0x1100])
        .unwrap();
    for index in 0..4u32 {
        assert_eq!(
            bus.read_doubleword(0x1100 + u64::from(index) * 8).unwrap(),
            [0x1000_0000 + index, 0x2000_0000 + index]
        );
    }
}

#[test]
fn reference_mmu_contexts_are_not_aliased_to_eight_bits() {
    use helios::mmu::MmuConfig;
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x300_0000).unwrap();
    bus.write32(0x1000, 0x201).unwrap(); // context 0 -> L1 at 0x2000
    bus.write32(0x1400, 0x501).unwrap(); // context 0x100 -> L1 at 0x5000
    bus.write32(0x2000, 0x0010_008e).unwrap(); // 16 MiB leaf -> PA 0x01000000
    bus.write32(0x5000, 0x0020_008e).unwrap(); // 16 MiB leaf -> PA 0x02000000
    bus.write32(0x0100_0000, 0x1111_1111).unwrap();
    bus.write32(0x0200_0000, 0x2222_2222).unwrap();
    let mut mmu = Srmmu::reset_with(MmuConfig::ss20());
    write_reg(&mut mmu, &mut bus, 0, PCR_EN);
    write_reg(&mut mmu, &mut bus, 1, 0x100);
    assert_eq!(mmu.load(&mut bus, 0, 11, Width::Word).unwrap(), 0x1111_1111);
    write_reg(&mut mmu, &mut bus, 2, 0x100);
    assert_eq!(mmu.load(&mut bus, 0, 11, Width::Word).unwrap(), 0x2222_2222);
}

#[test]
fn reference_segment_probe_follows_descriptors_at_region_table_boundary() {
    use helios::mmu::MmuConfig;
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x6000).unwrap();
    // VA f5940000: L1 index f5, L2 index 25, L3 index 00.
    bus.write32(0x1000, 0x201).unwrap();
    bus.write32(0x23d4, 0x301).unwrap();
    bus.write32(0x3094, 0x401).unwrap();
    bus.write32(0x4000, 0x007b_789e).unwrap();
    // Past the end of the 1 KiB L1 table: not a descriptor in this walk.
    bus.write32(0x2468, 0x02cd_02ce).unwrap();
    let mut mmu = Srmmu::reset_with(MmuConfig::ss20());
    write_reg(&mut mmu, &mut bus, 0, PCR_EN);
    write_reg(&mut mmu, &mut bus, 1, 0x100);
    assert_eq!(
        mmu.load(&mut bus, 0xf594_0100, 3, Width::Word).unwrap(),
        0x401
    );
    assert_eq!(
        mmu.load(&mut bus, 0xf594_0000, 3, Width::Word).unwrap(),
        0x007b_789e
    );
    assert_eq!(
        mmu.load(&mut bus, 0xf594_0400, 3, Width::Word).unwrap(),
        0x007b_78be
    );
    assert_eq!(
        mmu.load(&mut bus, 0xf594_0100, 3, Width::Word).unwrap(),
        0x401
    );
}

#[test]
fn mxcc_stream_completion_reports_ready_and_preserves_36_bit_block_address() {
    use helios::mmu::MmuConfig;
    let mut bus = MemoryBus::default();
    bus.map_ram(0x1_0000_1000, 0x20).unwrap();
    bus.map_ram(0x2_0000_2000, 0x20).unwrap();
    for index in 0..8u64 {
        bus.write32(0x1_0000_1000 + index * 4, 0x1122_3300 + index as u32)
            .unwrap();
    }
    let mut mmu = Srmmu::reset_with(MmuConfig::ss20());
    // Reserved bits and byte offset are ignored; C and PA are retained.
    mmu.store_doubleword(&mut bus, 0x01c0_0100, 2, [0x7fff_fff1, 0x101f])
        .unwrap();
    assert_eq!(
        mmu.load_doubleword(&mut bus, 0x01c0_0100, 2).unwrap(),
        [0x8000_0011, 0x1000]
    );
    mmu.store_doubleword(&mut bus, 0x01c0_0200, 2, [0x12, 0x2000])
        .unwrap();
    assert_eq!(
        mmu.load_doubleword(&mut bus, 0x01c0_0200, 2).unwrap(),
        [0x8000_0012, 0x2000]
    );
    for index in 0..8u64 {
        assert_eq!(
            bus.read32(0x2_0000_2000 + index * 4).unwrap(),
            0x1122_3300 + index as u32
        );
    }
    // Software cannot manufacture completion by writing RDY on a failing transfer.
    assert!(mmu
        .store_doubleword(&mut bus, 0x01c0_0100, 2, [0x8000_0000, 0x5000])
        .is_err());
    assert_eq!(
        mmu.load_doubleword(&mut bus, 0x01c0_0100, 2).unwrap()[0] & 0x8000_0000,
        0
    );
}
