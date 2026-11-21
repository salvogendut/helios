use helios::{
    ultra5::{BootFlash, PowerOnReset, ResetNvram, FLASH_SIZE, RESET_NVRAM_SIZE},
    v9::{V9Cpu, V9Memory},
};

const VECTOR: u64 = 0xffff_ffff_f000_0000;
const SYNC: u32 = 0x8143_e040;
const RESET_WORDS: [u32; 6] = [
    0x8780_2058, // wr %g0,0x58,%asi
    0xc0f0_2008, // stxa %g0,[8]%asi
    SYNC,
    0xc0f0_2010, // stxa %g0,[0x10]%asi
    SYNC,
    0x8e00_0000, // add %g0,%g0,%g7
];

fn flash_at(offset: usize, words: &[u32]) -> BootFlash {
    let mut image = vec![0; FLASH_SIZE];
    for (i, word) in words.iter().enumerate() {
        let pos = offset + 4 * i;
        image[pos..pos + 4].copy_from_slice(&word.to_be_bytes());
    }
    BootFlash::new(image).unwrap()
}

fn privileged_cpu(pc: u64) -> V9Cpu {
    PowerOnReset {
        pc,
        npc: pc + 4,
        ..PowerOnReset::ultra_sparc_ii()
    }
    .into()
}

// Observe guest-visible control state through instruction reads on a CPU copy.
// Literal ASI loads make the two context reads independent of the saved %asi.
fn read_controls(cpu: &V9Cpu) -> (u64, u64, u64) {
    let mut copy = cpu.clone();
    copy.write_reg(3, 8);
    copy.write_reg(4, 16);
    let words = [SYNC, 0x8f40_c000, 0xc2d8_cb00, 0xc4d9_0b00];
    let probe = flash_at((cpu.pc() - VECTOR) as usize, &words);
    for _ in words {
        probe.step(&mut copy).unwrap();
    }
    (copy.read_reg(7), copy.read_reg(1), copy.read_reg(2))
}

#[test]
fn guest_seeds_reads_and_clears_independent_contexts() {
    // These words were also single-stepped in QEMU 10.2.2 with a TI UltraSPARC-IIi.
    let words = [
        0x8610_2008,
        0x8810_2010,
        0x8a10_2357,
        0x8c10_26ac, // addresses and context IDs
        0xcaf0_cb00,
        SYNC,
        0xccf1_0b00,
        SYNC, // literal ASI stores to 8 and 16
        RESET_WORDS[0],
        0xd0d9_3ff8,
        0xd2d8_e008, // ldxa [%g4-8]%asi,%o0; ldxa [%g3+8]%asi,%o1
        RESET_WORDS[1],
        SYNC,
        0xd4d9_0b00, // read secondary with literal ASI after clearing primary
        RESET_WORDS[3],
        SYNC,
        0xc2d8_2008,
        0xc4d8_2010,
        0x9740_c000,
    ];
    let mut cpu = privileged_cpu(0);
    let mut mem = V9Memory::new(words.len() * 4);
    mem.load_words(0, &words);
    cpu.run(&mut mem, words.len()).unwrap();
    assert_eq!(
        (cpu.read_reg(8), cpu.read_reg(9), cpu.read_reg(10)),
        (0x357, 0x6ac, 0x6ac)
    );
    assert_eq!(
        (cpu.read_reg(1), cpu.read_reg(2), cpu.read_reg(11)),
        (0, 0, 0x58)
    );
    assert_eq!(cpu.pc(), words.len() as u64 * 4);
}

#[test]
fn user_can_select_asi_but_cannot_access_a_restricted_space() {
    let mut cpu = V9Cpu::new(0);
    cpu.write_reg(1, 0x181);
    cpu.write_reg(2, 0x0d9);
    let mut mem = V9Memory::new(32);
    mem.load_words(
        0,
        &[
            0x8780_7ffe,
            0x8940_c000, // wr %g1,-2,%asi; rd %asi,%g4
            0x8780_4002,
            0x8740_c000, // wr %g1,%g2,%asi; rd %asi,%g3
            RESET_WORDS[1],
        ],
    );
    cpu.run(&mut mem, 4).unwrap();
    assert_eq!((cpu.read_reg(4), cpu.read_reg(3)), (0x7f, 0x58));
    let before = cpu.clone();
    assert!(cpu.step(&mut mem).unwrap_err().contains("privileged"));
    assert_eq!(cpu, before);
    mem.load_words(cpu.pc(), &[0xc2d8_0b00]); // restricted literal ASI, even with %g0 address
    assert!(cpu.step(&mut mem).unwrap_err().contains("privileged"));
    assert_eq!(cpu, before);
}

#[test]
fn register_asi_selects_live_device_or_memory_and_preserves_a_branch_target() {
    let mut cpu = privileged_cpu(0);
    cpu.write_reg(2, 0x48);
    cpu.write_reg(3, 3);
    let mut mem = V9Memory::new(0x50);
    mem.load_words(
        0,
        &[
            0x8780_2045,
            0xc6f0_2000, // wr 0x45,%asi; stxa %g3,[0]%asi to LSU control
            0xc8d8_2000,
            0x8780_2015, // read back both cache-enable bits; switch to physical
            0x1068_0004,
            0xc4d8_bff8, // ba to 0x20; ldxa [%g2-8]%asi,%g2 in its slot
        ],
    );
    mem.load_words(0x40, &[0x1234_5678, 0x9abc_def0]);
    cpu.run(&mut mem, 5).unwrap();
    assert_eq!((cpu.pc(), cpu.npc(), cpu.read_reg(4)), (0x14, 0x20, 3));
    cpu.step(&mut mem).unwrap();
    assert_eq!(
        (cpu.pc(), cpu.npc(), cpu.read_reg(2)),
        (0x20, 0x24, 0x1234_5678_9abc_def0)
    );
}

#[test]
fn invalid_context_accesses_and_state_encodings_leave_all_state_intact() {
    let mut cpu = privileged_cpu(0);
    cpu.write_reg(1, 0x1fff);
    cpu.write_reg(2, 0x1234);
    let mut mem = V9Memory::new(32);
    mem.load_words(
        0,
        &[
            RESET_WORDS[0],
            0xc2f0_2008,
            SYNC,
            0xc4f0_2010,
            SYNC,
            0x8080_0000,
        ],
    );
    cpu.run(&mut mem, 6).unwrap();
    cpu.write_reg(3, 0x2000);
    for word in [
        0xc6f0_2008,
        0xc6f0_2010, // store with nonzero reserved context bits
        0xc0f0_2009,
        0xc0d8_2011, // misaligned store and load into %g0
        0xc0f0_2018,
        0xc2d8_2000, // other, unimplemented D-MMU registers
        0x8780_4022,
        0x8740_c001,      // reserved WRASI and RDASI operand bits
        SYNC | (1 << 25), // malformed barrier destination
    ] {
        mem.load_words(cpu.pc(), &[word]);
        let before = (cpu.clone(), mem.clone());
        assert!(cpu.step(&mut mem).is_err(), "{word:08x}");
        assert_eq!((&cpu, &mem), (&before.0, &before.1));
    }
    mem.load_words(cpu.pc(), &[0xc2d8_2008, 0xc4d8_2010]);
    cpu.run(&mut mem, 2).unwrap();
    assert_eq!(
        (cpu.read_reg(1), cpu.read_reg(2), cpu.ccr()),
        (0x1fff, 0x1234, 0x44)
    );
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_clears_each_context_and_reaches_the_pil_write() {
    let flash = BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap();
    // Eight synthetic instructions end at the real POR vector. They allow
    // a second run to distinguish two real stores from no-ops on zero state.
    let setup = flash_at(
        0,
        &[
            0x8610_2008,
            0x8810_2010,
            0xcaf0_cb00,
            SYNC,
            0xccf1_0b00,
            SYNC,
            0x0100_0000,
            0x8780_2015,
        ],
    );
    for (seed, prefix) in [([0; 8], 22), (*b"soft-rst", 31)] {
        let mut bytes = vec![0; RESET_NVRAM_SIZE];
        bytes[0x10..0x18].copy_from_slice(&seed);
        let mut nvram = ResetNvram::new(bytes).unwrap();
        for seeded in [false, true] {
            let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
            if seeded {
                cpu = privileged_cpu(VECTOR);
                cpu.write_reg(5, 0x357);
                cpu.write_reg(6, 0x6ac);
                cpu.write_reg(7, 0xabc);
                for _ in 0..8 {
                    setup.step(&mut cpu).unwrap();
                }
            }
            for _ in 0..prefix {
                flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
            }
            let (asi, primary, secondary) = if seeded {
                (0x15, 0x357, 0x6ac)
            } else {
                (0, 0, 0)
            };
            assert_eq!(read_controls(&cpu), (asi, primary, secondary));
            let mut regs: [u64; 32] = std::array::from_fn(|r| cpu.read_reg(r as u8));
            let ccr = cpu.ccr();
            for (i, word) in RESET_WORDS.into_iter().enumerate() {
                let off = 0x15d4 + i * 4;
                let fetch = flash.step_with_nvram(&mut cpu, &mut nvram).unwrap();
                assert_eq!((fetch.flash_offset, fetch.instruction), (off, word));
                let next = VECTOR + off as u64 + 4;
                assert_eq!(
                    (cpu.pc(), cpu.npc(), cpu.ccr(), cpu.current_window()),
                    (next, next + 4, ccr, 7)
                );
                assert_eq!(
                    read_controls(&cpu),
                    (
                        0x58,
                        if i < 1 { primary } else { 0 },
                        if i < 3 { secondary } else { 0 }
                    )
                );
                if i == 5 {
                    regs[7] = 0;
                }
                assert_eq!(std::array::from_fn(|r| cpu.read_reg(r as u8)), regs);
            }
            assert_eq!(cpu.pc(), VECTOR + 0x15ec);
            assert_eq!(
                flash
                    .read_word(helios::ultra5::FLASH_BASE + 0x15ec)
                    .unwrap(),
                0x9190_200f
            );
        }
    }
}
