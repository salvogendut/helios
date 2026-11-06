use helios::bus::{Bus, BusErrorKind, MemoryBus};
use helios::cpu::{FaultKind, Step, PSR_ET};
use helios::machine::Machine;
use helios::mmu::{MemoryError, Srmmu, Width, PCR_BM, PCR_EN, PCR_NF};
use helios::system_bus::RESET_BASE;

fn put(mmu: &mut Srmmu, bus: &mut impl Bus, address: u32, asi: u8, value: u32) {
    mmu.store(bus, address, asi, Width::Word, value).unwrap();
}

fn get(mmu: &mut Srmmu, bus: &mut impl Bus, address: u32, asi: u8) -> u32 {
    mmu.load(bus, address, asi, Width::Word).unwrap()
}

fn alternate(op3: u32, rd: u32, rs1: u32, asi: u8) -> u32 {
    0xc000_0000 | (rd << 25) | (op3 << 19) | (rs1 << 14) | (u32::from(asi) << 5)
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

#[test]
fn all_tags_decode_their_own_index_and_mask_reserved_fields() {
    let mut mmu = Srmmu::reset();
    let mut bus = MemoryBus::default(); // Diagnostic accesses need no physical map.
    for (asi, shift, size, mask) in [
        (0x0c, 5, 0x4000, 0xffff_cfff),
        (0x0e, 4, 0x2000, 0xffff_efff),
    ] {
        for index in 0..512 {
            let address = index << shift;
            assert_eq!(get(&mut mmu, &mut bus, address, asi), 0);
            // Retain distinct context/ACC fields and a valid VA tag per entry.
            put(&mut mmu, &mut bus, address, asi, 0xffff_ffff ^ (index << 1));
        }
        for index in 0..512 {
            let expected = (0xffff_ffff ^ (index << 1)) & mask;
            for alias in [
                index << shift,
                (index << shift) + (1 << shift) - 4,
                0xff00_0000 | size | (index << shift),
            ] {
                assert_eq!(get(&mut mmu, &mut bus, alias, asi), expected);
            }
        }
    }
    // The I-cache index ignores bit 4; the D-cache index includes it.
    put(&mut mmu, &mut bus, 0x10, 0x0c, 0x1234_0001);
    assert_eq!(get(&mut mmu, &mut bus, 0, 0x0c), 0x1234_0001);
    assert_eq!(get(&mut mmu, &mut bus, 0, 0x0e), 0xffff_efff);
    assert_eq!(get(&mut mmu, &mut bus, 0x10, 0x0e), 0xffff_effd);
}

#[test]
fn data_arrays_have_independent_sizes_and_big_endian_lanes() {
    let mut mmu = Srmmu::default();
    let mut bus = MemoryBus::default();
    for (asi, size) in [(0x0d, 0x4000), (0x0f, 0x2000)] {
        for address in (0..size).step_by(4) {
            put(&mut mmu, &mut bus, address, asi, 0x8bad_0000 | address);
        }
        for address in (0..size).step_by(4) {
            assert_eq!(
                get(&mut mmu, &mut bus, 0xffff_c000 | size | address, asi),
                0x8bad_0000 | address
            );
        }
    }
    put(&mut mmu, &mut bus, 0, 0x0f, 0x89ab_cdef);
    for (offset, expected) in [0x89, 0xab, 0xcd, 0xef].into_iter().enumerate() {
        assert_eq!(
            mmu.load(&mut bus, offset as u32, 0x0f, Width::Byte)
                .unwrap(),
            expected
        );
    }
    assert_eq!(mmu.load(&mut bus, 0, 0x0f, Width::Half).unwrap(), 0x89ab);
    assert_eq!(mmu.load(&mut bus, 2, 0x0f, Width::Half).unwrap(), 0xcdef);
    for offset in 0..4 {
        mmu.store(&mut bus, offset, 0x0f, Width::Byte, 0x100 | (0x10 + offset))
            .unwrap();
    }
    assert_eq!(get(&mut mmu, &mut bus, 0, 0x0f), 0x1011_1213);
    mmu.store(&mut bus, 0, 0x0f, Width::Half, 0x1abcd).unwrap();
    mmu.store(&mut bus, 2, 0x0f, Width::Half, 0x2ef01).unwrap();
    assert_eq!(get(&mut mmu, &mut bus, 0, 0x0f), 0xabcd_ef01);
    assert_eq!(get(&mut mmu, &mut bus, 0, 0x0d), 0x8bad_0000);
    assert_eq!(get(&mut mmu, &mut bus, 0, 0x0e), 0); // Data writes do not validate tags.
}

#[test]
fn doublewords_at_diagnostic_boundaries_preserve_word_order_and_atomicity() {
    let mut mmu = Srmmu::default();
    let mut bus = MemoryBus::default();
    for address in [0, 0x1ff8, 0xffff_fff8] {
        mmu.store_doubleword(&mut bus, address, 0x0f, [0x0123_4567, 0x89ab_cdef])
            .unwrap();
        assert_eq!(
            mmu.load_doubleword(&mut bus, address, 0x0f).unwrap(),
            [0x0123_4567, 0x89ab_cdef]
        );
        assert_eq!(get(&mut mmu, &mut bus, address & 0x1fff, 0x0f), 0x0123_4567);
        assert_eq!(
            get(&mut mmu, &mut bus, (address & 0x1fff) + 4, 0x0f),
            0x89ab_cdef
        );
    }
    let before = mmu.clone();
    assert!(
        matches!(mmu.store_doubleword(&mut bus, 0x1ffc, 0x0f, [1, 2]), Err(MemoryError::Bus(e)) if e.kind == BusErrorKind::Misaligned)
    );
    assert_eq!(mmu, before);
}

#[test]
fn unsupported_widths_and_atomics_leave_diagnostics_and_fault_registers_intact() {
    let mut mmu = Srmmu::reset();
    let mut bus = MemoryBus::default();
    put(&mut mmu, &mut bus, 0, 4, PCR_EN | PCR_NF | PCR_BM);
    put(&mut mmu, &mut bus, 0x1300, 4, 0x326);
    put(&mut mmu, &mut bus, 0x1400, 4, 0xdead_beef);
    for asi in 0x0c..=0x0f {
        put(&mut mmu, &mut bus, 0, asi, u32::MAX);
    }
    let before = mmu.clone();
    for asi in 0x0c..=0x14 {
        if asi != 0x0f {
            for width in [Width::Byte, Width::Half] {
                assert!(matches!(
                    mmu.load(&mut bus, 0, asi, width),
                    Err(MemoryError::UnsupportedAsi { write: false, .. })
                ));
                assert!(matches!(
                    mmu.store(&mut bus, 0, asi, width, 0),
                    Err(MemoryError::UnsupportedAsi { write: true, .. })
                ));
            }
            assert!(matches!(
                mmu.load_doubleword(&mut bus, 0, asi),
                Err(MemoryError::UnsupportedOperation { .. })
            ));
            assert!(matches!(
                mmu.store_doubleword(&mut bus, 0, asi, [0, 0]),
                Err(MemoryError::UnsupportedOperation { .. })
            ));
        }
        assert!(matches!(
            mmu.swap(&mut bus, 0, asi, 0),
            Err(MemoryError::UnsupportedOperation { .. })
        ));
        assert!(matches!(
            mmu.load_store_byte(&mut bus, 0, asi),
            Err(MemoryError::UnsupportedOperation { .. })
        ));
        assert_eq!(mmu, before);
    }
    for asi in [0x0c, 0x0d, 0x0e, 0x0f, 0x10] {
        assert!(
            matches!(mmu.store(&mut bus, 2, asi, Width::Word, 0), Err(MemoryError::Bus(e)) if e.kind == BusErrorKind::Misaligned)
        );
    }
    assert_eq!(mmu, before);
}

#[test]
fn flush_scopes_check_address_context_and_supervisor_access_bits_per_index() {
    let address = 0x1234_0080;
    let mut mmu = Srmmu::default();
    let mut bus = MemoryBus::default();
    put(&mut mmu, &mut bus, 0x200, 4, 5);
    for scope in 0..5 {
        for acc in 0..8 {
            for context in [5, 6] {
                for (distance, delta) in [0, 0x4000, 0x40000, 0x1000000].into_iter().enumerate() {
                    let tag = (address ^ delta) & 0xffff_c000 | (context << 4) | (acc << 1) | 1;
                    for asi in [0x0c, 0x0e] {
                        put(&mut mmu, &mut bus, address, asi, tag);
                        put(&mut mmu, &mut bus, address + 32, asi, tag);
                    }
                    put(&mut mmu, &mut bus, address, 0x0d, 0xfeed_face);
                    put(&mut mmu, &mut bus, address, 0x0f, 0xdead_beef);
                    put(&mut mmu, &mut bus, address, 0x10 + scope, u32::MAX);
                    let invalidated = match scope {
                        0..=2 => (acc >= 6 || context == 5) && distance <= usize::from(scope),
                        3 => acc < 6 && context == 5,
                        4 => acc < 6,
                        _ => unreachable!(),
                    };
                    for asi in [0x0c, 0x0e] {
                        assert_eq!(get(&mut mmu, &mut bus, address, asi), tag & !u32::from(invalidated), "asi={asi:x} scope={scope} acc={acc} context={context} distance={distance}");
                        assert_eq!(get(&mut mmu, &mut bus, address + 32, asi), tag);
                    }
                    assert_eq!(get(&mut mmu, &mut bus, address, 0x0d), 0xfeed_face);
                    assert_eq!(get(&mut mmu, &mut bus, address, 0x0f), 0xdead_beef);
                }
            }
        }
    }
}

#[test]
fn diagnostic_guest_transfers_use_register_pairs_and_do_not_touch_ram() {
    let program = [
        alternate(0x17, 2, 1, 0x0f),
        alternate(0x13, 4, 1, 0x0f),
        alternate(0x19, 6, 1, 0x0f),
        alternate(0x1a, 7, 1, 0x0f),
    ];
    let mut m = Machine::bare_metal(0x1000, 0).unwrap();
    m.load_image(0, &bytes(&program)).unwrap();
    m.cpu.write_reg(1, 0x800);
    m.cpu.write_reg(2, 0x89ab_cdef);
    m.cpu.write_reg(3, 0x0123_4567);
    for _ in 0..4 {
        m.step().unwrap();
    }
    assert_eq!(
        (m.cpu.read_reg(4), m.cpu.read_reg(5)),
        (0x89ab_cdef, 0x0123_4567)
    );
    assert_eq!(
        (m.cpu.read_reg(6), m.cpu.read_reg(7)),
        (0xffff_ff89, 0xffff_89ab)
    );
    assert_eq!(m.bus.read_doubleword(0x800).unwrap(), [0, 0]);
}

#[test]
fn rejected_diagnostic_guest_access_is_precise_and_privilege_precedes_effects() {
    let mut m = Machine::bare_metal(0x1000, 0).unwrap();
    m.load_image(0, &bytes(&[alternate(0x16, 2, 1, 0x0c)]))
        .unwrap();
    m.cpu.write_reg(1, 0x400);
    m.cpu.write_reg(2, u32::MAX);
    let before = m.cpu.clone();
    assert!(matches!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedMemory(_)
    ));
    assert_eq!(m.cpu, before);
    for asi in 0x0c..=0x0f {
        let mut m = Machine::bare_metal(0x1000, 0).unwrap();
        m.load_image(
            0,
            &bytes(&[0x8188_2000 | PSR_ET, alternate(0x14, 2, 1, asi)]),
        )
        .unwrap();
        m.cpu.write_reg(1, 0x400);
        m.cpu.write_reg(2, u32::MAX);
        put(&mut m.cpu.mmu, &mut m.bus, 0x400, asi, 0x1234_0001);
        m.step().unwrap();
        assert!(matches!(
            m.step().unwrap(),
            Step::TrapTaken { trap_type: 3, .. }
        ));
        assert_eq!(get(&mut m.cpu.mmu, &mut m.bus, 0x400, asi), 0x1234_0001);
    }
}

#[test]
fn flush_instruction_is_unprivileged_and_only_invalidates_the_matching_i_tag() {
    for matches in [false, true] {
        let mut m = Machine::bare_metal(0x1000, 0).unwrap();
        m.load_image(0, &bytes(&[0x8188_2000 | PSR_ET, 0x81d8_6003]))
            .unwrap();
        m.cpu.write_reg(1, 0xf000_0080); // No physical map, deliberately unaligned after +3.
        let tag = if matches { 0xf000_0001 } else { 0xe000_0001 };
        for asi in [0x0c, 0x0e] {
            put(&mut m.cpu.mmu, &mut m.bus, 0x80, asi, tag);
            put(&mut m.cpu.mmu, &mut m.bus, 0xa0, asi, tag);
        }
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(
            get(&mut m.cpu.mmu, &mut m.bus, 0x80, 0x0c),
            tag & !u32::from(matches)
        );
        assert_eq!(get(&mut m.cpu.mmu, &mut m.bus, 0x80, 0x0e), tag);
        assert_eq!(get(&mut m.cpu.mmu, &mut m.bus, 0xa0, 0x0c), tag);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
    }
}

#[test]
fn power_on_and_software_reset_clear_diagnostic_storage() {
    for software in [false, true] {
        let mut m = Machine::sun4m(0x1000, &bytes(&[alternate(0x14, 2, 1, 0x20)])).unwrap();
        m.cpu.write_reg(1, RESET_BASE as u32);
        m.cpu.write_reg(2, 1);
        for asi in 0x0c..=0x0f {
            put(&mut m.cpu.mmu, &mut m.bus, 0, asi, u32::MAX);
            put(&mut m.cpu.mmu, &mut m.bus, 0x3ffc, asi, u32::MAX);
        }
        if software {
            assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
        } else {
            m.reset();
        }
        for asi in 0x0c..=0x0f {
            assert_eq!(get(&mut m.cpu.mmu, &mut m.bus, 0, asi), 0);
            assert_eq!(get(&mut m.cpu.mmu, &mut m.bus, 0x3ffc, asi), 0);
        }
    }
}
