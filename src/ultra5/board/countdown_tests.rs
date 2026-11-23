use super::*;
use crate::ultra5::{PowerOnReset, FLASH_SIZE, RESET_NVRAM_SIZE};

const RAM: u64 = 0x1000_0000;
const NOP: u32 = 0x0100_0000;

fn countdown(tick_compare: u64) -> (Ultra5ResetBoard, V9Cpu) {
    let mut board = Ultra5ResetBoard::new(
        BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
        ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
    );
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    cpu.execute_fetched(0x8d90_2006, &mut board).unwrap(); // WRPR PRIV|IE, PSTATE
    cpu.execute_fetched(0x8f90_2000, &mut board).unwrap(); // WRPR 0, TL
                                                           // Map VA 0 to the first installed RAM bank and enable I-MMU and I-cache.
    for (asi, address, data) in [
        (0x50, 0x30, 0),
        (0x55, 0, (1 << 63) | RAM | 0x20),
        (0x45, 0, 5),
    ] {
        cpu.write_reg(1, address);
        cpu.write_reg(2, data);
        cpu.execute_fetched(0xc4f0_4000 | (asi << 5), &mut board)
            .unwrap();
    }
    cpu.write_reg(4, tick_compare);
    cpu.execute_fetched(0xaf81_2000, &mut board).unwrap(); // WR %g4, TICK_CMPR
    cpu.execute_fetched(0x81c0_2000, &mut board).unwrap(); // JMPL 0, %g0
    cpu.execute_fetched(NOP, &mut board).unwrap();
    cpu.write_reg(17, 100_000);
    board
        .write_cacheable_xword(RAM, 0x1468_0000_a2a4_6001)
        .unwrap();
    board.step_mapped(&mut cpu).unwrap();
    board.step_mapped(&mut cpu).unwrap();
    assert_eq!((cpu.pc(), cpu.npc(), cpu.read_reg(17)), (0, 4, 99_999));
    (board, cpu)
}

#[test]
fn serial_timeout_and_vector_have_the_same_boundary_as_single_stepping() {
    let mut fast = countdown(1 << 63);
    let mut reference = countdown(1 << 63);
    for board in [&mut fast.0, &mut reference.0] {
        // SAB receiver at 9600 baud, threshold 32, interrupt on timeout.
        for (reg, value) in [
            (0x2c, 0xc3),
            (0x22, 8),
            (0x28, 0x4c),
            (0x3f, 0x40),
            (0x34, 0xaf),
            (0x27, 0),
            (0x39, 0x80),
            (0x3a, 0xbf),
            (0x3b, 0xff),
        ] {
            board.serial.write(reg, value).unwrap();
        }
        board.pbm.write(0x1058, 1 << 31).unwrap(); // SAB INO 0x2b
        assert!(board.receive_serial_a(b'A'));
    }
    // 4,166,667 ns to TIME. Take even loop pairs, then one ordinary step
    // across the fractional last interval, then enter TT 0x60.
    for expected in [41_666, 1, 1] {
        assert_eq!(
            fast.0.step_or_repeat(&mut fast.1, 100_000).unwrap(),
            expected
        );
        for _ in 0..expected {
            reference.0.step_outcome(&mut reference.1).unwrap();
        }
        assert!(fast.1 == reference.1);
        assert_eq!(fast.0.elapsed_ns, reference.0.elapsed_ns);
    }
    assert_eq!(fast.1.pc(), 0x60 * 32);
    assert_eq!(fast.0.serial.read(0x3a).unwrap(), 0x40);
}

#[test]
fn tick_and_delay_slot_match_single_steps_with_small_budgets() {
    let (mut board, mut cpu) = countdown(8 * 44);
    let (mut reference, mut slow) = countdown(8 * 44);
    for (budget, expected) in [(3, 2), (1, 1), (50, 1), (50, 1), (50, 1), (50, 1)] {
        assert_eq!(board.step_or_repeat(&mut cpu, budget).unwrap(), expected);
        for _ in 0..expected {
            reference.step_outcome(&mut slow).unwrap();
        }
        assert!(cpu == slow);
        assert_eq!(board.elapsed_ns, reference.elapsed_ns);
    }
    assert_eq!(cpu.pc(), 0x4e * 32);
}

#[test]
fn synchronous_trap_counts_as_one_and_unsupported_instruction_stops_at_its_pc() {
    for (word, supported) in [(0x91d0_2001u32, true), (0x91d0_2081, false)] {
        let (mut board, mut cpu) = countdown(1 << 63);
        let (mut reference, mut slow) = countdown(1 << 63);
        for b in [&mut board, &mut reference] {
            b.write_cacheable_xword(RAM, u64::from(word) << 32).unwrap();
            b.dma_writes.push((RAM, 4));
        }
        let initial_time = board.elapsed_ns;
        let result = board.step_or_repeat(&mut cpu, 100);
        let single = reference.step_outcome(&mut slow);
        if supported {
            assert_eq!(result.unwrap(), 1);
            assert!(matches!(
                single,
                Ok(BoardStep::TrapTaken {
                    tt: 0x101,
                    fault_pc: 0,
                    ..
                })
            ));
            assert_eq!(board.elapsed_ns, initial_time + crate::machine::STEP_NS);
        } else {
            assert_eq!(result.unwrap_err(), single.unwrap_err());
            assert_eq!((cpu.pc(), board.elapsed_ns), (0, initial_time));
        }
        assert!(cpu == slow);
    }
}

#[test]
fn dma_snoop_precedes_countdown_and_zero_budget_does_no_work() {
    let (mut board, mut cpu) = countdown(1 << 63);
    let initial = cpu.clone();
    let time = board.elapsed_ns;
    board
        .write_cacheable_xword(RAM, u64::from(NOP) << 32 | u64::from(NOP))
        .unwrap();
    board.dma_writes.push((RAM, 8));
    assert_eq!(board.step_or_repeat(&mut cpu, 0).unwrap(), 0);
    assert!(cpu == initial);
    assert_eq!(board.elapsed_ns, time);
    assert_eq!(board.dma_writes.len(), 1);
    assert_eq!(board.step_or_repeat(&mut cpu, 50).unwrap(), 1);
    assert!(board.dma_writes.is_empty());
    assert_eq!((cpu.pc(), cpu.npc(), cpu.read_reg(17)), (4, 8, 99_999));
    assert_eq!(board.elapsed_ns, time + crate::machine::STEP_NS);
}

#[test]
fn demapped_countdown_takes_one_instruction_miss() {
    let (mut board, mut cpu) = countdown(1 << 63);
    let (mut reference, mut slow) = countdown(1 << 63);
    for (b, c) in [(&mut board, &mut cpu), (&mut reference, &mut slow)] {
        c.write_reg(1, 0);
        // STXA to I-MMU Demap, primary context, page zero.
        c.execute_fetched(0xc4f0_4000 | (0x57 << 5), b).unwrap();
    }
    let pc = cpu.pc();
    assert_eq!(board.step_or_repeat(&mut cpu, 100_000).unwrap(), 1);
    assert!(matches!(reference.step_outcome(&mut slow).unwrap(),
        BoardStep::TrapTaken { fault_pc, tt: 0x64, .. } if fault_pc == pc));
    assert!(cpu == slow);
    assert_eq!(board.elapsed_ns, reference.elapsed_ns);
}
