#![cfg(feature = "jit-native")]
use helios::bus::Bus;
use helios::cpu::{Icc, Step};
use helios::machine::Machine;
use helios::mmu::Width;

fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn bare(words: &[u32]) -> Machine {
    let mut m = Machine::bare_metal(0x10000, 0).unwrap();
    m.load_image(0, &image(words)).unwrap();
    m
}
fn steps(m: &mut Machine, n: u64) {
    for _ in 0..n {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
}

#[test]
fn warmed_native_block_respects_remaining_budget() {
    let words = [
        0x8200_6001, // add %g1, 1, %g1
        0x8400_a002, // add %g2, 2, %g2
        0x10bf_fffe, // ba 0
        0x0100_0000,
    ];
    let mut reference = bare(&words);
    let mut candidate = bare(&words);
    steps(&mut reference, 5);
    let stats = candidate.run_jit_native(5).unwrap();
    assert_eq!(
        stats.executed, 5,
        "cached two-word block must not overshoot the last one-word budget: {stats:?}"
    );
    assert_eq!(candidate.cpu.read_reg(1), 2);
    assert_eq!(candidate.cpu.read_reg(2), 2);
    assert_eq!(candidate.virtual_ns(), 500);
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn guest_enabling_mmu_revokes_native_admission() {
    let build = || {
        let mut m = bare(&[
            0xc4a0_4080, // sta %g2, [%g1] ASI 4: PCR <- EN
            0x8600_e06f, // untranslated decoy: add %g3, 111, %g3
        ]);
        for (address, word) in [
            (0x1000, 0x201),
            (0x2000, 0x301),
            (0x3000, 0x401),
            (0x4000, 0x088e),
            (0x8004, 0x8600_e007), // mapped target: add 7
        ] {
            m.bus.write32(address, word).unwrap();
        }
        m.cpu
            .mmu
            .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
            .unwrap();
        m.cpu.write_reg(1, 0);
        m.cpu.write_reg(2, 1);
        m
    };
    let mut reference = build();
    let mut candidate = build();
    steps(&mut reference, 2);
    assert_eq!(reference.cpu.read_reg(3), 7);
    let stats = candidate.run_jit_native(2).unwrap();
    assert_eq!(stats.executed, 2);
    assert_eq!(
        candidate.cpu.read_reg(3),
        7,
        "native admission must be recomputed after MMU-control fallback; {stats:?}"
    );
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn disabled_mmu_still_applies_physical_address_mask() {
    let build = || {
        let mut m = bare(&[0x8200_6001]); // actual low physical instruction: add 1
        m.bus.map_ram(0x8000_0000, 0x1000).unwrap();
        m.bus.write32(0x8000_0000, 0x8200_6009).unwrap(); // decoy: add 9
        m.cpu.set_pc_npc(0x8000_0000, 0x8000_0004);
        m
    };
    let mut reference = build();
    let mut candidate = build();
    steps(&mut reference, 1);
    assert_eq!(reference.cpu.read_reg(1), 1);
    let stats = candidate.run_jit_native(1).unwrap();
    assert_eq!(stats.executed, 1);
    assert_eq!(
        candidate.cpu.read_reg(1),
        1,
        "MMU-disabled VA must still obey the CPU physical mask; {stats:?}"
    );
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

#[test]
fn native_block_does_not_cross_physical_mask_boundary() {
    let build = || {
        let mut m = bare(&[0x8400_a002]); // wrapped physical target: add 2
        m.bus.map_ram(0x7fff_f000, 0x2000).unwrap();
        m.bus.write32(0x7fff_fffc, 0x8200_6001).unwrap();
        m.bus.write32(0x8000_0000, 0x8400_a009).unwrap(); // decoy: add 9
        m.cpu.set_pc_npc(0x7fff_fffc, 0x8000_0000);
        m
    };
    let mut reference = build();
    let mut candidate = build();
    steps(&mut reference, 2);
    let stats = candidate.run_jit_native(2).unwrap();
    assert_eq!(stats.executed, 2);
    assert_eq!(candidate.cpu.read_reg(1), 1);
    assert_eq!(
        candidate.cpu.read_reg(2),
        2,
        "the entire block must respect the physical mask; {stats:?}"
    );
    assert!(reference
        .diagnostic_snapshot()
        .differences(&candidate.diagnostic_snapshot())
        .is_empty());
}

fn alu(op: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
    0x8000_0000 | (rd << 25) | (op << 19) | (rs1 << 14) | rs2
}

#[test]
fn native_emitter_matches_independent_alu_and_icc_matrix() {
    let pairs = [
        (0, 0),
        (1, 1),
        (0, 1),
        (u32::MAX, 1),
        (0x7fff_ffff, 1),
        (0x8000_0000, 1),
        (0x8000_0000, 0x8000_0000),
        (0x1234_5678, 0xfedc_ba98),
        (u32::MAX, u32::MAX),
    ];
    let initial = Icc {
        n: true,
        z: false,
        v: true,
        c: true,
    };
    let mut count = 0;
    for op in [0, 1, 2, 3, 4, 0x10, 0x11, 0x12, 0x13, 0x14] {
        for rd in [0, 1, 2, 7, 8, 15, 16, 23, 24, 31] {
            for (a, b) in pairs {
                let words = [alu(op, rd, 1, 2)];
                let mut reference = bare(&words);
                let mut candidate = bare(&words);
                for m in [&mut reference, &mut candidate] {
                    m.cpu.write_reg(1, a);
                    m.cpu.write_reg(2, b);
                    m.cpu.set_icc(initial);
                    m.cpu.set_window_invalid_mask(0xa5);
                }
                let (value, carry, overflow) = match op & 15 {
                    0 => {
                        let signed = i64::from(a as i32) + i64::from(b as i32);
                        (
                            a.wrapping_add(b),
                            u64::from(a) + u64::from(b) > u64::from(u32::MAX),
                            signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX),
                        )
                    }
                    4 => {
                        let signed = i64::from(a as i32) - i64::from(b as i32);
                        (
                            a.wrapping_sub(b),
                            a < b,
                            signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX),
                        )
                    }
                    1 => (a & b, false, false),
                    2 => (a | b, false, false),
                    3 => (a ^ b, false, false),
                    _ => unreachable!(),
                };
                let expected = if op & 0x10 == 0 {
                    initial
                } else {
                    Icc {
                        n: value & 0x8000_0000 != 0,
                        z: value == 0,
                        v: overflow,
                        c: carry,
                    }
                };
                steps(&mut reference, 1);
                let stats = candidate.run_jit_native(1).unwrap();
                assert_eq!(stats.executed, 1);
                assert_eq!(stats.native_blocks, 1);
                assert_eq!(
                    candidate.cpu.read_reg(rd as u8),
                    if rd == 0 { 0 } else { value },
                    "op={op:x} rd={rd} a={a:x} b={b:x}"
                );
                assert_eq!(
                    candidate.cpu.icc(),
                    expected,
                    "op={op:x} rd={rd} a={a:x} b={b:x}"
                );
                assert!(reference
                    .diagnostic_snapshot()
                    .differences(&candidate.diagnostic_snapshot())
                    .is_empty());
                count += 1;
            }
        }
    }
    assert_eq!(count, 900);
}
