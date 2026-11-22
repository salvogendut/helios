use super::{Access, Tlb};
use crate::ultra5::{
    BoardStep, BootFlash, ResetNvram, Ultra5ResetBoard, FLASH_SIZE, RESET_NVRAM_SIZE,
};
use crate::v9::{
    UnmappedData, V9Cpu, V9DataBus, LSU_DM, LSU_IM, PSTATE_MG, PSTATE_PEF, PSTATE_PRIV, PSTATE_RED,
};

const VALID: u64 = 1 << 63;
const USED: u64 = 1 << 41;
const LOCK: u64 = 1 << 6;
const RAM: u64 = 0x1000_0000;

fn board() -> Ultra5ResetBoard {
    Ultra5ResetBoard::new(
        BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
        ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
    )
}

#[test]
fn udivx_zero_divisor_in_delay_slot_enters_tt28_without_writeback() {
    let mut board = board();
    let mut cpu = V9Cpu::new(0x2024);
    cpu.npc = 0x5040;
    cpu.tba = 0x8000;
    cpu.ccr = 0x5a;
    cpu.cwp = 3;
    cpu.write_reg(19, 7);
    cpu.lsu_control = LSU_IM;
    cpu.instruction_tlb.stage_tag(0x2000);
    cpu.instruction_tlb.write_data(1, VALID | RAM | 0x20, true);
    board
        .write_cacheable_xword(RAM + 0x20, 0x0100_0000_a66c_e000)
        .unwrap(); // udivx %l3,0,%l3
    let before = cpu.clone();
    assert!(board
        .step_mapped(&mut cpu)
        .unwrap_err()
        .contains("division_by_zero"));
    assert_eq!(cpu, before);
    cpu.tl = 4;
    let full_stack = cpu.clone();
    assert!(board.step_outcome(&mut cpu).is_err());
    assert_eq!(cpu, full_stack);
    cpu.tl = 0;
    assert_eq!(
        board.step_outcome(&mut cpu).unwrap(),
        BoardStep::TrapTaken {
            fault_pc: 0x2024,
            tt: 0x28,
            vector: 0x8500
        }
    );
    let frame = cpu.traps[0];
    assert_eq!(
        (frame.tpc, frame.tnpc, frame.tt, frame.tstate),
        (0x2024, 0x5040, 0x28, (0x5a << 32) | 3)
    );
    assert_eq!(cpu.pstate, PSTATE_PRIV | PSTATE_PEF | crate::v9::PSTATE_AG);
    assert_eq!((cpu.read_reg(19), cpu.tick), (7, before.tick + 44));
    assert_ne!(cpu.instruction_tlb.read_data(1) & USED, 0);
    cpu.execute_fetched(0x81f0_0000, &mut UnmappedData).unwrap(); // done skips the divide
    assert_eq!(
        (cpu.pc, cpu.npc, cpu.pstate, cpu.ccr, cpu.cwp),
        (0x5040, 0x5044, 0, 0x5a, 3)
    );
}

#[test]
fn delay_slot_store_miss_enters_mmu_globals_refills_and_retries_once() {
    let mut board = board();
    let mut cpu = V9Cpu::new(0x2020);
    cpu.npc = 0x1_2340;
    cpu.tba = 0x8000;
    cpu.cwp = 6;
    cpu.ccr = 0x9c;
    cpu.asi = 0x82;
    cpu.pstate = PSTATE_PRIV | 0x100 | 2; // TLE and IE set
    cpu.primary_context = 0x123;
    cpu.lsu_control = LSU_IM | LSU_DM;
    cpu.instruction_tlb.stage_tag(0x2123);
    cpu.instruction_tlb.write_data(0, VALID | RAM | 0x20, true);
    board
        .write_cacheable_xword(RAM + 0x20, 0xc470_6000_0100_0000)
        .unwrap(); // stx %g2,[%g1]
    cpu.write_reg(1, 0x6080);
    cpu.write_reg(2, 0x1234_5678_9abc_def0);
    let before = cpu.clone();
    assert!(board
        .step_mapped(&mut cpu)
        .unwrap_err()
        .contains("TLB miss"));
    assert_eq!(cpu, before);

    assert_eq!(
        board.step_outcome(&mut cpu).unwrap(),
        BoardStep::TrapTaken {
            fault_pc: 0x2020,
            tt: 0x68,
            vector: 0x8d00
        }
    );
    let frame = cpu.traps[0];
    assert_eq!((frame.tpc, frame.tnpc, frame.tt), (0x2020, 0x1_2340, 0x68));
    assert_eq!(
        frame.tstate,
        (0x9cu64 << 32) | (0x82 << 24) | (0x106 << 8) | 6
    );
    assert_eq!(
        (cpu.pstate, cpu.tl, cpu.cwp, cpu.read_reg(1)),
        (PSTATE_MG | PSTATE_PRIV | PSTATE_PEF | 0x300, 1, 6, 0)
    );
    assert_ne!(cpu.instruction_tlb.read_data(0) & USED, 0);
    assert_eq!(cpu.data_tlb.tag_access(), 0x6123);
    assert_eq!(board.read_cacheable_xword(RAM + 0x4080).unwrap(), 0);
    cpu.execute_fetched(0x8750_8000, &mut UnmappedData).unwrap(); // rdpr %tstate,%g3
    assert_eq!(cpu.read_reg(3), frame.tstate);
    cpu.write_reg(2, VALID | (RAM + 0x4000) | 0x22);
    cpu.execute_fetched(0xc4f0_0b80, &mut UnmappedData).unwrap(); // stxa %g2,[%g0] 0x5c
    cpu.ccr = 0;
    cpu.asi = 0x58;
    cpu.execute_fetched(0x83f0_0000, &mut UnmappedData).unwrap();
    assert_eq!(
        (
            cpu.tl,
            cpu.ccr,
            cpu.asi,
            cpu.pstate,
            cpu.pc(),
            cpu.npc(),
            cpu.cwp
        ),
        (0, 0x9c, 0x82, 0x106, 0x2020, 0x1_2340, 6)
    );
    assert_eq!(
        (cpu.read_reg(2), cpu.read_reg(3)),
        (0x1234_5678_9abc_def0, 0)
    );
    assert!(
        matches!(board.step_outcome(&mut cpu).unwrap(),BoardStep::Retired(s) if s.instruction==0xc470_6000)
    );
    assert_eq!(
        board.read_cacheable_xword(RAM + 0x4080).unwrap(),
        0x1234_5678_9abc_def0
    );
    assert_eq!((cpu.pc(), cpu.npc()), (0x1_2340, 0x1_2344));
}

#[test]
fn nested_instruction_miss_saves_nucleus_context_and_done_skips() {
    let mut cpu = V9Cpu::new(0x4560);
    cpu.tl = 1;
    cpu.tba = 0x1_8000;
    cpu.primary_context = 7;
    cpu.lsu_control = LSU_IM;
    cpu.pstate = PSTATE_PRIV;
    cpu.npc = 0x4500;
    cpu.data_tlb.stage_tag(0x8005);
    let mut board = board();
    assert_eq!(
        board.step_outcome(&mut cpu).unwrap(),
        BoardStep::TrapTaken {
            fault_pc: 0x4560,
            tt: 0x64,
            vector: 0x1_cc80
        }
    );
    assert_eq!(
        (
            cpu.instruction_tlb.tag_access(),
            cpu.data_tlb.tag_access(),
            cpu.tl
        ),
        (0x4000, 0x8005, 2)
    );
    cpu.execute_fetched(0x81f0_0000, &mut UnmappedData).unwrap();
    assert_eq!(
        (cpu.pc(), cpu.npc(), cpu.tl, cpu.pstate),
        (0x4500, 0x4504, 1, PSTATE_PRIV)
    );
}

#[test]
fn unsupported_trap_levels_and_host_bus_faults_leave_cpu_intact() {
    let mut board = board();
    let mut cpu = V9Cpu::new(0);
    cpu.lsu_control = LSU_IM | LSU_DM;
    for tl in [4, 5] {
        cpu.tl = tl;
        let before = cpu.clone();
        assert!(board.step_outcome(&mut cpu).is_err());
        assert_eq!(cpu, before);
    }
    cpu.tl = 0;
    cpu.pstate = PSTATE_RED;
    let before = cpu.clone();
    // RED bypasses the I-MMU. Address zero is also outside flash.
    assert!(board.step_outcome(&mut cpu).is_err());
    assert_eq!(cpu, before);
    cpu.pstate = PSTATE_PRIV;
    cpu.instruction_tlb.stage_tag(0);
    cpu.instruction_tlb.write_data(0, VALID | RAM | 0x20, true);
    board
        .write_cacheable_xword(RAM, 0xc470_6000_0100_0000)
        .unwrap();
    cpu.data_tlb.stage_tag(0);
    cpu.data_tlb.write_data(1, VALID | RAM | 0x20, false); // read-only, a protection fault is distinct
    let before = cpu.clone();
    assert!(board
        .step_outcome(&mut cpu)
        .unwrap_err()
        .contains("protection"));
    assert_eq!(cpu, before);
    for (mode, tl, op) in [
        (PSTATE_PRIV, 0, 0x83f0_0000),
        (0, 1, 0x81f0_0000),
        (PSTATE_PRIV, 1, 0x83f0_0001),
    ] {
        cpu.pstate = mode;
        cpu.tl = tl;
        let before = cpu.clone();
        assert!(cpu.execute_fetched(op, &mut UnmappedData).is_err());
        assert_eq!(cpu, before);
    }
}

#[test]
fn data_in_chooses_invalid_then_unused_unlocked_then_used_unlocked() {
    let mut tlb = Tlb::default();
    let tte = VALID | 0x22;
    for slot in 0..64 {
        tlb.stage_tag((slot as u64) << 13);
        tlb.write_data(slot, tte | USED | LOCK, false);
    }
    tlb.write_data(2, tte | USED, false);
    tlb.write_data(3, tte, false);
    tlb.write_data(4, 0, false);
    tlb.stage_tag(0x100001);
    for (slot, page) in [(4, 0x2000), (3, 0x4000), (2, 0x6000)] {
        tlb.insert(tte | USED | page, false);
        assert_eq!(tlb.read_tag(slot), 0x100001);
        assert_eq!(tlb.read_data(slot), tte | USED | page);
    }
    assert_eq!(tlb.read_data(0) & (LOCK | USED), LOCK);
    for slot in 0..64 {
        tlb.write_data(slot, tte | LOCK | USED, false);
    }
    tlb.insert(tte, false);
    assert_eq!(tlb.read_data(63), tte);
    assert_eq!(tlb.read_data(62) & LOCK, LOCK);
}

#[test]
fn demap_obeys_page_size_global_context_bank_and_lock_rules() {
    let mut cpu = V9Cpu::new(0);
    cpu.pstate = PSTATE_PRIV;
    cpu.primary_context = 5;
    cpu.secondary_context = 7;
    for tlb in [&mut cpu.instruction_tlb, &mut cpu.data_tlb] {
        for (slot, tag, flags) in [
            (0, 0x2_0005, (1 << 61) | LOCK),
            (1, 0x2_0007, (1 << 61) | LOCK),
            (2, 0x3_0000, 1 | LOCK),
            (3, 0x4_0000, LOCK),
        ] {
            tlb.stage_tag(tag);
            tlb.write_data(slot, VALID | flags, false);
        }
    }
    let i_before = cpu.instruction_tlb.clone();
    let demap = |cpu: &mut V9Cpu, addr| {
        cpu.write_reg(1, addr);
        cpu.execute_fetched(0xc0f0_4be0, &mut UnmappedData) // stxa %g0,[%g1] 0x5f
    };
    demap(&mut cpu, 0x2_e780).unwrap(); // interior of locked 64 KiB page, primary context
    assert_eq!(cpu.data_tlb.read_data(0) >> 63, 0);
    assert_eq!(cpu.data_tlb.read_data(1) >> 63, 1);
    demap(&mut cpu, 0x50).unwrap(); // context demap, secondary
    assert_eq!(cpu.data_tlb.read_data(1) >> 63, 0);
    demap(&mut cpu, 0x60).unwrap(); // context demap, nucleus
    assert_eq!(cpu.data_tlb.read_data(3) >> 63, 0);
    assert_ne!(cpu.data_tlb.read_data(2) >> 63, 0); // global survives
    let before = cpu.data_tlb.clone();
    demap(&mut cpu, 0x3_0030).unwrap(); // reserved context select
    for bad in [0x3_0008, 1 << 43] {
        assert!(demap(&mut cpu, bad).is_err());
    }
    assert_eq!(cpu.data_tlb, before);
    demap(&mut cpu, 0x3_0000).unwrap(); // global matches any page context
    assert!(cpu
        .data_tlb
        .lookup(0x3_0000, 5, true, Access::Read)
        .is_err());
    assert_eq!(cpu.instruction_tlb, i_before);
}
