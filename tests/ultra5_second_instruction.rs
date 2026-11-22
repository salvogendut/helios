use helios::{
    ultra5::{BootFlash, PowerOnReset, FLASH_SIZE},
    v9::V9Cpu,
};

fn flash_pair(first: u32, second: u32) -> BootFlash {
    let mut image = vec![0; FLASH_SIZE];
    image[0x20..0x24].copy_from_slice(&first.to_be_bytes());
    image[0x24..0x28].copy_from_slice(&second.to_be_bytes());
    BootFlash::new(image).unwrap()
}

#[test]
fn stxa_uses_its_source_and_both_address_registers() {
    // stxa %g3, [%g1 + %g4] 0x45; ldxa [%g0] 0x45, %g2
    let flash = flash_pair(0xc6f0_48a4, 0xc4d8_08a0);
    for value in [1, 2, 3] {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(1, 0u64.wrapping_sub(8));
        cpu.write_reg(3, value);
        cpu.write_reg(4, 8);
        flash.execute_first_two_instructions(&mut cpu).unwrap();
        assert_eq!((cpu.read_reg(2), cpu.read_reg(3)), (value, value));
    }
}

#[test]
fn a_rejected_second_store_preserves_the_retired_first_instruction() {
    for (store, offset, data) in [
        (0xc6f0_48a0, 1, 1),       // unaligned address
        (0xc6f0_48a0, 8, 1),       // aligned but unmapped register
        (0xc6f0_48a0, 1 << 41, 1), // no flash PA mask on ASI register addresses
        (0xc6f0_0880, 0, 1),       // wrong ASI
        (0xc6f0_28a0, 0, 1),       // %asi + simm13 form
        (0xc6f0_08a0, 0, 0x10),    // unsupported parity-mask field
        (0xc6f0_08a0, 0, 1 << 63), // reserved high bit
    ] {
        // The first store gives LSU a nonzero value so premature clearing is observable.
        let flash = flash_pair(0xc8f0_08a0, store);
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(1, offset);
        cpu.write_reg(3, data);
        cpu.write_reg(4, 2);
        let mut expected = cpu.clone();
        flash.execute_first_instruction(&mut expected).unwrap();
        assert!(flash.execute_first_two_instructions(&mut cpu).is_err());
        assert_eq!(cpu, expected);
    }
}

#[test]
fn user_store_fails_even_when_the_source_is_zero() {
    let flash = flash_pair(0xc0f0_08a0, 0x0100_0000);
    let mut cpu = V9Cpu::new(0xffff_ffff_f000_0020);
    let before = cpu.clone();
    assert!(flash
        .execute_first_two_instructions(&mut cpu)
        .unwrap_err()
        .contains("privileged"));
    assert_eq!(cpu, before);
}

#[test]
fn a_branch_leaves_its_delay_slot_for_the_caller() {
    // ba POR+0x10 establishes a nonsequential nPC at the second fetch boundary.
    let flash = flash_pair(0x1080_0004, 0x8410_2007);
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    let mut expected = cpu.clone();
    flash.execute_first_instruction(&mut expected).unwrap();
    assert_eq!(
        (expected.pc(), expected.npc()),
        (0xffff_ffff_f000_0024, 0xffff_ffff_f000_0030)
    );
    assert!(flash.execute_first_two_instructions(&mut cpu).is_err());
    assert_eq!(cpu, expected);
}
