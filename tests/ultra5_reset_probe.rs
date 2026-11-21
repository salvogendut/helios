use helios::{
    ultra5::{BootFlash, PowerOnReset, FLASH_BASE, FLASH_SIZE},
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

#[test]
fn firmware_prefix_reaches_its_first_board_read() {
    let mut image = vec![0xff; FLASH_SIZE];
    for (off, word) in PREFIX
        .into_iter()
        .chain(SETUP.iter().enumerate().map(|(i, t)| (0x2118 + i * 4, t.0)))
        .chain([(0x2144, PROBE)])
    {
        image[off..off + 4].copy_from_slice(&word.to_be_bytes());
    }
    verify_reset_probe(&BootFlash::new(image).unwrap());
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
        (true, 0x10, PROBE | (1 << 13), "%asi register"),
        (true, 0x10, PROBE ^ (8 << 5), "alternate address space"), // little-endian ASI 0x1d
        (true, 0x10, 0xd2f2_82a0, "alternate address space"),      // stxa
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
