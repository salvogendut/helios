use helios::{
    ultra5::{BootFlash, PowerOnReset, FLASH_SIZE},
    v9::{V9Cpu, V9Memory},
};

fn reset_cpu_at(pc: u64, npc: u64) -> V9Cpu {
    V9Cpu::from(PowerOnReset {
        pc,
        npc,
        ..PowerOnReset::ultra_sparc_ii()
    })
}

fn execute(cpu: &mut V9Cpu, memory: &mut V9Memory, word: u32) {
    memory.load_words(cpu.pc(), &[word]);
    cpu.step(memory).unwrap();
}

fn select(cpu: &mut V9Cpu, memory: &mut V9Memory, window: u8) {
    execute(cpu, memory, 0x9390_2000 | u32::from(window));
}

fn check_firmware_pair(flash: &BootFlash, mut cpu: V9Cpu) {
    flash.execute_reset_branch(&mut cpu).unwrap();
    flash.step(&mut cpu).unwrap();
    assert_eq!((cpu.current_window(), cpu.read_reg(1)), (7, 7));
    let saved_ccr = cpu.ccr();
    let old_outs: [u64; 8] = std::array::from_fn(|i| cpu.read_reg(8 + i as u8));
    let mut expected = cpu.clone();
    // Advance a separate copy through two NOPs to establish the expected full state.
    let mut nop_flash = vec![0; FLASH_SIZE];
    nop_flash[0x2110..0x2118].copy_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0]);
    let nops = BootFlash::new(nop_flash).unwrap();
    nops.step(&mut expected).unwrap();
    nops.step(&mut expected).unwrap();

    let to_zero = flash.step(&mut cpu).unwrap();
    assert_eq!(
        (to_zero.flash_offset, to_zero.instruction),
        (0x2110, 0x9390_2000)
    );
    assert_eq!(
        (cpu.current_window(), cpu.read_reg(1), cpu.ccr()),
        (0, 7, saved_ccr)
    );
    assert_eq!(
        std::array::from_fn(|i| cpu.read_reg(24 + i as u8)),
        old_outs
    );
    assert_eq!(
        (cpu.pc(), cpu.npc()),
        (0xffff_ffff_f000_2114, 0xffff_ffff_f000_2118)
    );

    let restore = flash.step(&mut cpu).unwrap();
    assert_eq!(
        (restore.flash_offset, restore.instruction),
        (0x2114, 0x9390_6000)
    );
    assert_eq!(
        (cpu.pc(), cpu.npc()),
        (0xffff_ffff_f000_2118, 0xffff_ffff_f000_211c)
    );
    assert_eq!(cpu, expected);
}

#[test]
fn obp_pair_selects_zero_then_restores_the_saved_window() {
    let mut image = vec![0; FLASH_SIZE];
    for (off, word) in [
        (0x20, 0xc4d8_08a0_u32),
        (0x24, 0xc0f0_08a0),
        (0x28, 0x3068_0839),
        (0x210c, 0x8352_4000),
        (0x2110, 0x9390_2000),
        (0x2114, 0x9390_6000),
        (0x2118, 0x8610_2009), // mov 9, %g3 must remain unexecuted.
    ] {
        image[off..off + 4].copy_from_slice(&word.to_be_bytes());
    }
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    for reg in 1..32 {
        cpu.write_reg(reg, u64::MAX - u64::from(reg));
    }
    check_firmware_pair(&BootFlash::new(image).unwrap(), cpu);
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_ultra5_obp_switches_windows_and_stops_after_six_words() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    ))
    .unwrap();
    check_firmware_pair(
        &BootFlash::new(image).unwrap(),
        V9Cpu::from(PowerOnReset::ultra_sparc_ii()),
    );
}

#[test]
fn every_window_keeps_locals_and_shares_outs_with_the_next_windows_ins() {
    let mut cpu = reset_cpu_at(0, 4);
    let mut memory = V9Memory::new(512);
    for reg in 0..8 {
        cpu.write_reg(reg, u64::MAX - u64::from(reg));
    }
    for w in 0..8 {
        select(&mut cpu, &mut memory, w);
        for lane in 0..8 {
            let tag = 0x1234_5678_0000_0000 | (u64::from(w) << 8) | u64::from(lane);
            cpu.write_reg(8 + lane, tag);
            cpu.write_reg(16 + lane, !tag);
        }
    }
    for w in [4u8, 0, 6, 3, 5, 7, 1, 2] {
        select(&mut cpu, &mut memory, w);
        assert_eq!(cpu.read_reg(0), 0);
        for reg in 1..8 {
            assert_eq!(cpu.read_reg(reg), u64::MAX - u64::from(reg));
        }
        for lane in 0..8 {
            let own = 0x1234_5678_0000_0000 | (u64::from(w) << 8) | u64::from(lane);
            let prev = (w + 7) % 8;
            let incoming = 0x1234_5678_0000_0000 | (u64::from(prev) << 8) | u64::from(lane);
            assert_eq!(cpu.read_reg(8 + lane), own);
            assert_eq!(cpu.read_reg(16 + lane), !own);
            assert_eq!(cpu.read_reg(24 + lane), incoming);
        }
    }
    for w in 0..8 {
        select(&mut cpu, &mut memory, w);
        for lane in 0..8 {
            cpu.write_reg(24 + lane, u64::from(w) * 8 + u64::from(lane));
        }
        select(&mut cpu, &mut memory, (w + 7) % 8);
        for lane in 0..8 {
            assert_eq!(cpu.read_reg(8 + lane), u64::from(w) * 8 + u64::from(lane));
        }
    }
}

#[test]
fn wrpr_xors_old_window_operands_and_keeps_only_three_bits() {
    let mut cpu = reset_cpu_at(0, 4);
    let mut memory = V9Memory::new(128);
    execute(&mut cpu, &mut memory, 0x8080_0000); // addcc %g0,%g0,%g0
    cpu.write_reg(16, u64::MAX - 2); // %l0 low bits = 5
    cpu.write_reg(31, 0xff); // %i7 low bits = 7
    execute(&mut cpu, &mut memory, 0x9394_001f); // wrpr %l0,%i7,%cwp
    assert_eq!(cpu.current_window(), 2);
    assert_eq!(cpu.read_reg(16), 0);
    cpu.write_reg(16, 0x12);
    execute(&mut cpu, &mut memory, 0x9394_3fff); // wrpr %l0,-1,%cwp
    assert_eq!(cpu.current_window(), 5);
    execute(&mut cpu, &mut memory, 0x8752_4000); // rdpr %cwp,%g3
    assert_eq!(cpu.read_reg(3), 5);
    assert_eq!(cpu.ccr(), 0x44);
}

#[test]
fn wrpr_in_a_delay_slot_preserves_the_pending_target() {
    let mut cpu = reset_cpu_at(0, 4);
    let mut memory = V9Memory::new(32);
    execute(&mut cpu, &mut memory, 0x1068_0005); // ba,pt %xcc, 20
    select(&mut cpu, &mut memory, 3);
    assert_eq!((cpu.current_window(), cpu.pc(), cpu.npc()), (3, 20, 24));
}

#[test]
fn guest_window_alias_program_matches_qemu() {
    let mut cpu = reset_cpu_at(0, 4);
    let mut memory = V9Memory::new(44);
    memory.load_words(
        0,
        &[
            0x9010_2171, // mov 0x171,%o0
            0xa010_2271, // mov 0x271,%l0
            0x9390_2000, // wrpr %g0,0,%cwp
            0x8610_0018, // mov %i0,%g3
            0xb010_22e2, // mov 0x2e2,%i0
            0xa010_2333, // mov 0x333,%l0
            0x9390_3fff, // wrpr %g0,-1,%cwp
            0x8a10_0008, // mov %o0,%g5
            0x8c10_0010, // mov %l0,%g6
            0x9390_2000, // wrpr %g0,0,%cwp
            0x8e10_0010, // mov %l0,%g7
        ],
    );
    for expected_window in [7, 7, 0, 0, 0, 0, 7, 7, 7, 0, 0] {
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.current_window(), expected_window);
    }
    assert_eq!(
        [3, 5, 6, 7].map(|r| cpu.read_reg(r)),
        [0x171, 0x2e2, 0x271, 0x333]
    );
}

#[test]
fn rejected_wrpr_preserves_windows_and_control_state() {
    for (privileged, word, error) in [
        (false, 0x9390_2000, "privileged"),
        (false, 0x9390_0001, "privileged"),
        (true, 0x9590_2000, "unsupported"), // CANSAVE
        (true, 0x9190_2000, "unsupported"), // PIL
        (true, 0x9390_0020, "unsupported"), // reserved register-form bit 5
        (true, 0x9390_1000, "unsupported"), // reserved register-form bit 12
    ] {
        let mut cpu = if privileged {
            reset_cpu_at(0, 20)
        } else {
            V9Cpu::new(0)
        };
        cpu.write_reg(1, 0);
        cpu.write_reg(8, u64::MAX);
        let before = cpu.clone();
        let mut memory = V9Memory::new(4);
        memory.load_words(0, &[word]);
        assert!(cpu.step(&mut memory).unwrap_err().contains(error));
        assert_eq!(cpu, before);
    }
}
