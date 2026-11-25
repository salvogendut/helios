//! UltraSPARC-IIi §13.5.3 and the Solaris 7 Xsun startup block store.

use super::*;
use crate::ultra5::{BootFlash, ResetNvram, Ultra5ResetBoard, FLASH_SIZE, RESET_NVRAM_SIZE};

const VA: u64 = 0xfd80_0000;
const PA: u64 = 0x01ff_e180_0000; // BAR0 + 8 MiB, second framebuffer aperture
const TTE: u64 = 0x8000_01ff_e180_001a;
const STORE: u32 = 0xc1bf_5e00; // stda %f0, [%i5 + %g0] ASI_BLK_P
const LOAD: u32 = 0xc19f_5e00;
const USED: u64 = 1 << 41;
const DISPLAY_CONFIG: u64 = 0x01fe_0101_1000;

fn display() -> Ultra5ResetBoard {
    let mut board = Ultra5ResetBoard::new(
        BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
        ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
    );
    board.write_bypass_byte(0x01fe_0100_0919, 1).unwrap();
    board.write_bypass_byte(0x01fe_0100_091a, 1).unwrap();
    board
        .write_bypass_le_word(DISPLAY_CONFIG + 0x10, 0xe100_0000)
        .unwrap();
    board.write_bypass_byte(DISPLAY_CONFIG + 4, 2).unwrap();
    board
}

fn cpu(tte: u64) -> V9Cpu {
    let mut cpu = V9Cpu::new(0xfeb6_1a38);
    cpu.pstate = PSTATE_PEF;
    cpu.fprs = 4;
    cpu.primary_context = 0x1062;
    cpu.data_tlb.stage_tag(VA | 0x1062);
    cpu.data_tlb.write_data(18, tte, false);
    cpu.lsu_control = LSU_DM | LSU_DC;
    cpu.write_reg(29, VA);
    cpu
}

#[test]
fn captured_block_store_writes_all_mach64_lanes_and_bypasses_the_data_cache() {
    let mut board = display();
    let mut cpu = cpu(TTE);
    // A diagnostic cache line with the same tag makes an unintended cache
    // write observable even for this CP=0, CV=1 mapping.
    let index = (VA as usize >> 5) & 511;
    cpu.dcache_tags[index] = ((PA >> 13) as u32) << 2 | 3;
    cpu.dcache_data[index] = [u64::MAX; 4];
    for i in 0..8 {
        let bytes = std::array::from_fn(|j| (i * 8 + j + 1) as u8);
        cpu.write_fp_double(2 * i as u8, u64::from_be_bytes(bytes));
    }
    board.write_bypass_byte(PA + 64, 0xa5).unwrap();
    cpu.execute_fetched(STORE, &mut board).unwrap();
    for i in 0..64 {
        assert_eq!(
            board.read_bypass_byte(PA - 0x80_0000 + i).unwrap(),
            (i + 1) as u8
        );
    }
    assert_eq!(board.read_bypass_byte(PA + 64).unwrap(), 0xa5);
    assert_eq!(cpu.dcache_data[index], [u64::MAX; 4]);
    assert_eq!((cpu.pc, cpu.npc), (0xfeb6_1a3c, 0xfeb6_1a40));
    assert_eq!(cpu.data_tlb.read_data(18), TTE | USED);
}

#[test]
fn uncached_load_and_store_reverse_bytes_per_doubleword_only() {
    let mut board = display();
    for (invert, little_asi) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut cpu = cpu(TTE | (u64::from(invert) << 59));
        let asi_bit = u32::from(little_asi) << 8;
        for i in 0..64 {
            board.write_bypass_byte(PA + i, i as u8).unwrap();
        }
        cpu.execute_fetched(LOAD | asi_bit, &mut board).unwrap();
        for i in 0..8 {
            let bytes: [u8; 8] = std::array::from_fn(|j| (i * 8 + j) as u8);
            let expected = if invert ^ little_asi {
                u64::from_le_bytes(bytes)
            } else {
                u64::from_be_bytes(bytes)
            };
            assert_eq!(cpu.fp_double(2 * i as u8), expected);
        }
        cpu.write_reg(29, VA + 64);
        cpu.execute_fetched(STORE | asi_bit, &mut board).unwrap();
        for i in 0..64 {
            assert_eq!(board.read_bypass_byte(PA + 64 + i).unwrap(), i as u8);
        }
    }
}

#[test]
fn rejected_blocks_preserve_registers_pixels_and_tlb_used_bit() {
    let mut board = display();
    board.write_bypass_byte(PA, 0xc5).unwrap();
    let reject = |board: &mut Ultra5ResetBoard, tte, va, word| {
        let mut cpu = cpu(tte);
        cpu.write_reg(29, va);
        let before = cpu.clone();
        let fault = cpu.execute_fetched(word, board).unwrap_err();
        assert!(cpu == before);
        fault
    };
    assert!(matches!(
        reject(&mut board, TTE & !2, VA, STORE),
        CpuFault::MmuProtection { .. }
    ));
    assert!(matches!(
        reject(&mut board, TTE, VA + 8, STORE),
        CpuFault::Synchronous(0x34)
    ));
    assert!(matches!(
        reject(&mut board, TTE, VA, STORE | (2 << 25)),
        CpuFault::Synchronous(0x10)
    ));
    for word in [STORE, LOAD] {
        let fault = reject(&mut board, TTE + 0x7f_e000, VA + 0x1c00, word);
        assert!(
            matches!(fault, CpuFault::Stop(msg) if msg.contains("unsupported Ultra 5 noncacheable block"))
        );
    }
    board.write_bypass_byte(DISPLAY_CONFIG + 4, 0).unwrap();
    assert!(matches!(
        reject(&mut board, TTE, VA, STORE),
        CpuFault::Stop(_)
    ));
    board.write_bypass_byte(DISPLAY_CONFIG + 4, 2).unwrap();
    assert_eq!(board.read_bypass_byte(PA).unwrap(), 0xc5);
}

#[test]
fn short_flat_memory_rejects_the_store_before_the_first_write() {
    let mut cpu = cpu(0x8000_0000_0000_0002);
    cpu.write_fp_double(0, u64::MAX);
    let mut memory = V9Memory::new(32);
    let before = cpu.clone();
    assert!(matches!(
        cpu.execute_fetched(STORE, &mut memory),
        Err(CpuFault::Stop(_))
    ));
    assert!(cpu == before);
    assert_eq!(memory.read_u64(0), 0);
}
