use helios::{
    ultra5::{BootFlash, PowerOnReset, FLASH_SIZE, POWER_ON_RESET_PC},
    v9::V9Cpu,
};

fn flash_with(words: &[(usize, u32)]) -> BootFlash {
    let mut image = vec![0; FLASH_SIZE];
    for &(offset, word) in words {
        image[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    BootFlash::new(image).unwrap()
}

fn check_obp_cwp_read(flash: &BootFlash, mut cpu: V9Cpu) {
    let initial_g3 = cpu.read_reg(3);
    flash.execute_reset_branch(&mut cpu).unwrap();
    let fetch = flash.step(&mut cpu).unwrap();
    assert_eq!(fetch.instruction, 0x8352_4000);
    assert_eq!(fetch.flash_offset, 0x210c);
    assert_eq!(fetch.physical_address, 0x01ff_f000_210c);
    assert_eq!(
        (fetch.pc, fetch.npc),
        (0xffff_ffff_f000_210c, 0xffff_ffff_f000_2110)
    );
    let mut expected = V9Cpu::from(PowerOnReset {
        pc: 0xffff_ffff_f000_2110,
        npc: 0xffff_ffff_f000_2114,
        ..PowerOnReset::ultra_sparc_ii()
    });
    expected.write_reg(1, 7);
    expected.write_reg(3, initial_g3);
    assert_eq!(cpu, expected);
}

#[test]
fn fourth_reset_word_reads_cwp_and_stops_before_the_next_word() {
    let flash = flash_with(&[
        (0x20, 0xc4d8_08a0),
        (0x24, 0xc0f0_08a0),
        (0x28, 0x3068_0839),
        (0x2c, 0x8610_2005), // annulled: mov 5, %g3
        (0x210c, 0x8352_4000),
        (0x2110, 0x8610_2009), // next: mov 9, %g3
    ]);
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    cpu.write_reg(1, u64::MAX);
    cpu.write_reg(2, 0x0800_0000);
    cpu.write_reg(3, 19);
    check_obp_cwp_read(&flash, cpu);
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_ultra5_obp_reads_cwp_at_the_branch_target() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    ))
    .unwrap();
    check_obp_cwp_read(
        &BootFlash::new(image).unwrap(),
        V9Cpu::from(PowerOnReset::ultra_sparc_ii()),
    );
}

#[test]
fn flash_step_uses_the_live_delay_slot_and_nonsequential_npc() {
    let flash = flash_with(&[
        (0x20, 0x0100_0000),
        (0x24, 0x0100_0000),
        (0x28, 0x1068_0017), // ba,pt %xcc, flash+0x84 (retain the delay slot)
        (0x2c, 0x8b52_4000), // rdpr %cwp, %g5
    ]);
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    flash.execute_reset_branch(&mut cpu).unwrap();
    let fetch = flash.step(&mut cpu).unwrap();
    assert_eq!(fetch.flash_offset, 0x2c);
    assert_eq!(fetch.npc, 0xffff_ffff_f000_0084);
    assert_eq!(cpu.read_reg(5), 7);
    assert_eq!(
        (cpu.pc(), cpu.npc()),
        (0xffff_ffff_f000_0084, 0xffff_ffff_f000_0088)
    );
}

#[test]
fn rdpr_rejections_preserve_the_faulting_state() {
    for (word, privileged, reason) in [
        (0x8352_4000, false, "privileged"),
        (0x8152_4000, false, "privileged"), // %g0 still checks privilege
        (0x8352_8000, true, "unsupported"), // CANSAVE, separate register
        (0x8352_4001, true, "unsupported"), // reserved bit 0
        (0x8352_6000, true, "unsupported"), // reserved bit 13
    ] {
        let flash = flash_with(&[(0x20, word)]);
        let mut cpu = V9Cpu::from(PowerOnReset {
            privileged,
            ..PowerOnReset::ultra_sparc_ii()
        });
        cpu.write_reg(1, 11);
        let before = cpu.clone();
        assert!(flash.step(&mut cpu).unwrap_err().contains(reason));
        assert_eq!(cpu, before);
    }
}

#[test]
fn a_failed_target_fetch_keeps_the_live_cpu_state() {
    let flash = flash_with(&[]);
    for pc in [POWER_ON_RESET_PC + 2, 0xffff_ffff_f010_0000] {
        let mut cpu = V9Cpu::from(PowerOnReset {
            pc,
            npc: pc + 4,
            ..PowerOnReset::ultra_sparc_ii()
        });
        let before = cpu.clone();
        assert!(flash.step(&mut cpu).is_err());
        assert_eq!(cpu, before);
    }
}
