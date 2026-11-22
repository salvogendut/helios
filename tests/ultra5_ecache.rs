use helios::ultra5::{
    BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard, FLASH_SIZE, RESET_NVRAM_SIZE,
};
use helios::v9::{V9Cpu, V9Memory};

const DATA: u64 = 0x80_0000_0000;
const TAG: u64 = 0x100_0000_0000;
const SIZE: u64 = 0x20_0000;

// STXA %g2,[%g1] asi and LDXA [%g1] asi,%g3.
const STORE: u32 = 0xc4f0_4000;
const LOAD: u32 = 0xc6d8_4000;
const WRITE_DATA: u32 = STORE | (0x76 << 5);
const READ_DATA: u32 = LOAD | (0x7e << 5);
const STAGE: u32 = STORE | (0x4e << 5);
const READ_STAGE: u32 = LOAD | (0x4e << 5);
const CONFIG: u32 = STORE | (0x4a << 5);
const READ_CONFIG: u32 = LOAD | (0x4a << 5);

fn cpu() -> V9Cpu {
    PowerOnReset {
        pc: 0,
        npc: 4,
        ..PowerOnReset::ultra_sparc_ii()
    }
    .into()
}

fn step(cpu: &mut V9Cpu, mem: &mut V9Memory, word: u32, address: u64, source: u64) -> u64 {
    cpu.write_reg(1, address);
    cpu.write_reg(2, source);
    mem.load_words(cpu.pc(), &[word]);
    cpu.step(mem).unwrap();
    cpu.read_reg(3)
}

#[test]
fn probe_observes_real_cells_at_each_offset_and_only_the_capacity_wraps() {
    let mut cpu = cpu();
    let mut mem = V9Memory::new(1024);
    mem.write_u64(0x380, 0x1122_3344_5566_7788);
    let cells = [0, 8, 0x40000, 0x80000, 0x100000, 0x1ffff8];
    for offset in cells {
        step(&mut cpu, &mut mem, WRITE_DATA, DATA + offset, !offset);
    }
    step(
        &mut cpu,
        &mut mem,
        WRITE_DATA,
        DATA + SIZE,
        0xcafe_f00d_feed_face,
    );
    for offset in cells {
        let expected = if offset == 0 {
            0xcafe_f00d_feed_face
        } else {
            !offset
        };
        assert_eq!(
            step(&mut cpu, &mut mem, READ_DATA, DATA + offset, 0),
            expected
        );
    }
    step(&mut cpu, &mut mem, WRITE_DATA, DATA, 0);
    assert_eq!(step(&mut cpu, &mut mem, READ_DATA, DATA + SIZE, 0), 0);
    assert_eq!(mem.read_u64(0x380), 0x1122_3344_5566_7788);
}

#[test]
fn tag_sram_transfers_through_staging_and_elim_limits_only_the_tag_index() {
    let mut cpu = cpu();
    let mut mem = V9Memory::new(2048);
    assert_eq!(step(&mut cpu, &mut mem, READ_CONFIG, 0, 0), 0);
    for (offset, bits) in [
        (0, 0x30001),
        (64, 0x24002),
        (SIZE / 2, 0x18003),
        (SIZE - 64, 0x0c004),
    ] {
        step(&mut cpu, &mut mem, STAGE, 0, bits | 0xffff_ffff_fffc_3000);
        assert_eq!(step(&mut cpu, &mut mem, READ_STAGE, 0, 0), bits);
        step(&mut cpu, &mut mem, WRITE_DATA, TAG + offset, u64::MAX);
    }
    step(&mut cpu, &mut mem, WRITE_DATA, DATA + SIZE / 2, 0x1234);
    for (elim, offset, expected) in [
        (3, SIZE / 2, 0x18003),
        (4, SIZE / 2, 0x30001),
        (5, 0x80040, 0x24002),
        (6, 0x40040, 0x24002),
        (7, 0x40000, 0x30001),
        (0, SIZE - 64, 0x0c004),
    ] {
        step(&mut cpu, &mut mem, CONFIG, 0, (elim << 33) | 0xff);
        assert_eq!(step(&mut cpu, &mut mem, READ_CONFIG, 0, 0), elim << 33);
        step(&mut cpu, &mut mem, READ_DATA, TAG + offset, 0);
        assert_eq!(step(&mut cpu, &mut mem, READ_STAGE, 0, 0), expected);
        assert_eq!(
            step(&mut cpu, &mut mem, READ_DATA, DATA + SIZE / 2, 0),
            0x1234
        );
    }
}

#[test]
fn rejected_diagnostic_accesses_preserve_the_cpu_and_its_sram() {
    let mut cpu = cpu();
    let mut mem = V9Memory::new(128);
    step(&mut cpu, &mut mem, WRITE_DATA, DATA, 0x1234);
    step(&mut cpu, &mut mem, STAGE, 0, 0x30000);
    step(&mut cpu, &mut mem, WRITE_DATA, TAG, 0);
    for (word, addr, value) in [
        (READ_DATA, DATA + 7, 0),
        (WRITE_DATA, DATA + SIZE + 8, 0),
        (WRITE_DATA, TAG + 8, 0),
        (READ_DATA, TAG + SIZE, 0),
        (WRITE_DATA, DATA | (1 << 41), 0),
        (READ_DATA, 0, 0),
        (READ_STAGE, 8, 0),
        (CONFIG, 8, 0),
        (CONFIG, 0, 1 << 33),
        (CONFIG, 0, 2 << 33),
        (CONFIG, 0, 1 << 36),
        (STORE | (0x7e << 5), DATA, 0),
        (LOAD | (0x76 << 5), DATA, 0),
        (0xc4a0_4000 | (0x76 << 5), DATA, 0),
    ] {
        cpu.write_reg(1, addr);
        cpu.write_reg(2, value);
        mem.load_words(cpu.pc(), &[word]);
        let before = (cpu.clone(), mem.clone());
        assert!(cpu.step(&mut mem).is_err(), "{word:#x} at {addr:#x}");
        assert_eq!((&cpu, &mem), (&before.0, &before.1));
    }
    let mut user = V9Cpu::new(0);
    for word in [
        READ_CONFIG,
        CONFIG,
        STAGE,
        READ_STAGE,
        READ_DATA,
        WRITE_DATA,
    ] {
        mem.load_words(0, &[word]);
        let before = user.clone();
        assert!(user.step(&mut mem).unwrap_err().contains("privileged"));
        assert_eq!(user, before);
    }
}

#[test]
#[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
fn original_obp_sizes_two_mib_clears_every_tag_and_transmits_the_build_banner() {
    let flash = BootFlash::new(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap(),
    )
    .unwrap();
    let reset = PowerOnReset::ultra_sparc_ii();
    let base = reset.pc - 0x20;
    for soft in [false, true] {
        for diag in [false, true] {
            let mut nvram = vec![0; RESET_NVRAM_SIZE];
            nvram[1] = u8::from(diag);
            if soft {
                nvram[16..24].copy_from_slice(b"soft-rst");
            }
            let mut board = Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(nvram).unwrap());
            let mut cpu = V9Cpu::from(reset);
            let mut probes = 0;
            let mut elim_writes = 0;
            let mut lines = 0;
            let mut retired = 0;
            let mut banner_end = None;
            let mut output = Vec::new();
            let stop = if soft { 0x780 } else { 0x554 };
            while cpu.pc() != base + stop && retired < 180_000 {
                let offset = cpu.pc() - base;
                match offset {
                    0x740 => {
                        assert_eq!(cpu.read_reg(8), SIZE);
                        probes += 1;
                    }
                    0x193c => {
                        assert_eq!(cpu.read_reg(3), 3 << 33);
                        elim_writes += 1;
                    }
                    0x74c => assert_eq!(cpu.read_reg(9), 0x30000),
                    0x760 => {
                        assert_eq!(cpu.read_reg(9), TAG);
                        assert_eq!(cpu.read_reg(8), SIZE - 64 * (lines + 1));
                        lines += 1;
                    }
                    _ => {}
                }
                board.step(&mut cpu).unwrap();
                retired += 1;
                let [a, b] = board.take_serial_output();
                assert!(b.is_empty());
                if !a.is_empty() {
                    assert_eq!(offset, 0x380);
                    output.extend(a);
                    if output.ends_with(b"20:36") {
                        banner_end = Some(retired);
                    }
                }
            }
            assert_eq!((cpu.pc(), cpu.npc()), (base + stop, base + stop + 4));
            assert_eq!((probes, elim_writes, lines), (2, 1, 32_768));
            let expected = if !diag {
                String::new()
            } else {
                let prefix = if soft { "Software" } else { "Hardware" };
                let suffix = if soft {
                    "Clearing E$ Tags  Done\r\nClearing I/D TLBs "
                } else {
                    "Probing keyboard "
                };
                format!("{prefix} Power ON\r\n\r\n@(#) Sun Ultra 5/10 UPA/PCI 3.31 Version 0 created 2001/07/25 20:36\r\n{suffix}")
            };
            assert_eq!(output, expected.as_bytes());
            assert_eq!(banner_end.is_some(), diag);
            // After the original run, use diagnostic loads on its saved CPU
            // state to read back every invalid tag and its two parity bits.
            check_firmware_tags(cpu, stop as usize);
            println!("soft={soft} diag={diag} retired={retired} last_banner_character={banner_end:?} tag_writes={lines}");
        }
    }
}

fn check_firmware_tags(mut cpu: V9Cpu, stop_offset: usize) {
    let mut image = vec![0; FLASH_SIZE];
    let instructions = &mut image[stop_offset..stop_offset + (SIZE as usize / 64) * 12];
    for probe in instructions.chunks_exact_mut(12) {
        probe[..4].copy_from_slice(&0xc0f0_09c0_u32.to_be_bytes()); // clear staging only
        probe[4..8].copy_from_slice(&READ_DATA.to_be_bytes());
        probe[8..].copy_from_slice(&0xc6d8_09c0_u32.to_be_bytes()); // read staging to %g3
    }
    let diagnostic = BootFlash::new(image).unwrap();
    for offset in (0..SIZE).step_by(64) {
        cpu.write_reg(1, TAG + offset);
        diagnostic.step(&mut cpu).unwrap();
        diagnostic.step(&mut cpu).unwrap();
        diagnostic.step(&mut cpu).unwrap();
        assert_eq!(cpu.read_reg(3), 0x30000, "tag at offset {offset:#x}");
    }
}
