use helios::{
    ultra5::{BootFlash, PowerOnReset, FLASH_SIZE},
    v9::{V9Cpu, V9Memory},
};

const OBP_LDXA: u32 = 0xc4d8_08a0;
const PC: u64 = 0xffff_ffff_f000_0020;

fn flash_with(word: u32) -> BootFlash {
    let mut image = vec![0xff; FLASH_SIZE];
    image[0x20..0x24].copy_from_slice(&word.to_be_bytes());
    // If a second instruction runs, %g2 will become 1.
    image[0x24..0x28].copy_from_slice(&0x8410_2001_u32.to_be_bytes());
    BootFlash::new(image).unwrap()
}

fn verify_one_reset_read(flash: &BootFlash, cpu: &mut V9Cpu) {
    let before = cpu.clone();
    let fetch = flash.execute_first_instruction(cpu).unwrap();
    assert_eq!((fetch.pc, fetch.npc), (PC, PC + 4));
    assert_eq!(
        (fetch.physical_address, fetch.flash_offset),
        (0x01ff_f000_0020, 0x20)
    );
    assert_eq!(fetch.instruction, OBP_LDXA);
    assert_eq!((cpu.pc(), cpu.npc()), (PC + 4, PC + 8));
    assert_eq!(cpu.ccr(), before.ccr());
    for register in 0..32 {
        let expected = if register == 2 {
            0
        } else {
            before.read_reg(register)
        };
        assert_eq!(cpu.read_reg(register), expected, "r{register}");
    }
    let retired = cpu.clone();
    assert!(flash.execute_first_instruction(cpu).is_err());
    assert_eq!(*cpu, retired);
}

#[test]
fn reset_ldxa_retires_exactly_one_instruction() {
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    for register in 1..32 {
        cpu.write_reg(register, u64::MAX - u64::from(register));
    }
    verify_one_reset_read(&flash_with(OBP_LDXA), &mut cpu);
}

#[test]
fn uses_the_fetched_opcode_and_the_encoded_registers() {
    // or %g0, 7, %g3 proves the hook executes the flash word via the V9 decoder.
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    flash_with(0x8610_2007)
        .execute_first_instruction(&mut cpu)
        .unwrap();
    assert_eq!(cpu.read_reg(3), 7);

    // ldxa [%g1 + %g3] 0x45, %g1. Effective address wraps to zero before writeback.
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    cpu.write_reg(1, u64::MAX);
    cpu.write_reg(3, 1);
    flash_with(0xc2d8_48a3)
        .execute_first_instruction(&mut cpu)
        .unwrap();
    assert_eq!(cpu.read_reg(1), 0);
    assert_eq!(cpu.read_reg(3), 1);
}

#[test]
fn unsupported_accesses_leave_cpu_state_unchanged() {
    for (word, base) in [
        (0xc4d8_0880, 0),       // Immediate ASI 0x44.
        (0xc4d8_28a0, 0),       // i=1 uses %asi and simm13.
        (0xc4d8_48a0, 8),       // Correct ASI, unmapped register offset.
        (0xc0d8_48a0, 1),       // Misaligned, rd=%g0 must still check the access.
        (0xc4d8_48a0, 1 << 41), // The flash PA mask does not apply to the LSU register VA.
        (0xc0f0_08a0, 0),       // Actual second OBP word (STXA), unsupported in this slice.
        (0xc458_6000, 0),       // Ordinary LDX requires a data bus.
        (0xc470_6000, 0),       // Ordinary STX requires a data bus.
    ] {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(1, base);
        cpu.write_reg(2, u64::MAX);
        let before = cpu.clone();
        assert!(
            flash_with(word)
                .execute_first_instruction(&mut cpu)
                .is_err(),
            "{word:#x}"
        );
        assert_eq!(cpu, before);
    }
}

#[test]
fn user_mode_cannot_read_lsu_control() {
    let mut memory = V9Memory::new(4);
    memory.load_words(0, &[OBP_LDXA]);
    let mut cpu = V9Cpu::new(0);
    cpu.write_reg(2, 0x1234);
    let before = cpu.clone();
    assert!(cpu.step(&mut memory).unwrap_err().contains("privileged"));
    assert_eq!(cpu, before);
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_ultra5_obp_retires_the_first_instruction() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    ))
    .unwrap();
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    verify_one_reset_read(&BootFlash::new(image).unwrap(), &mut cpu);
    println!(
        "After one OBP instruction: %g2={:#x} PC={:#x} nPC={:#x}",
        cpu.read_reg(2),
        cpu.pc(),
        cpu.npc()
    );
}
