use helios::v9::{V9Cpu, V9Memory};

// SPARC Architecture Manual V9 A.49. Non-X SLL shifts the full register,
// selecting the count using five bits. QEMU 10.2.2 clears the result's high word.
fn run(instruction: u32, value: u64, count: u64) -> u64 {
    let mut cpu = V9Cpu::new(0);
    let mut memory = V9Memory::new(4);
    memory.load_words(0, &[instruction]);
    cpu.write_reg(1, value);
    cpu.write_reg(4, count);
    cpu.step(&mut memory).unwrap();
    cpu.read_reg(2)
}

#[test]
fn sll_immediate_zero_preserves_high_word() {
    assert_eq!(run(0x85286000, 0x1_0000_0000, 0), 0x1_0000_0000);
}

#[test]
fn sll_register_masks_count_and_preserves_high_word() {
    assert_eq!(run(0x85284004, 0x1_0000_0000, 32), 0x1_0000_0000);
}

#[test]
fn sll_can_shift_across_bit_31() {
    assert_eq!(run(0x85286001, 0x8000_0000, 0), 0x1_0000_0000);
}
