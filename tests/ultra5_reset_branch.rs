use helios::{
    ultra5::{BootFlash, PowerOnReset, FLASH_SIZE},
    v9::{V9Cpu, V9Memory},
};

const RESET_PREFIX: [u32; 3] = [0xc4d8_08a0, 0xc0f0_08a0, 0x3068_0839];
const TARGET: u64 = 0xffff_ffff_f000_210c;

fn check_original_prefix(flash: &BootFlash) {
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    cpu.write_reg(2, 0x0800_0000);
    cpu.write_reg(3, 17);
    cpu.write_reg(4, 31);
    let fetches = flash.execute_reset_branch(&mut cpu).unwrap();
    assert_eq!(fetches.map(|f| f.instruction), RESET_PREFIX);
    assert_eq!(fetches.map(|f| f.flash_offset), [0x20, 0x24, 0x28]);
    assert_eq!(fetches[2].pc, 0xffff_ffff_f000_0028);
    assert_eq!(fetches[2].physical_address, 0x01ff_f000_0028);

    let mut expected = V9Cpu::from(PowerOnReset {
        pc: TARGET - 12,
        npc: TARGET - 8,
        ..PowerOnReset::ultra_sparc_ii()
    });
    // Three sequential NOPs reach the same PC/nPC and elapsed CPU clock.
    let mut nops = vec![0; FLASH_SIZE];
    for word in nops[0x2100..0x210c].chunks_exact_mut(4) {
        word.copy_from_slice(&0x0100_0000u32.to_be_bytes());
    }
    let nops = BootFlash::new(nops).unwrap();
    for _ in 0..3 {
        nops.step(&mut expected).unwrap();
    }
    expected.write_reg(3, 17);
    expected.write_reg(4, 31);
    assert_eq!(cpu, expected);
}

#[test]
fn reset_branch_annuls_the_slot_and_stops_before_the_target() {
    let mut image = vec![0; FLASH_SIZE];
    for (i, word) in RESET_PREFIX.into_iter().enumerate() {
        let offset = 0x20 + 4 * i;
        image[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    image[0x2c..0x30].copy_from_slice(&0x8610_2007_u32.to_be_bytes()); // mov 7, %g3
    image[0x210c..0x2110].copy_from_slice(&0x8810_2009_u32.to_be_bytes()); // mov 9, %g4
    check_original_prefix(&BootFlash::new(image).unwrap());
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_ultra5_obp_branches_out_of_the_reset_vector() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    ))
    .unwrap();
    check_original_prefix(&BootFlash::new(image).unwrap());
}

#[test]
fn backward_bpa_keeps_nonzero_ccr_and_runs_an_unannulled_slot() {
    // addcc %g0, %g0, %g0; ba,pn %icc, 0; add %g3, 7, %g3
    let mut memory = V9Memory::new(32);
    memory.load_words(12, &[0x8080_0000, 0x1047_fffc, 0x8600_e007]);
    let mut cpu = V9Cpu::new(12);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.ccr(), 0x44);
    cpu.step(&mut memory).unwrap();
    assert_eq!((cpu.pc(), cpu.npc(), cpu.read_reg(3)), (20, 0, 0));
    cpu.step(&mut memory).unwrap();
    assert_eq!((cpu.pc(), cpu.npc(), cpu.read_reg(3)), (0, 4, 7));
    assert_eq!(cpu.ccr(), 0x44);
}

#[test]
fn original_bpa_preserves_distinct_icc_and_xcc_and_skips_a_visible_slot() {
    let mut memory = V9Memory::new(0x40);
    // addcc %g1, 1, %g0 establishes ICC.Z/C and leaves XCC clear.
    memory.load_words(0x24, &[0x8080_6001, RESET_PREFIX[2], 0x8610_2007]);
    let mut cpu = V9Cpu::new(0x24);
    cpu.write_reg(1, u64::from(u32::MAX));
    cpu.write_reg(3, 17);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.ccr(), 0x05);
    cpu.step(&mut memory).unwrap();
    assert_eq!((cpu.pc(), cpu.npc()), (0x210c, 0x2110));
    assert_eq!((cpu.ccr(), cpu.read_reg(3)), (0x05, 17));
}

#[test]
fn annulled_bpa_wraps_the_signed_disp19_limits() {
    // Both prediction hints and legal condition-code selectors give the same result.
    for base in [0x3040_0000, 0x3048_0000, 0x3060_0000, 0x3068_0000] {
        for (disp19, target) in [(0x3_ffff, 0xf_fffc), (0x4_0000, 0xffff_ffff_fff0_0000)] {
            let mut memory = V9Memory::new(4);
            memory.load_words(0, &[base | disp19]);
            let mut cpu = V9Cpu::new(0);
            cpu.step(&mut memory).unwrap();
            assert_eq!((cpu.pc(), cpu.npc()), (target, target + 4));
        }
    }
}

#[test]
fn unsupported_bpcc_preserves_state_after_the_first_two_words() {
    for word in [0x3258_0839_u32, 0x3058_0839, 0x3078_0839] {
        // Reserved cc selectors 1 and 3, including a conditional branch.
        let mut image = vec![0; FLASH_SIZE];
        image[0x20..0x24].copy_from_slice(&RESET_PREFIX[0].to_be_bytes());
        image[0x24..0x28].copy_from_slice(&RESET_PREFIX[1].to_be_bytes());
        image[0x28..0x2c].copy_from_slice(&word.to_be_bytes());
        let flash = BootFlash::new(image).unwrap();
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(2, 17);
        let mut expected = cpu.clone();
        flash.execute_first_two_instructions(&mut expected).unwrap();
        assert!(flash.execute_reset_branch(&mut cpu).is_err());
        assert_eq!(cpu, expected);
    }
}
