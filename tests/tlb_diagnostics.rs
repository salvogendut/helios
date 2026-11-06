use helios::bus::{Bus, BusErrorKind, MemoryBus};
use helios::mmu::{FaultType, MemoryError, Srmmu, Width, PCR_EN, PCR_NF};

fn put(m: &mut Srmmu, b: &mut MemoryBus, address: u32, asi: u8, value: u32) {
    m.store(b, address, asi, Width::Word, value).unwrap();
}
fn get(m: &mut Srmmu, b: &mut MemoryBus, address: u32, asi: u8) -> u32 {
    m.load(b, address, asi, Width::Word).unwrap()
}
fn fixture() -> (Srmmu, MemoryBus) {
    let mut b = MemoryBus::default();
    b.map_ram(0, 0x10000).unwrap();
    for context in 0..8 {
        b.write32(0x1000 + context * 4, 0x201).unwrap();
    }
    for (address, value) in [
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4010, 0x80e),
        (0x8000, 0x1234_5678),
    ] {
        b.write32(address, value).unwrap();
    }
    let mut m = Srmmu::default();
    put(&mut m, &mut b, 0x100, 4, 0x100);
    put(&mut m, &mut b, 0, 4, PCR_EN);
    (m, b)
}

#[test]
fn all_64_diagnostic_slots_have_independent_data_and_both_tag_halves() {
    let mut m = Srmmu::default();
    let mut b = MemoryBus::default();
    for index in 0..64 {
        put(&mut m, &mut b, index * 4, 6, u32::MAX ^ (index << 8));
        put(&mut m, &mut b, 0x100 + index * 4, 6, index << 4);
        put(
            &mut m,
            &mut b,
            0x300 + index * 4,
            6,
            0xaaaa_a000 | index << 4,
        );
    }
    for index in 0..64 {
        assert_eq!(
            get(&mut m, &mut b, index * 4, 6),
            0xe7ff_fffe ^ (index << 8)
        );
        assert_eq!(get(&mut m, &mut b, 0x100 + index * 4, 6), index << 4);
        assert_eq!(
            get(&mut m, &mut b, 0x300 + index * 4, 6),
            0xaaaa_a000 | index << 4
        );
    }
    put(&mut m, &mut b, 0, 6, u32::MAX);
    for (tag, expected) in [(2, 0xe7ff_fff1), (4, 0xe7ff_ff06), (0, 0xe7ff_fffe)] {
        put(&mut m, &mut b, 0x100, 6, tag);
        assert_eq!(get(&mut m, &mut b, 0, 6), expected);
    }
    put(&mut m, &mut b, 0x100, 6, u32::MAX);
    assert_eq!(get(&mut m, &mut b, 0x100, 6), 0x3ff);
}

#[test]
fn hardware_walks_and_modified_updates_are_visible_in_diagnostic_entries() {
    let (mut m, mut b) = fixture();
    assert_eq!(get(&mut m, &mut b, 0x4000, 11), 0x1234_5678);
    assert_eq!(get(&mut m, &mut b, 0, 6), 0xe000_082e);
    assert_eq!(get(&mut m, &mut b, 0x100, 6), 0x3f0);
    assert_eq!(get(&mut m, &mut b, 0x300, 6), 0x4008);
    put(&mut m, &mut b, 0x4000, 11, 0xabcd_ef01);
    assert_eq!(get(&mut m, &mut b, 0, 6), 0xe000_086e);
    assert_eq!(get(&mut m, &mut b, 0x100, 6), 0x3f1);
    assert_eq!(b.read32(0x4010).unwrap(), 0x86e);
    let counter = get(&mut m, &mut b, 0x1000, 4);
    put(&mut m, &mut b, 0x4000, 3, 0);
    assert_eq!(get(&mut m, &mut b, 0x300, 6), 0x4000);
    assert_eq!(get(&mut m, &mut b, 0x100, 6), 0x3f1);
    assert_eq!(get(&mut m, &mut b, 0, 6), 0xe000_086e);
    assert_eq!(get(&mut m, &mut b, 0x1000, 4), counter);
}

#[test]
fn injected_leaf_entries_translate_and_use_tag_permissions_and_context() {
    for (level, va, pa) in [
        (0, 0x1234_5000, 0x8000),
        (1, 0x1234_0000, 0x40000),
        (3, 0x1200_0000, 0x1000000),
    ] {
        let mut m = Srmmu::default();
        let mut b = MemoryBus::default();
        b.map_ram(pa, 8).unwrap();
        b.write32(pa, 0x1357_9bdf).unwrap();
        put(&mut m, &mut b, 0, 4, PCR_EN);
        put(&mut m, &mut b, 0x200, 4, 5);
        put(&mut m, &mut b, 0xfc, 6, (pa as u32 >> 4) | 2); // ACC=0; tag rights control hits.
        put(&mut m, &mut b, 0x1fc, 6, 0x3f1);
        put(&mut m, &mut b, 0x3fc, 6, va | 0x58 | level);
        assert_eq!(get(&mut m, &mut b, va, 8), 0x1357_9bdf);
        put(&mut m, &mut b, va + 4, 10, 0x2468_ace0);
        assert_eq!(b.read32(pa + 4).unwrap(), 0x2468_ace0);
        put(&mut m, &mut b, 0x1fc, 6, 0x121); // User/supervisor read only, M already set.
        assert!(
            matches!(m.store(&mut b, va, 11, Width::Word, 0), Err(MemoryError::Translation(f)) if f.kind == FaultType::Protection)
        );
        put(&mut m, &mut b, 0x1fc, 6, 0x079); // Supervisor RWX, global, modified.
        put(&mut m, &mut b, 0x200, 4, 6);
        assert_eq!(get(&mut m, &mut b, va, 11), 0x1357_9bdf);
        put(&mut m, &mut b, 0x200, 4, 5);
        assert!(
            matches!(m.load(&mut b, va, 10, Width::Word), Err(MemoryError::Translation(f)) if f.kind == FaultType::Privilege)
        );
        put(&mut m, &mut b, 0x3fc, 6, va | 0x50 | level); // Clear V, no tables exist.
        assert!(matches!(
            m.load(&mut b, va, 11, Width::Word),
            Err(MemoryError::Translation(_))
        ));
    }
}

#[test]
fn diagnostic_m_clear_requires_explicit_rewalk_support_before_a_store() {
    let (mut m, mut b) = fixture();
    get(&mut m, &mut b, 0x4000, 11);
    put(&mut m, &mut b, 0x100, 6, 0x3f0); // Diagnostic edits detach the table origin.
    let before = m.clone();
    assert_eq!(
        m.store(&mut b, 0x4000, 11, Width::Word, 0),
        Err(MemoryError::UnsupportedTlbModifiedUpdate)
    );
    assert_eq!(m, before);
    assert_eq!(b.read32(0x8000).unwrap(), 0x1234_5678);
    assert_eq!(b.read32(0x4010).unwrap(), 0x82e);
    put(&mut m, &mut b, 0x100, 6, 0x3f1);
    put(&mut m, &mut b, 0x4000, 11, 0xbeef_cafe);
    assert_eq!(b.read32(0x8000).unwrap(), 0xbeef_cafe);
}

#[test]
fn replacement_control_wrap_freeze_ptp_slots_and_io_partition_affect_real_fills() {
    for (control, slots, expected_counter) in [
        ((2 << 7) | 2, [2, 0, 1], 2),
        ((63 << 7) | 0x40 | 12, [12, 12, 12], 12),
        ((63 << 7) | 0x1c000, [3, 4, 5], 3),
        ((63 << 7) | 0x2000 | 47, [47, 0, 1], 2),
    ] {
        let (mut m, mut b) = fixture();
        put(&mut m, &mut b, 0x1000, 4, control);
        for (context, slot) in slots.into_iter().enumerate() {
            put(&mut m, &mut b, 0x200, 4, context as u32);
            get(&mut m, &mut b, 0x4000, 11);
            assert_eq!(
                get(&mut m, &mut b, 0x300 + slot * 4, 6),
                0x4008 | ((context as u32) << 4)
            );
        }
        assert_eq!(get(&mut m, &mut b, 0x1000, 4) & 63, expected_counter);
    }
    let (mut m, mut b) = fixture();
    put(&mut m, &mut b, 0x1000, 4, u32::MAX);
    assert_eq!(get(&mut m, &mut b, 0x10fc, 4), 0x01f1_ffff);
}

#[test]
fn diagnostic_reserved_addresses_widths_and_unsupported_lookup_are_precise() {
    let (mut m, mut b) = fixture();
    put(&mut m, &mut b, 0, 4, PCR_EN | PCR_NF);
    let before = m.clone();
    for address in [0x200, 0x2fc, 0x400, 0xffff_fffc] {
        assert_eq!(
            m.load(&mut b, address, 6, Width::Word),
            Err(MemoryError::UnsupportedRegister(address))
        );
        assert_eq!(
            m.store(&mut b, address, 6, Width::Word, 0),
            Err(MemoryError::UnsupportedRegister(address))
        );
    }
    for width in [Width::Byte, Width::Half] {
        assert!(matches!(
            m.store(&mut b, 0, 6, width, 0),
            Err(MemoryError::UnsupportedAsi { .. })
        ));
    }
    assert!(
        matches!(m.store(&mut b, 2, 6, Width::Word, 0), Err(MemoryError::Bus(e)) if e.kind == BusErrorKind::Misaligned)
    );
    assert!(matches!(
        m.store_doubleword(&mut b, 0, 6, [0; 2]),
        Err(MemoryError::UnsupportedOperation { .. })
    ));
    assert!(matches!(
        m.swap(&mut b, 0, 6, 0),
        Err(MemoryError::UnsupportedOperation { .. })
    ));
    assert_eq!(m, before);
    put(&mut m, &mut b, 0x100, 6, 2);
    put(&mut m, &mut b, 0x300, 6, 0xf); // Active PTP may affect any table walk.
    let before = m.clone();
    assert!(matches!(
        m.load(&mut b, 0x4000, 11, Width::Word),
        Err(MemoryError::UnsupportedTlbDiagnostic { .. })
    ));
    assert_eq!(m, before);
    put(&mut m, &mut b, 0, 3, 0);
    assert_eq!(get(&mut m, &mut b, 0x4000, 11), 0x1234_5678);
}
