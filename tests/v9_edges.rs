use helios::v9::{V9Cpu, V9Memory};

fn one(word: u32, input: u64) -> (V9Cpu, V9Memory) {
    let mut m = V9Memory::new(64);
    m.load_words(0, &[word]);
    let mut c = V9Cpu::new(0);
    c.write_reg(1, input);
    (c, m)
}

#[test]
fn addcc_reads_aliased_input_before_writing_destination() {
    let (mut c, mut m) = one(0x82806001, u64::MAX);
    c.step(&mut m).unwrap();
    assert_eq!((c.read_reg(1), c.ccr()), (0, 0x55));
}

#[test]
fn addcc_signed_64_bit_overflow_with_distinct_destination() {
    let (mut c, mut m) = one(0x84806001, i64::MAX as u64);
    c.step(&mut m).unwrap();
    assert_eq!((c.read_reg(2), c.ccr()), (1 << 63, 0xa5));
}

#[test]
fn non_x_srl_uses_low_word_or_fails_explicitly() {
    let (mut c, mut m) = one(0x85306000, 1 << 32);
    let before = c.clone();
    match c.step(&mut m) {
        Ok(()) => assert_eq!(c.read_reg(2), 0),
        Err(_) => assert_eq!(c, before),
    }
}

#[test]
fn non_x_sll_uses_five_bit_register_count_or_fails_explicitly() {
    let (mut c, mut m) = one(0x85284004, 1);
    c.write_reg(4, 32);
    let before = c.clone();
    match c.step(&mut m) {
        Ok(()) => assert_eq!(c.read_reg(2), 1),
        Err(_) => assert_eq!(c, before),
    }
}

fn check_rejected(mut c: V9Cpu, mut m: V9Memory) {
    let before = (c.clone(), m.clone());
    assert!(c.step(&mut m).is_err(), "expected an explicit fixture stop");
    assert_eq!((c, m), before);
}

#[test]
fn misaligned_fetch_is_rejected_before_execution() {
    let mut m = V9Memory::new(16);
    m.load_words(0, &[0x00000100, 0x00000000]); // NOP bytes beginning at offset 2.
    check_rejected(V9Cpu::new(2), m);
}

#[test]
fn out_of_range_fetch_is_rejected_without_panicking() {
    check_rejected(V9Cpu::new(0x1_0000_0000), V9Memory::new(64));
}

#[test]
fn misaligned_ldx_is_rejected_before_commit() {
    let (mut c, m) = one(0xc4586000, 9);
    c.write_reg(2, 0x1122334455667788);
    check_rejected(c, m);
}

#[test]
fn misaligned_stx_is_rejected_before_commit() {
    let (mut c, m) = one(0xc4706000, 9);
    c.write_reg(2, 0x1122334455667788);
    check_rejected(c, m);
}

#[test]
fn out_of_range_ldx_is_rejected_without_panicking() {
    let (c, m) = one(0xc4586000, u64::MAX - 7);
    check_rejected(c, m);
}

#[test]
fn out_of_range_stx_is_rejected_without_panicking() {
    let (c, m) = one(0xc4706000, u64::MAX - 7);
    check_rejected(c, m);
}

#[test]
fn branch_skips_fallthrough_and_annul_skips_effect() {
    // QEMU-confirmed excerpt from edge.s. The arrival at address 16 is
    // distinguishable from falling through the poison instruction at 12.
    let mut c = V9Cpu::new(0);
    let mut m = V9Memory::new(64);
    m.load_words(
        0,
        &[
            0x80802000, 0x02800003, 0x8a016007, 0x8a016020, 0x32800003, 0x8a016020, 0x8a016001,
        ],
    );
    c.run(&mut m, 2).unwrap();
    assert_eq!((c.pc(), c.npc()), (8, 16));
    c.step(&mut m).unwrap();
    assert_eq!((c.pc(), c.npc(), c.read_reg(5)), (16, 20, 7));
    c.step(&mut m).unwrap();
    assert_eq!((c.pc(), c.npc()), (24, 28));
    c.step(&mut m).unwrap();
    assert_eq!(c.read_reg(5), 8);
}
