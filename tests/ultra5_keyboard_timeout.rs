use helios::ultra5::{BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard, RESET_NVRAM_SIZE};
use helios::v9::{V9Cpu, V9Memory};

#[test]
fn little_endian_xwords_preserve_byte_order_alignment_and_fault_atomicity() {
    let mut cpu = V9Cpu::from(PowerOnReset {
        pc: 0,
        npc: 4,
        ..PowerOnReset::ultra_sparc_ii()
    });
    let mut ram = V9Memory::new(128);
    cpu.write_reg(1, 112);
    cpu.write_reg(2, 0x0123_4567_89ab_cdef);
    const STORE: u32 = 0xc4f0_43a0; // stxa %g2,[%g1] 0x1d
    const LOAD: u32 = 0xc6d8_43a0; // ldxa [%g1] 0x1d,%g3
    ram.load_words(0, &[STORE, LOAD]);
    cpu.step(&mut ram).unwrap();
    assert_eq!(ram.read_word(112), 0xefcd_ab89);
    assert_eq!(ram.read_word(116), 0x6745_2301);
    ram.write_u64(112, 0x1032_5476_98ba_dcfe);
    cpu.step(&mut ram).unwrap();
    assert_eq!(cpu.read_reg(3), 0xfedc_ba98_7654_3210);
    for address in [113, 128, 1 << 44] {
        cpu.write_reg(1, address);
        for op in [LOAD, STORE] {
            ram.load_words(cpu.pc(), &[op]);
            let before = (cpu.clone(), ram.clone());
            assert!(cpu.step(&mut ram).is_err());
            assert_eq!((&cpu, &ram), (&before.0, &before.1));
        }
    }
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn cold_rom_times_out_three_times_and_reports_the_unplugged_keyboard() {
    let flash = BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap();
    let mut seed = vec![0; RESET_NVRAM_SIZE];
    seed[1] = 1;
    let mut board = Ultra5ResetBoard::new(flash, ResetNvram::new(seed).unwrap());
    let reset = PowerOnReset::ultra_sparc_ii();
    let base = reset.pc - 0x20;
    let mut cpu = V9Cpu::from(reset);
    let mut clears = Vec::new();
    let mut expiries = Vec::new();
    let mut reset_byte = None;
    let mut tx_done = None;
    for retired in 0..31_000_000 {
        match cpu.pc() - base {
            0x630 => {
                assert_eq!(cpu.read_reg(9), 0x01fe_0100_08b0);
                clears.push(retired);
            }
            0x660 => {
                let ticks = cpu.read_reg(12);
                // Eight instructions per unsuccessful poll, 6.6 ticks per instruction.
                assert!((66_667_000..66_667_054).contains(&ticks), "ticks={ticks}");
                expiries.push(retired);
            }
            0x644 => panic!("disconnected keyboard supplied a receive byte"),
            0x5e0 => {
                assert_eq!((cpu.read_reg(8), cpu.read_reg(9)), (1, 0x01ff_f130_83f8));
                assert!(reset_byte.replace(retired).is_none());
            }
            0x5f4 => tx_done = Some(retired),
            0x780 => break,
            _ => {}
        }
        board.step(&mut cpu).unwrap();
    }
    assert_eq!((cpu.pc(), cpu.npc()), (base + 0x780, base + 0x784));
    assert_eq!((clears.len(), expiries.len()), (3, 3));
    for (&clear, &expiry) in clears.iter().zip(&expiries) {
        assert!((10_101_061..10_101_073).contains(&(expiry - clear)));
    }
    let tx = reset_byte.unwrap();
    assert!(expiries[0] < tx && tx < clears[1]);
    assert!((83_200..83_210).contains(&(tx_done.unwrap() - tx)));
    let [a, b] = board.take_serial_output();
    assert!(
        a.ends_with(b"\r\nProbing keyboard Done\r\nClearing E$ Tags  Done\r\nClearing I/D TLBs ")
    );
    assert!(b.is_empty());
    println!("clear={clears:?} expiry={expiries:?} transmit={tx} drained={tx_done:?}");
}
