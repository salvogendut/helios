use helios::v9::{V9Cpu, V9Memory};

const CODE: u64 = 0x400000;
const DATA: u64 = 0x500000;

const PROGRAM: [u32; 16] = [
    0x033f_ffff, // sethi %hi(0xffffffff), %g1
    0x8210_63ff, // or %g1, 0x3ff, %g1
    0x8328_7020, // sllx %g1, 32, %g1
    0x8330_7020, // srlx %g1, 32, %g1
    0x0700_1400, // sethi %hi(0x500000), %g3
    0x8480_6001, // addcc %g1, 1, %g2
    0x8d40_8000, // rd %ccr, %g6
    0xc470_c000, // stx %g2, [%g3]
    0xc858_c000, // ldx [%g3], %g4
    0x1080_0002, // ba target
    0x8a01_6007, // delay: add %g5, 7, %g5
    0xc270_e020, // target: stx %g1, [%g3+32]
    0xca70_e008, // stx %g5, [%g3+8]
    0xcc70_e010, // stx %g6, [%g3+16]
    0xc870_e018, // stx %g4, [%g3+24]
    0xc670_e028, // stx %g3, [%g3+40]
];

#[test]
fn v9_64bit_add_cc_branch_delay_and_stx_ldx_match_qemu() {
    let mut memory = V9Memory::new(0x600000);
    memory.load_words(CODE, &PROGRAM);
    let mut cpu = V9Cpu::new(CODE);
    cpu.run(&mut memory, PROGRAM.len()).unwrap();

    assert_eq!(cpu.read_reg(1), 0x0000_0000_ffff_ffff);
    assert_eq!(cpu.read_reg(2), 0x0000_0001_0000_0000);
    assert_eq!(cpu.read_reg(3), DATA);
    assert_eq!(cpu.read_reg(4), 0x0000_0001_0000_0000);
    assert_eq!(cpu.read_reg(5), 7);
    assert_eq!(cpu.read_reg(6), 5);
    assert_eq!(cpu.ccr(), 5);

    assert_eq!(memory.read_u64(DATA), 0x0000_0001_0000_0000);
    assert_eq!(memory.read_u64(DATA + 8), 7);
    assert_eq!(memory.read_u64(DATA + 16), 5);
    assert_eq!(memory.read_u64(DATA + 24), 0x0000_0001_0000_0000);
    assert_eq!(memory.read_u64(DATA + 32), 0x0000_0000_ffff_ffff);
    assert_eq!(memory.read_u64(DATA + 40), DATA);
    assert_eq!(cpu.pc(), CODE + 64);
    assert_eq!(cpu.npc(), CODE + 68);
}
