use helios::{
    ultra5::{
        BootFlash, PowerOnReset, ResetNvram, FLASH_BASE, FLASH_SIZE, RESET_NVRAM_BASE,
        RESET_NVRAM_SIZE,
    },
    v9::{V9Cpu, V9Memory},
};

const VECTOR: u64 = 0xffff_ffff_f000_0000;
const PROBE: u32 = 0xd2da_82a0; // ldxa [%o2 + %g0] 0x15,%o1
const PREFIX: [(usize, u32); 6] = [
    (0x20, 0xc4d8_08a0),
    (0x24, 0xc0f0_08a0),
    (0x28, 0x3068_0839),
    (0x210c, 0x8352_4000),
    (0x2110, 0x9390_2000),
    (0x2114, 0x9390_6000),
];
// Instruction, destination and independent QEMU single-step result.
const SETUP: [(u32, u8, u64); 11] = [
    (0x131c_dbd9, 9, 0x736f_6400),
    (0x9212_6274, 9, 0x736f_6674),
    (0x932a_7020, 9, 0x736f_6674_0000_0000),
    (0x110b_5c9c, 8, 0x2d72_7000),
    (0x9012_2374, 8, 0x2d72_7374),
    (0x9012_0009, 8, u64::from_be_bytes(*b"soft-rst")),
    (0x9210_21ff, 9, 0x1ff),
    (0x932a_7020, 9, 0x01ff_0000_0000),
    (0x153c_4000, 10, 0xf100_0000),
    (0x9412_a010, 10, 0xf100_0010),
    (0x9412_8009, 10, 0x01ff_f100_0010),
];

fn verify_reset_probe(flash: &BootFlash) {
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    for (off, word) in PREFIX {
        let fetch = flash.step(&mut cpu).unwrap();
        assert_eq!((fetch.flash_offset, fetch.instruction), (off, word));
    }
    let mut regs: [u64; 32] = std::array::from_fn(|r| cpu.read_reg(r as u8));
    let ccr = cpu.ccr();
    for (i, (word, rd, value)) in SETUP.into_iter().enumerate() {
        let offset = 0x2118 + 4 * i as u64;
        let fetch = flash.step(&mut cpu).unwrap();
        assert_eq!(fetch.instruction, word);
        assert_eq!(
            (fetch.pc, fetch.npc),
            (VECTOR + offset, VECTOR + offset + 4)
        );
        assert_eq!(fetch.physical_address, FLASH_BASE + offset);
        assert_eq!((cpu.current_window(), cpu.ccr()), (7, ccr));
        regs[usize::from(rd)] = value;
        assert_eq!(std::array::from_fn(|r| cpu.read_reg(r as u8)), regs);
    }
    assert_eq!((cpu.pc(), cpu.npc()), (VECTOR + 0x2144, VECTOR + 0x2148));
    assert_eq!(flash.read_word(FLASH_BASE + 0x2144).unwrap(), PROBE);
    let before = cpu.clone();
    assert_eq!(
        flash.step(&mut cpu).unwrap_err(),
        "unmapped V9 physical read: ASI=0x15 PA=0x1fff1000010 width=8"
    );
    assert_eq!(cpu, before);
}

fn synthetic_flash() -> BootFlash {
    let mut image = vec![0xff; FLASH_SIZE];
    for (off, word) in PREFIX
        .into_iter()
        .chain(SETUP.iter().enumerate().map(|(i, t)| (0x2118 + i * 4, t.0)))
        .chain([
            (0x2144, PROBE),
            (0x2148, 0x80a2_0009),
            (0x214c, 0x1268_001d),
            (0x2150, 0x0100_0000),
        ])
    {
        image[off..off + 4].copy_from_slice(&word.to_be_bytes());
    }
    BootFlash::new(image).unwrap()
}

#[test]
fn firmware_prefix_reaches_its_first_board_read() {
    verify_reset_probe(&synthetic_flash());
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_reaches_the_same_unretired_board_read() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    ))
    .unwrap();
    verify_reset_probe(&BootFlash::new(image).unwrap());
}

fn flat_cpu(privileged: bool) -> V9Cpu {
    V9Cpu::from(PowerOnReset {
        pc: 0,
        npc: 4,
        privileged,
        ..PowerOnReset::ultra_sparc_ii()
    })
}

fn verify_nvram_decision(flash: &BootFlash) {
    for (seed, expected_ccr, path) in [
        ([0; 8], 0, 0x21c0),
        (*b"soft-rst", 0x44, 0x2154),
        // ICC.Z is set, XCC.Z is clear: using the wrong flags reverses the path.
        ([0, 0, 0, 0, b'-', b'r', b's', b't'], 0x04, 0x21c0),
    ] {
        let mut image = vec![0xa5; RESET_NVRAM_SIZE];
        image[0x10..0x18].copy_from_slice(&seed);
        let mut nvram = ResetNvram::new(image).unwrap();
        // Power-cycling the CPU retains the supplied SRAM.
        for _ in 0..2 {
            let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
            for _ in 0..17 {
                flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
            }
            let value = u64::from_be_bytes(seed);
            let before_ccr = cpu.ccr();
            for (offset, word, next, npc, ccr) in [
                (0x2144, PROBE, 0x2148, 0x214c, before_ccr),
                (0x2148, 0x80a2_0009, 0x214c, 0x2150, expected_ccr),
                (0x214c, 0x1268_001d, 0x2150, path, expected_ccr),
                (0x2150, 0x0100_0000, path, path + 4, expected_ccr),
            ] {
                let fetch = flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
                assert_eq!((fetch.flash_offset, fetch.instruction), (offset, word));
                assert_eq!((cpu.pc(), cpu.npc()), (VECTOR + next, VECTOR + npc));
                assert_eq!(cpu.read_reg(8), u64::from_be_bytes(*b"soft-rst"));
                assert_eq!(
                    (cpu.read_reg(9), cpu.read_reg(10)),
                    (value, 0x01ff_f100_0010)
                );
                assert_eq!((cpu.ccr(), cpu.current_window()), (ccr, 7));
            }
        }
    }
}

#[test]
fn retained_nvram_selects_both_paths_after_the_delay_slot() {
    verify_nvram_decision(&synthetic_flash());
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_branches_on_the_retained_nvram_bytes() {
    let image = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    ))
    .unwrap();
    let flash = BootFlash::new(image).unwrap();
    verify_nvram_decision(&flash);
    // With reset %g7 zero, the second comparison also rejects a marker alone.
    // Both traces then take the annulled branch at 0x21c0 into MMU initialization.
    for (seed, count) in [([0; 8], 22), (*b"soft-rst", 31)] {
        let mut bytes = vec![0; RESET_NVRAM_SIZE];
        bytes[0x10..0x18].copy_from_slice(&seed);
        let mut nvram = ResetNvram::new(bytes).unwrap();
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        for _ in 0..count - 1 {
            flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
        }
        let jump = flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
        assert_eq!((jump.flash_offset, jump.instruction), (0x21c0, 0x306f_fd05));
        assert_eq!((cpu.pc(), cpu.npc()), (VECTOR + 0x15d4, VECTOR + 0x15d8));
        assert_eq!(flash.read_word(FLASH_BASE + 0x15d4).unwrap(), 0x8780_2058);
    }
}

#[test]
fn nvram_reset_aperture_only_serves_backed_sram() {
    for len in [0, RESET_NVRAM_SIZE - 1, RESET_NVRAM_SIZE + 1] {
        assert!(ResetNvram::new(vec![0; len]).is_err());
    }
    let mut image = vec![0xff; RESET_NVRAM_SIZE];
    image[..8].copy_from_slice(b"abcdefgh");
    image[0x1fe8..0x1ff0].copy_from_slice(b"ABCDEFGH");
    let mut nvram = ResetNvram::new(image).unwrap();
    let mut rom = vec![0; FLASH_SIZE];
    rom[0x20..0x24].copy_from_slice(&PROBE.to_be_bytes());
    let flash = BootFlash::new(rom).unwrap();
    for (offset, expected) in [(0, *b"abcdefgh"), (0x1fe8, *b"ABCDEFGH")] {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(10, RESET_NVRAM_BASE + offset);
        flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
        assert_eq!(cpu.read_reg(9).to_be_bytes(), expected);
    }
    for pa in [
        0x10,
        0xf100_0010,
        FLASH_BASE + 0x10,
        RESET_NVRAM_BASE - 8,
        RESET_NVRAM_BASE + 1,
        RESET_NVRAM_BASE + 0x1fec,
        RESET_NVRAM_BASE + 0x1ff0,
        RESET_NVRAM_BASE + 0x1ff8,
        RESET_NVRAM_BASE + 0x2000,
    ] {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(10, pa);
        cpu.write_reg(9, u64::MAX);
        let before = cpu.clone();
        assert!(
            flash.step_with_nvram(&mut cpu, &mut nvram).is_err(),
            "{pa:#x}"
        );
        assert_eq!(cpu, before);
    }
}

#[test]
fn bypass_reads_live_big_endian_data_after_address_addition() {
    // Both source registers participate, and rd aliases rs1.
    let ldxa = 0xd4da_82a9; // ldxa [%o2 + %o1] 0x15,%o2
    for base in [0x28, 0x0200_0000_0028, 0xffff_fe00_0000_0028] {
        let mut cpu = flat_cpu(true);
        let mut memory = V9Memory::new(64);
        memory.load_words(0, &[0x8080_0000, ldxa]); // addcc %g0,%g0,%g0
        memory.load_words(0x30, &[0x1122_3344, 0xaabb_ccdd]);
        cpu.write_reg(10, base);
        cpu.write_reg(9, 8);
        cpu.step(&mut memory).unwrap();
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.read_reg(10), 0x1122_3344_aabb_ccdd);
        assert_eq!(cpu.read_reg(9), 8);
        assert_eq!((cpu.pc(), cpu.npc(), cpu.ccr()), (8, 12, 0x44));
    }
}

#[test]
fn rejected_bypass_access_preserves_cpu_and_memory() {
    for (privileged, address, word, reason) in [
        (false, 0x10, PROBE, "privileged"),
        (true, 0x11, PROBE, "misaligned"),
        (true, 0x38, PROBE, "outside memory"), // four bytes left in the fixture
        (true, 0x0000_0800_0000_0010, PROBE, "effective address"),
        (true, 0x01ff_f100_0010, PROBE, "outside memory"),
        (
            true,
            0x01ff_f100_0010,
            PROBE & !(31 << 25),
            "outside memory",
        ), // %g0 still reads
        (true, 0x10, PROBE | (1 << 13), "alternate address space 0x0"), // initial %asi
        (true, 0x10, PROBE ^ (8 << 5), "alternate address space"),      // little-endian ASI 0x1d
        (true, 0x10, 0xd2f2_82a0, "alternate address space"),           // stxa
    ] {
        let mut cpu = flat_cpu(privileged);
        cpu.write_reg(10, address);
        cpu.write_reg(9, 0x5a);
        let mut memory = V9Memory::new(60);
        memory.load_words(0, &[word]);
        let before = (cpu.clone(), memory.clone());
        let error = cpu.step(&mut memory).unwrap_err();
        assert!(error.contains(reason), "{error}");
        assert_eq!((cpu, memory), before);
    }
}
