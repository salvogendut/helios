use helios::ultra5::{
    BootFlash, PowerOnReset, ResetNvram, FLASH_BASE, FLASH_SIZE, POWER_ON_RESET_PC,
    RESET_NVRAM_SIZE,
};
use helios::v9::{V9Cpu, V9Memory};

const VECTOR: u64 = POWER_ON_RESET_PC - 0x20;
const SETUP: [u32; 10] = [
    0x9190_200f, // wrpr 15, %pil
    0x8d90_2005, // wrpr 5, %pstate
    0x8b90_2000, // wrpr 0, %tba
    0xad80_2000, // wr 0, %softint
    0x9990_2007, // wrpr 7, %cleanwin
    0x9b90_2000, // wrpr 0, %otherwin
    0x9d90_2000, // wrpr 0, %wstate
    0x9390_2000, // wrpr 0, %cwp
    0x9790_2000, // wrpr 0, %canrestore
    0x9590_2006, // wrpr 6, %cansave
];

fn flash_at(pc: u64, words: &[u32]) -> BootFlash {
    let mut bytes = vec![0; FLASH_SIZE];
    let start = (pc - VECTOR) as usize;
    for (dst, word) in bytes[start..].chunks_exact_mut(4).zip(words) {
        dst.copy_from_slice(&word.to_be_bytes());
    }
    BootFlash::new(bytes).unwrap()
}

fn cpu_at(pc: u64) -> V9Cpu {
    PowerOnReset {
        pc,
        npc: pc + 4,
        ..PowerOnReset::ultra_sparc_ii()
    }
    .into()
}

fn step_word(cpu: &mut V9Cpu, memory: &mut V9Memory, word: u32) {
    memory.load_words(cpu.pc(), &[word]);
    cpu.step(memory).unwrap();
}

// Use guest register reads on a copy. Indexes follow the ten original writes.
fn read_setup(cpu: &V9Cpu) -> [u64; 10] {
    let words = [
        0x8352_0000,
        0x8351_8000,
        0x8351_4000,
        0x8345_8000,
        0x8353_0000,
        0x8353_4000,
        0x8353_8000,
        0x8352_4000,
        0x8352_c000,
        0x8352_8000,
    ];
    let probe = flash_at(cpu.pc(), &words);
    let mut copy = cpu.clone();
    std::array::from_fn(|_| {
        probe.step(&mut copy).unwrap();
        copy.read_reg(1)
    })
}

#[test]
fn reset_writes_replace_only_the_selected_register() {
    let mut cpu = cpu_at(VECTOR);
    let seed = flash_at(
        VECTOR,
        &[
            0x8b90_c000,
            0xad81_0000, // wrpr %g3,%tba; wr %g4,%softint
            0x9b90_2003,
            0x9d90_202e,
            0x9790_2002,
            0x9590_2001,
            0x9990_2002,
            0x9190_2003,
            0x8080_0000, // addcc %g0,%g0,%g0
        ],
    );
    cpu.write_reg(3, 0xffff_ffff_ff01_ffff);
    cpu.write_reg(4, 0x8001);
    for _ in 0..9 {
        seed.step(&mut cpu).unwrap();
    }
    let mut expected = [3, 0x35, 0xffff_ffff_ff01_8000, 0x8001, 2, 3, 0x2e, 7, 2, 1];
    assert_eq!(read_setup(&cpu), expected);
    let group = flash_at(cpu.pc(), &SETUP);
    let values = [15, 5, 0, 0, 7, 0, 0, 0, 0, 6];
    for (i, word) in SETUP.into_iter().enumerate() {
        let pc = cpu.pc();
        let fetch = group.step(&mut cpu).unwrap();
        assert_eq!(
            (fetch.instruction, cpu.pc(), cpu.npc(), cpu.ccr()),
            (word, pc + 4, pc + 8, 0x44)
        );
        expected[i] = values[i];
        assert_eq!(read_setup(&cpu), expected, "after {word:08x}");
    }
}

#[test]
fn wrpr_uses_xor_and_applies_each_register_width() {
    let mut cpu = cpu_at(0);
    let mut mem = V9Memory::new(128);
    cpu.write_reg(2, 0x6e);
    cpu.write_reg(3, 0x13); // XOR = 0x7d, including bits above each implemented field.
    for (reg, value) in [
        (8, 0xd),
        (9, 5),
        (10, 5),
        (11, 5),
        (12, 5),
        (13, 5),
        (14, 0x3d),
    ] {
        step_word(&mut cpu, &mut mem, 0x8190_8003 | (reg << 25));
        step_word(&mut cpu, &mut mem, 0x8350_0000 | (reg << 14));
        assert_eq!(cpu.read_reg(1), value);
    }
    for word in [0xad80_8003, 0x8345_8000] {
        step_word(&mut cpu, &mut mem, word);
    }
    assert_eq!(cpu.read_reg(1), 0x7d);
    step_word(&mut cpu, &mut mem, 0x9990_bfff); // %g2 XOR sign-extended -1
    step_word(&mut cpu, &mut mem, 0x8353_0000);
    assert_eq!(cpu.read_reg(1), 1);
}

#[test]
fn pstate_switches_global_banks_while_preserving_windows_and_zero() {
    let mut cpu = cpu_at(0);
    let mut mem = V9Memory::new(128);
    cpu.write_reg(16, 0xcafe);
    for (state, value) in [(5, 11), (4, 22), (0x404, 33), (0x804, 44)] {
        step_word(&mut cpu, &mut mem, 0x8d90_2000 | state);
        assert_eq!(cpu.read_reg(1), 0);
        cpu.write_reg(0, 0x55);
        cpu.write_reg(1, value);
    }
    for (state, value) in [(0x404, 33), (0x804, 44), (5, 11), (4, 22)] {
        step_word(&mut cpu, &mut mem, 0x8d90_2000 | state);
        step_word(&mut cpu, &mut mem, 0x8551_8000);
        assert_eq!(
            (
                cpu.read_reg(0),
                cpu.read_reg(1),
                cpu.read_reg(2),
                cpu.read_reg(16),
                cpu.current_window()
            ),
            (0, value, u64::from(state), 0xcafe, 7)
        );
    }
    // The source belongs to the old bank even when the write selects a new one.
    cpu.write_reg(1, 0x404);
    step_word(&mut cpu, &mut mem, 0x8d90_4000);
    assert_eq!(cpu.read_reg(1), 33);
}

#[test]
fn unsupported_and_unprivileged_accesses_leave_the_faulting_cpu_intact() {
    let mut cpu = cpu_at(0);
    let mut mem = V9Memory::new(64);
    cpu.write_reg(2, 1 << 44);
    for word in [
        0x8d90_2405,
        0x8d90_2007,
        0x8d90_200d,
        0x8d90_2205, // multiple banks, IE, AM, CLE
        0x8b90_8000,
        0xad80_8000, // TBA in VA hole, reserved SOFTINT bits
        0xad80_0020,
        0x8345_8001,
        0x9990_0020,
        0x8353_0001, // malformed operands
    ] {
        mem.load_words(cpu.pc(), &[word]);
        let before = cpu.clone();
        assert!(cpu.step(&mut mem).is_err(), "{word:08x}");
        assert_eq!(cpu, before);
    }
    step_word(&mut cpu, &mut mem, 0x8d90_2001); // drop PRIV, keep alternate globals
    for word in [
        0x8151_8000,
        0x8d90_2005,
        0x8145_8000,
        0xad80_2000,
        0xc4d8_08a0,
    ] {
        mem.load_words(cpu.pc(), &[word]);
        let before = cpu.clone();
        assert!(cpu.step(&mut mem).unwrap_err().contains("privileged"));
        assert_eq!(cpu, before);
    }
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_sets_up_interrupt_and_window_registers_on_both_paths() {
    let flash = BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap();
    for (marker, prefix) in [([0; 8], 28), (*b"soft-rst", 37)] {
        let mut bytes = vec![0; RESET_NVRAM_SIZE];
        bytes[0x10..0x18].copy_from_slice(&marker);
        let mut nvram = ResetNvram::new(bytes).unwrap();
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        for _ in 0..prefix {
            flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
        }
        let ccr = cpu.ccr();
        let mut expected = [0, 0x35, 0, 0, 0, 0, 0, 7, 0, 0];
        assert_eq!(read_setup(&cpu), expected);
        let values = [15, 5, 0, 0, 7, 0, 0, 0, 0, 6];
        for (i, word) in SETUP.into_iter().enumerate() {
            let off = 0x15ec + i * 4;
            let fetch = flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
            let next = VECTOR + off as u64 + 4;
            assert_eq!(
                (
                    fetch.flash_offset,
                    fetch.instruction,
                    cpu.pc(),
                    cpu.npc(),
                    cpu.ccr()
                ),
                (off, word, next, next + 4, ccr)
            );
            expected[i] = values[i];
            assert_eq!(read_setup(&cpu), expected);
        }
        assert_eq!(cpu.pc(), VECTOR + 0x1614);
        assert_eq!(flash.read_word(FLASH_BASE + 0x1614).unwrap(), 0x8f90_2005);
    }
}
