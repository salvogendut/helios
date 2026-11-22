use super::{Access, Entry, Tlb};
use crate::ultra5::{BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard, RESET_NVRAM_SIZE};
use crate::v9::{
    UnmappedData, V9Cpu, V9DataBus, V9Memory, LSU_DM, LSU_IM, PSTATE_PRIV, PSTATE_RED,
};

// STXA %g2,[%g1] asi and LDXA [%g1] asi,%g3.
const STORE: u32 = 0xc4f0_4000;
const LOAD: u32 = 0xc6d8_4000;
const VALID: u64 = 1 << 63;
const USED: u64 = 1 << 41;

#[test]
fn lookup_respects_page_size_context_global_and_the_full_virtual_tag() {
    for (size, bytes) in [8192u64, 65536, 524288, 4194304].into_iter().enumerate() {
        let mut tlb = Tlb::default();
        let va = 0x01ff_8000_0000;
        let pa = 0x0102_4500_0000;
        let offset = bytes - 8;
        // Low PPN/VPN bits below each page boundary are retained in
        // diagnostic readback but do not participate in translation.
        tlb.stage_tag(va | (offset & !8191) | 7);
        let tte = VALID | ((size as u64) << 61) | pa | (offset & !8191) | 0x24;
        tlb.write_data(63, tte, true);
        for i in [0, 8, offset] {
            let hit = tlb.lookup(va + i, 7, true, Access::Fetch).unwrap();
            assert_eq!((hit.slot, hit.pa, hit.cacheable), (Some(63), pa + i, true));
        }
        for addr in [va - 4, va + bytes, va & 0xffff_ffff] {
            assert!(tlb.lookup(addr, 7, true, Access::Fetch).is_err());
        }
        assert!(tlb.lookup(va, 6, true, Access::Fetch).is_err());
        assert!(tlb.lookup(va, 7, false, Access::Fetch).is_err());
        tlb.write_data(63, tte | 1, true);
        assert_eq!(tlb.lookup(va, 6, true, Access::Fetch).unwrap().pa, pa);
        tlb.write_data(3, tte, true);
        assert!(tlb
            .lookup(va, 7, true, Access::Fetch)
            .map_err(String::from)
            .unwrap_err()
            .contains("multiple"));
        tlb.write_data(63, 0, true);
        assert_eq!(
            tlb.lookup(va, 7, true, Access::Fetch).unwrap().slot,
            Some(3)
        );
    }
}

#[test]
fn red_state_bypasses_only_i_mmu_and_tl_selects_the_implicit_context() {
    let mut cpu = V9Cpu::new(0x4000);
    cpu.lsu_control = LSU_IM | LSU_DM;
    cpu.pstate = PSTATE_PRIV;
    cpu.primary_context = 7;
    for (i, t) in [&mut cpu.instruction_tlb, &mut cpu.data_tlb]
        .into_iter()
        .enumerate()
    {
        for (slot, ctx) in [0, 7].into_iter().enumerate() {
            t.stage_tag(0x4000 | ctx);
            t.write_data(slot, VALID | (1 + i as u64 * 2 + slot as u64) << 13, i == 0);
        }
    }
    assert_eq!(cpu.instruction_address().unwrap().pa, 0x4000);
    assert_eq!(cpu.translate(0x4000, 8, Access::Read).unwrap().pa, 0x8000);
    cpu.tl = 1;
    assert_eq!(cpu.instruction_address().unwrap().pa, 0x2000);
    assert_eq!(cpu.translate(0x4000, 8, Access::Read).unwrap().pa, 0x6000);
    cpu.pstate |= PSTATE_RED;
    assert_eq!(cpu.instruction_address().unwrap().slot, None);
    assert_eq!(cpu.translate(0x4000, 8, Access::Read).unwrap().pa, 0x6000);
    cpu.pstate = PSTATE_PRIV;
    cpu.lsu_control = LSU_IM;
    assert_eq!(cpu.translate(0x4000, 8, Access::Read).unwrap().slot, None);
    assert_eq!(cpu.instruction_address().unwrap().slot, Some(0));
    assert!(cpu.translate(1 << 43, 4, Access::Fetch).is_err());
}

#[test]
fn translated_data_checks_writes_endianness_and_flush_before_mutation() {
    let mut cpu = V9Cpu::new(0);
    let mut ram = V9Memory::new(24);
    ram.write_u64(8, 0x0102_0304_0506_0708);
    cpu.lsu_control = LSU_DM;
    cpu.write_reg(1, 0x2008);
    cpu.data_tlb.stage_tag(0x2000);
    for (op, tte) in [
        (0xc470_6000, VALID),             // STX to read-only page
        (0xc458_6000, VALID | 4),         // user LDX to privileged page
        (0xc458_6000, VALID | (1 << 60)), // NFO
        (0x81d8_6000, VALID | 8),         // FLUSH with side effects
        (0x81d8_6000, VALID | (1 << 60)),
        (0xc458_6000, VALID | 0x2000), // legal translation, no backing
    ] {
        cpu.data_tlb.write_data(17, tte, false);
        let before = (cpu.clone(), ram.clone());
        assert!(cpu.execute_fetched(op, &mut ram).is_err());
        assert_eq!((&cpu, &ram), (&before.0, &before.1));
    }
    cpu.data_tlb.write_data(17, VALID | (1 << 59) | 0x22, false);
    cpu.execute_fetched(0xc458_6000, &mut ram).unwrap(); // ldx [%g1],%g2
    assert_eq!(cpu.read_reg(2), 0x0807_0605_0403_0201);
    assert_ne!(cpu.data_tlb.read_data(17) & USED, 0);
    cpu.write_reg(2, 0xf0e1_d2c3_b4a5_9687);
    cpu.execute_fetched(0xc470_6000, &mut ram).unwrap();
    assert_eq!(ram.read_u64(8), 0x8796_a5b4_c3d2_e1f0);
    cpu.execute_fetched(0xc408_6001, &mut ram).unwrap(); // ldub [%g1+1],%g2
    assert_eq!(cpu.read_reg(2), 0x96);
    cpu.data_tlb.write_data(17, VALID, false);
    // A read-only translation suffices for FLUSH and no device read occurs.
    cpu.execute_fetched(0x81d8_6000, &mut UnmappedData).unwrap();
    assert_ne!(cpu.data_tlb.read_data(17) & USED, 0);
}

#[test]
fn translated_fetch_updates_used_in_instruction_order_and_rolls_back_errors() {
    let mut cpu = V9Cpu::new(0x2000);
    let mut ram = V9Memory::new(16);
    cpu.pstate = PSTATE_PRIV;
    cpu.lsu_control = LSU_IM;
    cpu.instruction_tlb.stage_tag(0x2000);
    cpu.instruction_tlb.write_data(0, VALID, true);
    let before = cpu.clone();
    assert!(cpu.step(&mut ram).is_err()); // zero word is unsupported
    assert_eq!(cpu, before);
    ram.load_words(0, &[LOAD | (0x55 << 5), STORE | (0x55 << 5)]);
    cpu.step(&mut ram).unwrap(); // read the entry used for this fetch
    assert_eq!(cpu.read_reg(3), VALID | USED);
    cpu.step(&mut ram).unwrap(); // replace own entry with zero
    assert_eq!(cpu.instruction_tlb.read_data(0), 0);
    let before = cpu.clone();
    assert!(cpu.step(&mut ram).unwrap_err().contains("TLB miss"));
    assert_eq!(cpu, before);
}

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

const OBP_FLASH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
);

fn boot_to_mmu_enable(diagnostics: bool) -> (BootFlash, Ultra5ResetBoard, V9Cpu) {
    let flash = BootFlash::new(std::fs::read(OBP_FLASH).unwrap()).unwrap();
    let mut seed = vec![0; RESET_NVRAM_SIZE];
    seed[1] = u8::from(diagnostics);
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
    (flash, board, cpu)
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_installs_distinct_flash_and_workspace_translations_before_enabling_mmus() {
    let (flash, _, cpu) = boot_to_mmu_enable(false);
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

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_enables_mmus_copies_flash_and_executes_from_dram() {
    let (flash, mut board, mut cpu) = boot_to_mmu_enable(true);
    board.step_mapped(&mut cpu).unwrap(); // enable both MMUs
    assert_eq!(cpu.lsu_control, 0xc);
    let mut missing = cpu.clone();
    missing.instruction_tlb.write_data(2, 0, true);
    let before = missing.clone();
    assert!(board
        .step_mapped(&mut missing)
        .unwrap_err()
        .contains("TLB miss"));
    assert_eq!(missing, before);
    let fetch = board.step_mapped(&mut cpu).unwrap();
    assert_eq!(
        (fetch.physical_address, fetch.instruction),
        (0x01ff_f000_1f94, 0x8143_e040)
    );
    assert_ne!(cpu.instruction_tlb.read_data(2) & USED, 0);
    board.step_mapped(&mut cpu).unwrap(); // FLUSH at VA 0 via the D-TLB
    assert_ne!(cpu.data_tlb.read_data(3) & USED, 0);

    let dest = 0x3ff0_0000;
    board.write_cacheable_xword(dest - 8, 0x1234_5678).unwrap();
    board
        .write_cacheable_xword(dest + 0x40000, 0x9abc_def0)
        .unwrap();
    for _ in 0..140_000 {
        if cpu.pc() == 0x2038 {
            break;
        }
        board.step_mapped(&mut cpu).unwrap();
    }
    assert_eq!((cpu.pc(), cpu.npc()), (0x2038, 0x203c));
    for (i, bytes) in flash.image()[..0x40000].chunks_exact(8).enumerate() {
        assert_eq!(
            board.read_cacheable_xword(dest + 8 * i as u64).unwrap(),
            u64::from_be_bytes(bytes.try_into().unwrap()),
            "copied xword {i:#x}"
        );
    }
    assert_eq!(board.read_cacheable_xword(dest - 8).unwrap(), 0x1234_5678);
    assert_eq!(
        board.read_cacheable_xword(dest + 0x40000).unwrap(),
        0x9abc_def0
    );
    let original = board.read_cacheable_xword(dest + 0x2038).unwrap();
    // Distinguish a real fetch from copied storage from a ROM-offset alias.
    let mut probe = cpu.clone();
    let marker = 0x8a10_2007u32; // mov 7,%g5
    board
        .write_cacheable_xword(
            dest + 0x2038,
            u64::from(marker) << 32 | (original & 0xffff_ffff),
        )
        .unwrap();
    let fetch = board.step_mapped(&mut probe).unwrap();
    assert_eq!(
        (fetch.physical_address, fetch.instruction, probe.read_reg(5)),
        (dest + 0x2038, marker, 7)
    );
    board
        .write_cacheable_xword(dest + 0x2038, original)
        .unwrap();
    for offset in [0x2038, 0x203c] {
        let fetch = board.step_mapped(&mut cpu).unwrap();
        assert_eq!(
            (fetch.physical_address, fetch.instruction),
            (dest + offset, 0x0100_0000)
        );
    }
    assert_ne!(cpu.instruction_tlb.read_data(1) & USED, 0);
    assert_eq!(cpu.instruction_tlb.read_data(0) & USED, 0);
    for _ in 0..3_000 {
        if cpu.pc() == 0x26fc {
            break;
        }
        board.step_mapped(&mut cpu).unwrap();
    }
    assert_eq!((cpu.pc(), cpu.npc()), (0x26fc, 0x2700));
    assert_eq!(cpu.read_reg(16) + 0x26c, 0x4f26c); // target of ST %i0,[%l0+0x26c]
    assert_eq!(cpu.read_reg(24), 0x2a80);
    let [a, b] = board.take_serial_output();
    assert!(a.ends_with(b"MMUs ON\r\nCopy Done\r\nPC = 0000.01ff.f000.201c\r\nPC = 0000.0000.0000.2060\r\nDecompressing into Memory "));
    assert!(b.is_empty());
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash and Python 3"]
fn original_obp_expands_the_image_and_executes_its_entry_in_ram() {
    let reference = std::process::Command::new("python3")
        .args([
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tools/ultra5-fixture/decompress_reference.py"
            ),
            OBP_FLASH,
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    assert_eq!(reference.stdout.len(), 0x6eba0);

    let (_, mut board, mut cpu) = boot_to_mmu_enable(true);
    let run_to = |board: &mut Ultra5ResetBoard, cpu: &mut V9Cpu, pc, limit| {
        for _ in 0..limit {
            if cpu.pc() == pc {
                return;
            }
            board.step_mapped(cpu).unwrap();
        }
        panic!("did not reach {pc:#x}, stopped at {:#x}", cpu.pc());
    };

    run_to(&mut board, &mut cpu, 0x26e8, 150_000);
    let base = 0x3ff8_0000;
    let guards = [
        (base - 8, 0x1234_5678_9abc_def0),
        (base + 0x6eba0, 0xfedc_ba98_7654_3210),
    ];
    for (address, value) in guards {
        board.write_cacheable_xword(address, value).unwrap();
    }
    run_to(&mut board, &mut cpu, 0x20b0, 22_000_000);
    assert_eq!(
        board.read_cacheable_xword(0x3ff4_1f50).unwrap() >> 32,
        0xf006_eba0
    );
    for (i, chunk) in reference.stdout.chunks_exact(8).enumerate() {
        let address = base + i as u64 * 8;
        assert_eq!(
            board.read_cacheable_xword(address).unwrap().to_be_bytes(),
            chunk,
            "PA {address:#x}"
        );
    }
    for (address, value) in guards {
        assert_eq!(board.read_cacheable_xword(address).unwrap(), value);
    }

    run_to(&mut board, &mut cpu, 0xf005_1f8c, 5_000);
    let entry = board.step_mapped(&mut cpu).unwrap();
    assert_eq!(
        (entry.physical_address, entry.instruction),
        (0x3ffd_1f8c, 0x9190_200f)
    );
    run_to(&mut board, &mut cpu, 0xf005_1fb8, 32);
    assert_eq!(cpu.npc(), 0xf005_1fbc);
    let dispatch = board.step_mapped(&mut cpu).unwrap();
    assert_eq!(
        (
            dispatch.physical_address,
            dispatch.instruction,
            cpu.dispatch_control
        ),
        (0x3ffd_1fb8, 0xa580_203d, 0x3d)
    );
    run_to(&mut board, &mut cpu, 0xf005_1fc8, 4);
    assert_eq!(cpu.read_reg(1), 0x3fe0); // first D-cache diagnostic tag address
    assert_eq!(
        board.read_cacheable_xword(0x3ffd_1fc8).unwrap() >> 32,
        0xc0f0_48e0
    );
    // The ROM clears 512 D-cache lines and both sets of 256 I-cache
    // lines. Seed both banks so that a skipped store cannot pass the check.
    cpu.dcache_tags.fill(0x3fff_ffff);
    cpu.icache_tags.fill(0x1fff_ffff);
    for (store_pc, word, end_pc) in [
        (0xf005_1fc8, 0xc0f0_48e0, 0xf005_1fd8),
        (0xf005_1fe0, 0xc0f0_4ce0, 0xf005_1ff0),
    ] {
        let mut stores = 0;
        for _ in 0..3000 {
            if cpu.pc() == end_pc {
                break;
            }
            if cpu.pc() == store_pc {
                assert!(stores < 512);
                assert_eq!(cpu.read_reg(1), 0x3fe0 - 32 * stores);
                let step = board.step_mapped(&mut cpu).unwrap();
                assert_eq!(step.instruction, word);
                stores += 1;
            } else {
                board.step_mapped(&mut cpu).unwrap();
            }
        }
        assert_eq!((cpu.pc(), stores), (end_pc, 512));
        assert!(cpu.dcache_tags.iter().all(|&tag| tag == 0));
        let expected = if store_pc == 0xf005_1fc8 {
            0x1fff_ffff
        } else {
            0
        };
        assert!(cpu.icache_tags.iter().all(|&tag| tag == expected));
    }
    run_to(&mut board, &mut cpu, 0xf000_a764, 20_000);
    assert_eq!(cpu.ccr() & 0x40, 0); // TE checks XCC.Z
    let checked = board.step_mapped(&mut cpu).unwrap();
    assert_eq!((checked.instruction, cpu.pc()), (0x83d0_303e, 0xf000_a768));
    run_to(&mut board, &mut cpu, 0xf002_3944, 120_000);
    let address = cpu.instruction_address().unwrap().pa;
    assert_eq!(address, 0x3ffa_3944);
    assert_eq!(
        board.read_cacheable_xword(address & !7).unwrap() as u32,
        0x8d81_2000
    );
    assert_eq!((cpu.read_reg(4), cpu.pstate), (4, 0x14)); // wr %g4,0,%fprs
    let first = board.step_outcome(&mut cpu).unwrap();
    assert!(
        matches!(first, crate::ultra5::BoardStep::Retired(step) if step.instruction == 0x8d81_2000)
    );
    assert_eq!((cpu.pc(), cpu.fprs), (0xf002_3948, 4));
    verify_obp_mmu_refills(&mut board, &mut cpu);
    let [a, b] = board.take_serial_output();
    assert!(a.ends_with(
        b"Decompressing into Memory Done\r\nSize = 0000.0000.0006.eba0\r\nttya initialized\r\n"
    ));
    assert!(b.is_empty());
}

fn verify_obp_mmu_refills(board: &mut Ultra5ResetBoard, cpu: &mut V9Cpu) {
    use crate::ultra5::BoardStep;
    let mut traps = Vec::new();
    let mut first_retry_completed = false;
    for _ in 0..2_500_000 {
        let pc = cpu.pc();
        if pc == 0xf000_9238 && cpu.read_reg(4) == 0xfffc_6000 && traps.len() == 4 {
            let target = cpu.translate(cpu.read_reg(4), 8, Access::Write).unwrap();
            assert_eq!(
                (target.pa, target.cacheable, cpu.read_reg(16)),
                (0x01fe_0000_2000, false, 0x14_0020_010f)
            );
            assert_eq!(
                traps,
                [
                    (0xf000_9018, 0xffff_ffff_ff00_0000),
                    (0xf000_9238, 0xffff_fffc_003f_e000),
                    (0xf000_9238, 0xffff_fffc_003c_0000),
                    (0xf000_9238, 0xfffc_6000),
                ]
            );
            assert!(first_retry_completed);
            return;
        }
        match board.step_outcome(cpu).unwrap() {
            BoardStep::TrapTaken {
                fault_pc,
                tt,
                vector,
            } => {
                assert_eq!((fault_pc, tt, vector, cpu.tl), (pc, 0x68, 0xf000_0d00, 1));
                traps.push((pc, cpu.data_tlb.tag_access()));
            }
            BoardStep::Retired(step) if pc == 0xf000_0f78 && traps.len() == 1 => {
                assert_eq!(step.instruction, 0x83f0_0000); // RETRY
                assert_eq!(
                    (cpu.pc(), cpu.npc(), cpu.tl, cpu.pstate, cpu.read_reg(4)),
                    (0xf000_9018, 0xf000_901c, 0, 0x14, 0xffff_ffff_ff00_0f00)
                );
                let page = cpu.translate(cpu.read_reg(4), 8, Access::Read).unwrap();
                assert_eq!((page.slot, page.pa), (Some(5), 0x3ff6_cf00));
                assert_eq!(board.step_mapped(cpu).unwrap().instruction, 0xc859_2000);
                assert_eq!(cpu.read_reg(4), 0);
                first_retry_completed = true;
            }
            _ => {}
        }
    }
    panic!("did not reach PCI control, PC={:#x}", cpu.pc());
}

#[test]
fn narrow_integer_accesses_select_dram_lanes_and_apply_endianness_once() {
    let flash = BootFlash::new(vec![0; crate::ultra5::FLASH_SIZE]).unwrap();
    let mut board =
        Ultra5ResetBoard::new(flash, ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap());
    let mut cpu = V9Cpu::new(0);
    cpu.lsu_control = LSU_DM;
    cpu.data_tlb.stage_tag(0x2000);
    let pa = 0x1000_0000;
    let original = [0x81, 0x82, 0x93, 0x94, 0xa5, 0xa6, 0xb7, 0xb8];
    let source = 0xfedc_ba98_e4d3_c2b1u64;
    for pstate_little in [false, true] {
        for page_inverted in [false, true] {
            cpu.pstate = PSTATE_PRIV | (u16::from(pstate_little) << 9);
            cpu.data_tlb.write_data(
                3,
                VALID | pa | 0x22 | (u64::from(page_inverted) << 59),
                false,
            );
            let little = pstate_little ^ page_inverted;
            // Immediate-address forms: load/store %g2 at [%g1 + 0].
            for (load, store, width) in [
                (0xc408_6000, 0xc428_6000, 1),
                (0xc410_6000, 0xc430_6000, 2),
                (0xc400_6000, 0xc420_6000, 4),
            ] {
                for lane in (0..8).step_by(width) {
                    board
                        .write_cacheable_xword(pa, u64::from_be_bytes(original))
                        .unwrap();
                    cpu.write_reg(1, 0x2000 + lane as u64);
                    cpu.write_reg(2, u64::MAX);
                    cpu.execute_fetched(load, &mut board).unwrap();
                    let mut raw = original[lane..lane + width].to_vec();
                    if little {
                        raw.reverse();
                    }
                    let expected = raw.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
                    assert_eq!(cpu.read_reg(2), expected);
                    cpu.write_reg(2, source);
                    cpu.execute_fetched(store, &mut board).unwrap();
                    let mut expected = original;
                    let bytes = if little {
                        source.to_le_bytes()
                    } else {
                        source.to_be_bytes()
                    };
                    expected[lane..lane + width].copy_from_slice(if little {
                        &bytes[..width]
                    } else {
                        &bytes[8 - width..]
                    });
                    assert_eq!(
                        board.read_cacheable_xword(pa).unwrap().to_be_bytes(),
                        expected
                    );
                }
            }
            assert_ne!(cpu.data_tlb.read_data(3) & USED, 0);
        }
    }
}

#[test]
fn narrow_data_faults_preserve_registers_memory_and_tlb_used() {
    let mut cpu = V9Cpu::new(0);
    let mut ram = V9Memory::new(18);
    cpu.lsu_control = LSU_DM;
    cpu.data_tlb.stage_tag(0x2000);
    ram.write_u64(8, 0x1234_5678_90ab_cdef);
    cpu.write_reg(2, u64::MAX);

    // Read-only page, misaligned half/word, and a word straddling the backing limit.
    for (op, va, flags) in [
        (0xc428_6000, 0x2008, 0x20),
        (0xc410_6000, 0x2009, 0x22),
        (0xc430_6000, 0x2009, 0x22),
        (0xc400_6000, 0x200a, 0x22),
        (0xc420_6000, 0x2010, 0x22),
    ] {
        cpu.data_tlb.write_data(3, VALID | flags, false);
        cpu.write_reg(1, va);
        let before = (cpu.clone(), ram.clone());
        assert!(cpu.execute_fetched(op, &mut ram).is_err());
        assert_eq!((&cpu, &ram), (&before.0, &before.1));
    }

    cpu.write_reg(1, 0x2010);
    cpu.execute_fetched(0xc430_6000, &mut ram).unwrap();
    cpu.execute_fetched(0xc210_6000, &mut ram).unwrap(); // lduh [%g1],%g1, last two bytes
    assert_eq!(cpu.read_reg(1), 0xffff);
}
