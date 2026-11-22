use super::Entry;
use crate::ultra5::{BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard, RESET_NVRAM_SIZE};
use crate::v9::{UnmappedData, V9Cpu};

// STXA %g2,[%g1] asi and LDXA [%g1] asi,%g3.
const STORE: u32 = 0xc4f0_4000;
const LOAD: u32 = 0xc6d8_4000;

fn access(cpu: &mut V9Cpu, op: u32, asi: u32, address: u64, value: u64) -> u64 {
    cpu.write_reg(1, address);
    cpu.write_reg(2, value);
    cpu.execute_fetched(op | (asi << 5), &mut UnmappedData)
        .unwrap();
    cpu.read_reg(3)
}

#[test]
fn directed_stores_snapshot_their_own_tag_and_return_architectural_tte_fields() {
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    for (mmu, data, tags) in [(0x50, 0x55, 0x56), (0x58, 0x5d, 0x5e)] {
        for (size, diag) in [(0, 0), (1, 0x12), (2, 0x36), (3, 0x7e)] {
            let offset = size * 8;
            let context = (size << 8) | u64::from(mmu);
            let raw_tag = 0x1234_5800_001f_e000 | context;
            let tag = 0xffff_f800_001f_e000 | context;
            access(&mut cpu, STORE, mmu, 0x30, raw_tag);
            assert_eq!(access(&mut cpu, LOAD, mmu, 0x30, 0), tag);
            let raw_data = !(3u64 << 61) | (size << 61);
            access(&mut cpu, STORE, data, offset, raw_data);
            access(&mut cpu, STORE, mmu, 0x30, 0x1fff); // leave the captured tag intact
            assert_eq!(access(&mut cpu, LOAD, tags, offset, 0), tag);
            let flags = if mmu == 0x50 {
                0x8000_0000_0000_0065
            } else {
                0x9800_0000_0000_007f
            };
            let expected = flags | (size << 61) | ((diag | 1) << 41) | 0x0000_01ff_ffff_e000;
            assert_eq!(access(&mut cpu, LOAD, data, offset, 0), expected);
        }
    }
    // Same slot number on opposite sides must retain distinct tags.
    assert_eq!(access(&mut cpu, LOAD, 0x56, 24, 0) & 0xff, 0x50);
    assert_eq!(access(&mut cpu, LOAD, 0x5e, 24, 0) & 0xff, 0x58);
}

#[test]
fn indexed_tlb_rejects_bad_address_tag_stores_and_unprivileged_access_atomically() {
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    access(&mut cpu, STORE, 0x58, 0x30, 0x1234);
    access(&mut cpu, STORE, 0x5d, 0x1f8, 1 << 63);
    for (op, asi, address) in [
        (LOAD, 0x55, 1),
        (STORE, 0x5d, 0x200),
        (STORE, 0x56, 0),
        (STORE, 0x5e, 0x1f8),
        (LOAD, 0x50, 0x31),
    ] {
        cpu.write_reg(1, address);
        let before = cpu.clone();
        assert!(cpu
            .execute_fetched(op | (asi << 5), &mut UnmappedData)
            .is_err());
        assert_eq!(cpu, before);
    }
    let mut user = V9Cpu::new(0);
    for (op, asi) in [(LOAD, 0x56), (STORE, 0x55), (LOAD, 0x5e), (STORE, 0x5d)] {
        let before = user.clone();
        assert!(user
            .execute_fetched(op | (asi << 5), &mut UnmappedData)
            .is_err());
        assert_eq!(user, before);
    }
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn obp_clears_every_seeded_instruction_and_data_entry_then_probes_memory() {
    let flash = BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap();
    for soft in [true, false] {
        let mut seed = vec![0; RESET_NVRAM_SIZE];
        seed[1] = 1;
        if soft {
            seed[16..24].copy_from_slice(b"soft-rst");
        }
        let mut board = Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(seed).unwrap());
        let reset = PowerOnReset::ultra_sparc_ii();
        let base = reset.pc - 0x20;
        let mut cpu = V9Cpu::from(reset);
        // A missed store would otherwise look like a successful clear after POR.
        for (side, tlb) in [&mut cpu.instruction_tlb, &mut cpu.data_tlb]
            .into_iter()
            .enumerate()
        {
            for slot in 0..64 {
                tlb.stage_tag(((slot as u64 + 1) << 13) | side as u64);
                tlb.write_data(slot, (1 << 63) | 0x24, side == 0);
            }
        }
        let mut counts = [0; 2];
        let mut checked = false;
        for _ in 0..31_000_000 {
            match cpu.pc() - base {
                pc @ (0x794 | 0x79c) => {
                    let count = &mut counts[usize::from(pc == 0x79c)];
                    assert_eq!(cpu.read_reg(8), (63 - *count) * 8);
                    *count += 1;
                }
                0x7ac => {
                    assert_eq!(counts, [64, 64]);
                    for tlb in [&cpu.instruction_tlb, &cpu.data_tlb] {
                        assert_eq!(tlb.tag_access(), 0);
                        assert_eq!(tlb.entries, [Entry::default(); 64]);
                    }
                    checked = true;
                }
                0xfec => break,
                _ => {}
            }
            board.step(&mut cpu).unwrap();
        }
        assert!(checked);
        assert_eq!((cpu.pc(), cpu.npc()), (base + 0xfec, base + 0xff0));
        assert_eq!(
            (cpu.read_reg(24), cpu.read_reg(12), cpu.read_reg(13)),
            (0, 0x80_0000, 0x100_0000)
        );
        assert_eq!(
            (cpu.read_reg(9), cpu.read_reg(10)),
            (0xaaaa_aaaa_5555_5555, 0x5555_5555_aaaa_aaaa)
        );
        let [a, b] = board.take_serial_output();
        assert!(a.ends_with(b"Clearing I/D TLBs Done\r\nProbing Memory "));
        assert!(b.is_empty());
        println!(
            "soft={soft} index stores={counts:?} first DRAM probe PA={:#x}",
            cpu.read_reg(24)
        );
    }
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_installs_distinct_flash_and_workspace_translations_before_enabling_mmus() {
    let flash = BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap();
    let mut seed = vec![0; RESET_NVRAM_SIZE];
    seed[16..24].copy_from_slice(b"soft-rst");
    let mut board = Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(seed).unwrap());
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    for _ in 0..600_000 {
        if cpu.pc() == 0x01ff_f000_1f90 {
            break;
        }
        board.step(&mut cpu).unwrap();
    }
    assert_eq!((cpu.pc(), cpu.npc()), (0x01ff_f000_1f90, 0x01ff_f000_1f94));
    assert_eq!(
        (cpu.primary_context, cpu.secondary_context, cpu.lsu_control),
        (0, 0, 0)
    );
    // OBP 0x83c..0x94c derives the page bases from the selected rank's end
    // (0x40000000) and maps the flash separately. Sizes are IIi TTE encodings.
    let flash_page = 0x01ff_f000_0000;
    let itlb = [
        (0xf000_0000, 0x3ff8_0000, 2, 0x64),
        (0, 0x3ff0_0000, 2, 0x24),
        (flash_page, flash_page, 2, 0x04),
    ];
    let dtlb = [
        (0xffff_ffff_ffff_c000, 0x3ff6_e000, 0, 0x76),
        (0xfffe_0000, 0x3ff7_0000, 1, 0x36),
        (flash_page, flash_page, 2, 0x0e),
        (0, 0x3ff0_0000, 2, 0x36),
        (0xf000_0000, 0x3ff8_0000, 2, 0x36),
    ];
    for (tlb, expected) in [
        (&cpu.instruction_tlb, itlb.as_slice()),
        (&cpu.data_tlb, dtlb.as_slice()),
    ] {
        for (index, &(tag, physical, size, flags)) in expected.iter().enumerate() {
            let data = tlb.read_data(index);
            assert_eq!(tlb.read_tag(index), tag);
            assert_eq!((data >> 63, (data >> 61) & 3), (1, size));
            assert_eq!(data & 0x0000_01ff_ffff_e000, physical);
            assert_eq!(data & 0x7f, flags);
        }
        assert!(tlb.entries[expected.len()..]
            .iter()
            .all(|entry| entry.data >> 63 == 0));
    }
    assert_eq!(flash.read_word(flash_page + 0x1f94).unwrap(), 0x8143_e040); // membar #Sync
}
