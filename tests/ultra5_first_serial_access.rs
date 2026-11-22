use helios::ultra5::{
    BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard, POWER_ON_RESET_PC, RESET_NVRAM_SIZE,
};
use helios::v9::{V9Cpu, V9Memory};

fn cpu_at(pc: u64) -> V9Cpu {
    PowerOnReset {
        pc,
        npc: pc + 4,
        ..PowerOnReset::ultra_sparc_ii()
    }
    .into()
}

fn execute(cpu: &mut V9Cpu, mem: &mut V9Memory, word: u32) {
    mem.load_words(cpu.pc(), &[word]);
    cpu.step(mem).unwrap();
}

#[test]
fn five_trap_levels_keep_independent_saved_pcs_and_types() {
    let mut cpu = cpu_at(0);
    let mut mem = V9Memory::new(256);
    execute(&mut cpu, &mut mem, 0x8351_c000); // rdpr %tl,%g1
    assert_eq!(cpu.read_reg(1), 5);
    execute(&mut cpu, &mut mem, 0x8350_c000);
    assert_eq!(cpu.read_reg(1), 1); // POR TT at MAXTL
    for level in 1..=5 {
        execute(&mut cpu, &mut mem, 0x8f90_2000 | level);
        let tag = u64::from(level) * 8;
        cpu.write_reg(1, 0x0000_0800_0000_0003 | tag);
        cpu.write_reg(2, 0x0000_0100_0000_0005 | tag);
        cpu.write_reg(3, 0x301 | tag);
        for word in [0x8190_4000, 0x8390_8000, 0x8790_c000] {
            execute(&mut cpu, &mut mem, word);
        }
    }
    for level in (1..=5).rev() {
        execute(&mut cpu, &mut mem, 0x8f90_2000 | level);
        for word in [0x8350_0000, 0x8550_4000, 0x8750_c000] {
            execute(&mut cpu, &mut mem, word);
        }
        let tag = u64::from(level) * 8;
        assert_eq!(cpu.read_reg(1), 0xffff_f800_0000_0000 | tag);
        assert_eq!(cpu.read_reg(2), 0x0000_0100_0000_0004 | tag);
        assert_eq!(cpu.read_reg(3), 0x101 | tag);
    }
    execute(&mut cpu, &mut mem, 0x8f90_2000);
    for word in [
        0x8350_0000,
        0x8550_4000,
        0x8750_c000,
        0x8190_3fff,
        0x8390_3fff,
        0x8790_3fff,
        0x8f90_2006,
    ] {
        mem.load_words(cpu.pc(), &[word]);
        let before = cpu.clone();
        assert!(cpu.step(&mut mem).is_err());
        assert_eq!(cpu, before);
    }
}

#[test]
fn call_and_jmpl_link_at_the_instruction_and_execute_the_delay_slot() {
    let mut cpu = cpu_at(0x10);
    let mut mem = V9Memory::new(64);
    mem.load_words(0x10, &[0x7fff_fffc, 0x8203_e008]); // call 0; add %o7,8,%g1
    mem.load_words(0, &[0x9fc3_e008, 0x8403_e000]); // jmpl %o7+8,%o7; add %o7,0,%g2
    for (pc, npc, link) in [(0x14, 0, 0x10), (0, 4, 0x10), (4, 0x18, 0), (0x18, 0x1c, 0)] {
        cpu.step(&mut mem).unwrap();
        assert_eq!((cpu.pc(), cpu.npc(), cpu.read_reg(15)), (pc, npc, link));
    }
    assert_eq!((cpu.read_reg(1), cpu.read_reg(2)), (0x18, 0));
    cpu.write_reg(15, 1);
    mem.load_words(cpu.pc(), &[0x9fc3_e008]);
    let before = cpu.clone();
    assert!(cpu.step(&mut mem).is_err());
    assert_eq!(cpu, before);
}

#[test]
fn alternate_byte_and_le_word_accesses_preserve_lanes_and_fault_atomicity() {
    let mut cpu = cpu_at(0);
    let mut mem = V9Memory::new(128);
    cpu.write_reg(1, 0x40);
    cpu.write_reg(2, 0xfeed_cafe_1122_3380);
    execute(&mut cpu, &mut mem, 0xc4a0_43a0); // sta %g2,[%g1] ASI 0x1d
    assert_eq!(mem.read_word(0x40), 0x8033_2211);
    execute(&mut cpu, &mut mem, 0xc688_42a0); // lduba [%g1] ASI 0x15,%g3
    assert_eq!(cpu.read_reg(3), 0x80);
    cpu.write_reg(1, 0x42);
    execute(&mut cpu, &mut mem, 0xc4a8_42a0); // stba %g2,[%g1] ASI 0x15
    assert_eq!(mem.read_word(0x40), 0x8033_8011);
    for (addr, word) in [
        (0x42, 0xc4a0_43a0),
        (128, 0xc4a8_42a0),
        (1 << 44, 0xc688_42a0),
        (0x40, 0xc4a0_43c0),
    ] {
        cpu.write_reg(1, addr);
        mem.load_words(cpu.pc(), &[word]);
        let before = (cpu.clone(), mem.clone());
        assert!(cpu.step(&mut mem).is_err());
        assert_eq!((&cpu, &mem), (&before.0, &before.1));
    }
    let mut user = V9Cpu::new(0);
    for word in [0xc4a0_43a0, 0xc688_42a0, 0xc4a8_42a0] {
        mem.load_words(0, &[word]);
        let before = (user.clone(), mem.clone());
        assert!(user.step(&mut mem).unwrap_err().contains("privileged"));
        assert_eq!((&user, &mem), (&before.0, &before.1));
    }
}

fn original_flash() -> BootFlash {
    BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap()
}

// QEMU's generic sun4u reports the physical flash alias. Compare flash-relative
// PC, nPC and original word in big-endian order on the common 181-instruction path.
fn hash_step(mut hash: u64, step: helios::ultra5::FlashFetch) -> u64 {
    let vector = POWER_ON_RESET_PC - 0x20;
    for v in [
        (step.pc - vector) as u32,
        (step.npc - vector) as u32,
        step.instruction,
    ] {
        for b in v.to_be_bytes() {
            hash = (hash ^ u64::from(b)).wrapping_mul(1_099_511_628_211);
        }
    }
    hash
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn obp_retires_the_first_channel_a_ccr0_write_on_both_reset_paths() {
    let flash = original_flash();
    let vector = POWER_ON_RESET_PC - 0x20;
    for (marker, retired) in [([0; 8], 219), (*b"soft-rst", 228)] {
        let mut nvram = vec![0; RESET_NVRAM_SIZE];
        nvram[0x10..0x18].copy_from_slice(&marker);
        let mut board = Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(nvram).unwrap());
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        let mut hash = 0xcbf2_9ce4_8422_2325;
        for n in 1..retired {
            let pc = cpu.pc();
            let step = board
                .step(&mut cpu)
                .unwrap_or_else(|err| panic!("PC={pc:#x}: {err}"));
            if n > retired - 181 {
                hash = hash_step(hash, step);
            }
            assert_eq!(board.serial_ccr0(), [0, 0]);
        }
        assert_eq!(
            (cpu.pc(), cpu.npc(), cpu.read_reg(12), cpu.read_reg(11)),
            (vector + 0x33c, vector + 0x340, 0x01ff_f140_002c, 0xc3)
        );
        let fetch = board.step(&mut cpu).unwrap();
        assert_eq!(fetch.instruction, 0xd6ab_2000);
        assert_eq!(hash_step(hash, fetch), 0xcf95_e55a_ca0a_8416);
        assert_eq!(board.serial_ccr0(), [0xc3, 0]);
        assert_eq!((cpu.pc(), cpu.npc()), (vector + 0x340, vector + 0x344));
        println!(
            "retired={retired} PC={:#x} word={:#010x} STBA ASI=0x15 PA={:#x} width=1 value={:#04x}",
            fetch.pc,
            fetch.instruction,
            cpu.read_reg(12),
            cpu.read_reg(11)
        );
    }
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_power_on_diagnostics_are_transmitted_only_when_enabled() {
    let flash = original_flash();
    let base = POWER_ON_RESET_PC - 0x20;
    for (marker, line) in [
        ([0; 8], b"Hardware Power ON\r\n"),
        (*b"soft-rst", b"Software Power ON\r\n"),
    ] {
        for enabled in [false, true] {
            let mut seed = vec![0; RESET_NVRAM_SIZE];
            seed[1] = u8::from(enabled);
            seed[16..24].copy_from_slice(&marker);
            let mut board = Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(seed).unwrap());
            let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
            let mut ttya = Vec::new();
            let mut first = None;
            let mut last = None;
            let mut staged = None;
            for n in 1..=1500 {
                if cpu.pc() == base + 0x18fc {
                    break;
                }
                let pc = cpu.pc();
                if pc == base + 0x368 {
                    assert_eq!(cpu.read_reg(9), 0x01ff_f140_0000);
                    assert!(staged.replace(cpu.read_reg(8) as u8).is_none());
                }
                board
                    .step(&mut cpu)
                    .unwrap_or_else(|e| panic!("PC={pc:#x} {e}"));
                let [a, b] = board.take_serial_output();
                assert!(b.is_empty());
                if !a.is_empty() {
                    assert_eq!(pc, base + 0x380);
                    assert_eq!((cpu.read_reg(9), cpu.read_reg(10)), (0x01ff_f140_0000, 8));
                    assert_eq!(a.as_slice(), &[staged.take().unwrap()]);
                    first.get_or_insert(n);
                    last = Some(n);
                    ttya.extend(a);
                }
            }
            assert_eq!(cpu.pc(), base + 0x18fc);
            assert_eq!(
                (cpu.read_reg(2), cpu.read_reg(1)),
                (0x01ff_f172_6000, u64::MAX)
            );
            assert!(staged.is_none());
            assert_eq!(ttya.as_slice(), if enabled { line.as_slice() } else { &[] });
            assert_eq!(board.serial_ccr0(), [0xc3, 0xc3]);
            println!(
                "marker={marker:?} enabled={enabled} first={first:?} last={last:?} ttya={:?}",
                String::from_utf8(ttya).unwrap()
            );
        }
    }
}
