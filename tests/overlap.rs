use helios::bus::{Bus, MemoryBus};
use helios::mmu::{Srmmu, Width, PCR_EN};

fn reg(m: &mut Srmmu, b: &mut impl Bus, r: u32, v: u32) {
    m.store(b, r << 8, 4, Width::Word, v).unwrap();
}

#[test]
fn new_global_narrow_mapping_displaces_broad_fast_hits_in_both_caches() {
    for narrow_level in [2, 3] {
        let mut bus = MemoryBus::default();
        bus.map_ram(0, 0x10000).unwrap();
        bus.map_ram(0x0200_0000, 0x1000).unwrap();
        bus.map_ram(0x0400_0000, 0x1000).unwrap();
        bus.write32(0x0200_0000, 0x1111_1111).unwrap();
        bus.write32(0x0400_0000, 0x2222_2222).unwrap();
        // Context 0 has a nonglobal 16 MiB mapping. Context 1 supplies a
        // smaller, supervisor-global mapping for the same virtual address.
        for (address, word) in [
            (0x1000, 0x201),
            (0x2048, 0x0020_00ae),
            (0x1004, 0x501),
            (0x5048, 0x601),
        ] {
            bus.write32(address, word).unwrap();
        }
        if narrow_level == 2 {
            bus.write32(0x6000, 0x0040_00be).unwrap();
        } else {
            bus.write32(0x6000, 0x701).unwrap();
            bus.write32(0x7000, 0x0040_00be).unwrap();
        }
        let mut mmu = Srmmu::default();
        reg(&mut mmu, &mut bus, 1, 0x100);
        reg(&mut mmu, &mut bus, 0, PCR_EN);
        for _ in 0..2 {
            for asi in [9, 11] {
                assert_eq!(
                    mmu.load(&mut bus, 0x1200_0000, asi, Width::Word),
                    Ok(0x1111_1111)
                );
            }
        }
        reg(&mut mmu, &mut bus, 2, 1);
        assert_eq!(
            mmu.load(&mut bus, 0x1200_0000, 11, Width::Word),
            Ok(0x2222_2222)
        );
        reg(&mut mmu, &mut bus, 2, 0);
        for asi in [9, 11] {
            assert_eq!(
                mmu.load(&mut bus, 0x1200_0000, asi, Width::Word),
                Ok(0x2222_2222),
                "level {narrow_level}, ASI {asi}"
            );
        }
    }
}

#[test]
fn diagnostic_permission_edit_invalidates_warm_instruction_and_data_entries() {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x10000).unwrap();
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3000, 0x401),
        (0x4000, 0x8ae),
        (0x8000, 0x1234_5678),
    ] {
        bus.write32(address, word).unwrap();
    }
    let mut mmu = Srmmu::default();
    reg(&mut mmu, &mut bus, 1, 0x100);
    reg(&mut mmu, &mut bus, 0, PCR_EN);
    for _ in 0..2 {
        for asi in [9, 11] {
            assert_eq!(
                mmu.load(&mut bus, 0x1200_0000, asi, Width::Word),
                Ok(0x1234_5678)
            );
        }
    }
    // Clear all permission bits in the resident architectural entry.
    mmu.store(&mut bus, 0x100, 6, Width::Word, 0).unwrap();
    for asi in [9, 11] {
        assert!(mmu.load(&mut bus, 0x1200_0000, asi, Width::Word).is_err());
    }
}
