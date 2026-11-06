use helios::bus::{Bus, BusErrorKind, MemoryBus};
use helios::cpu::{Cpu, FaultKind, Icc};
use helios::machine::{demo_image, Machine};

const NOP: u32 = 0x0100_0000;

fn format3(op: u32, op3: u32, rd: u8, rs1: u8, operand: u32, immediate: bool) -> u32 {
    (op << 30)
        | (u32::from(rd) << 25)
        | (op3 << 19)
        | (u32::from(rs1) << 14)
        | (u32::from(immediate) << 13)
        | (operand & if immediate { 0x1fff } else { 31 })
}

fn alu(op3: u32, rd: u8, rs1: u8, immediate: i32) -> u32 {
    format3(2, op3, rd, rs1, immediate as u32, true)
}

fn branch(condition: u32, annul: bool, words: i32) -> u32 {
    (u32::from(annul) << 29) | (condition << 25) | (2 << 22) | (words as u32 & 0x3f_ffff)
}

fn machine(words: &[u32]) -> Machine {
    let mut machine = Machine::bare_metal(4096, 0).unwrap();
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    machine.load_image(0, &bytes).unwrap();
    machine
}

fn assert_architecture_unchanged(cpu: &Cpu, before: &Cpu) {
    assert_eq!(
        (
            cpu.pc(),
            cpu.npc(),
            cpu.psr(),
            cpu.tbr(),
            cpu.y(),
            cpu.window_invalid_mask()
        ),
        (
            before.pc(),
            before.npc(),
            before.psr(),
            before.tbr(),
            before.y(),
            before.window_invalid_mask()
        )
    );
    for register in 0..32 {
        assert_eq!(cpu.read_reg(register), before.read_reg(register));
    }
}

#[test]
fn complete_loop_uses_flags_negative_displacement_delay_slot_and_store() {
    let mut m = Machine::bare_metal(4096, 0).unwrap();
    m.load_image(0, &demo_image()).unwrap();
    for _ in 0..43 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(1), 0);
    assert_eq!(m.cpu.read_reg(2), 55);
    assert_eq!(m.bus.read32(0x100).unwrap(), 55);
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (28, 32));
    m.step().unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (28, 32)); // ba,a skips an unsupported code word
}

#[test]
fn g0_is_hardwired_but_cc_operations_still_update_flags() {
    let mut m = machine(&[alu(0x10, 0, 1, 1)]);
    m.cpu.write_reg(0, 123);
    m.cpu.write_reg(1, u32::MAX);
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(0), 0);
    assert_eq!(
        m.cpu.icc(),
        Icc {
            n: false,
            z: true,
            v: false,
            c: true
        }
    );
}

#[test]
fn sethi_and_negative_immediates() {
    let mut m = machine(&[0x0304_8d15, alu(2, 1, 1, 0x278), alu(0, 2, 1, -1)]);
    for _ in 0..3 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(1), 0x1234_5678);
    assert_eq!(m.cpu.read_reg(2), 0x1234_5677);
}

#[test]
fn arithmetic_carry_borrow_and_signed_overflow() {
    for (op, a, b, result, flags) in [
        (
            0x10,
            0x7fff_ffff,
            1,
            0x8000_0000,
            Icc {
                n: true,
                z: false,
                v: true,
                c: false,
            },
        ),
        (
            0x10,
            0xffff_ffff,
            1,
            0,
            Icc {
                n: false,
                z: true,
                v: false,
                c: true,
            },
        ),
        (
            0x10,
            0x8000_0000,
            0x8000_0000,
            0,
            Icc {
                n: false,
                z: true,
                v: true,
                c: true,
            },
        ),
        (
            0x14,
            0,
            1,
            0xffff_ffff,
            Icc {
                n: true,
                z: false,
                v: false,
                c: true,
            },
        ),
        (
            0x14,
            0x8000_0000,
            1,
            0x7fff_ffff,
            Icc {
                n: false,
                z: false,
                v: true,
                c: false,
            },
        ),
        (
            0x14,
            0x7fff_ffff,
            0xffff_ffff,
            0x8000_0000,
            Icc {
                n: true,
                z: false,
                v: true,
                c: true,
            },
        ),
    ] {
        let mut m = machine(&[format3(2, op, 3, 1, 2, false)]);
        m.cpu.write_reg(1, a);
        m.cpu.write_reg(2, b);
        m.step().unwrap();
        assert_eq!(m.cpu.read_reg(3), result);
        assert_eq!(m.cpu.icc(), flags);
    }
}

#[test]
fn extended_arithmetic_consumes_carry_and_non_cc_preserves_it() {
    let mut m = machine(&[alu(0x14, 0, 0, 1), alu(0x08, 1, 0, -1), alu(0x1c, 2, 0, -1)]);
    m.step().unwrap(); // 0-1 sets borrow
    let flags = m.cpu.icc();
    m.step().unwrap(); // 0 + -1 + carry = 0, no CC update
    assert_eq!(m.cpu.read_reg(1), 0);
    assert_eq!(m.cpu.icc(), flags);
    m.step().unwrap(); // 0 - UINT_MAX - borrow = 0, borrow set
    assert_eq!(m.cpu.read_reg(2), 0);
    assert_eq!(
        m.cpu.icc(),
        Icc {
            n: false,
            z: true,
            v: false,
            c: true
        }
    );
}

#[test]
fn logical_operations_and_shifts() {
    for (op, expected) in [
        (1, 0x0000_0000),
        (2, 0xffff_ffff),
        (3, 0xffff_ffff),
        (5, 0xaaaa_aaaa),
        (6, 0xaaaa_aaaa),
        (7, 0x0000_0000),
    ] {
        let mut m = machine(&[alu(0x14, 0, 0, 1), format3(2, op | 0x10, 3, 1, 2, false)]);
        m.cpu.write_reg(1, 0xaaaa_aaaa);
        m.cpu.write_reg(2, 0x5555_5555);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.read_reg(3), expected);
        assert!(!m.cpu.icc().v && !m.cpu.icc().c);
    }
    let mut m = machine(&[
        alu(0x25, 2, 1, 1),
        alu(0x26, 3, 1, 1),
        alu(0x27, 4, 1, 1),
        format3(2, 0x25, 5, 1, 6, false),
    ]);
    m.cpu.write_reg(1, 0x8000_0001);
    m.cpu.write_reg(6, 33);
    for _ in 0..4 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(2), 2);
    assert_eq!(m.cpu.read_reg(3), 0x4000_0000);
    assert_eq!(m.cpu.read_reg(4), 0xc000_0000);
    assert_eq!(m.cpu.read_reg(5), 2);
}

#[test]
fn all_integer_branch_conditions() {
    let cases: &[(u32, u32, &[u32])] = &[
        (0, 1, &[2, 3, 4, 5, 6, 8, 9, 15]),      // subcc: negative, borrow
        (1, 1, &[1, 2, 4, 8, 11, 13, 14, 15]),   // zero
        (2, 1, &[8, 9, 10, 11, 12, 13, 14, 15]), // positive
        (0x8000_0000, 1, &[2, 3, 7, 8, 9, 12, 13, 14]), // overflow
    ];
    for &(a, b, taken_conditions) in cases {
        for condition in 0..16 {
            let mut m = machine(&[
                format3(2, 0x14, 0, 1, 2, false),
                branch(condition, false, 4),
            ]);
            m.cpu.write_reg(1, a);
            m.cpu.write_reg(2, b);
            m.step().unwrap();
            m.step().unwrap();
            assert_eq!(m.cpu.pc(), 8);
            assert_eq!(
                m.cpu.npc(),
                if taken_conditions.contains(&condition) {
                    20
                } else {
                    12
                },
                "condition {condition}, a={a:x}"
            );
        }
    }
}

#[test]
fn branch_annul_and_delay_slot_rules() {
    for (condition, annul, expected_pc, delay_executed) in [
        (8, false, 4, true),
        (8, true, 12, false),
        (0, false, 4, true),
        (0, true, 8, false),
        (9, true, 4, true),  // bne,a taken with initial Z=0 retains delay slot
        (1, true, 8, false), // be,a not taken annuls it
    ] {
        let mut m = machine(&[branch(condition, annul, 3), alu(0, 1, 1, 1), NOP, NOP]);
        m.step().unwrap();
        assert_eq!(m.cpu.pc(), expected_pc);
        m.step().unwrap();
        assert_eq!(m.cpu.read_reg(1), u32::from(delay_executed));
    }
}

#[test]
fn call_and_jmpl_use_instruction_address_for_link() {
    let mut m = machine(&[
        NOP,
        0x4000_0004,
        alu(0, 1, 1, 1),
        NOP,
        NOP,
        alu(0x38, 0, 15, 8),
        alu(0, 2, 2, 1),
    ]);
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(15), 4);
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (8, 20));
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (24, 12));
    m.step().unwrap();
    assert_eq!(m.cpu.pc(), 12);
    assert_eq!((m.cpu.read_reg(1), m.cpu.read_reg(2)), (1, 1));
}

#[test]
fn save_and_restore_alias_outs_and_ins_and_write_new_window() {
    let mut m = machine(&[
        alu(0x3c, 14, 14, -96),
        alu(0, 24, 24, 1),
        alu(0x3d, 9, 24, 2),
    ]);
    m.cpu.write_reg(8, 40);
    m.cpu.write_reg(14, 0x1000);
    m.cpu.write_reg(16, 0xdead_beef);
    m.step().unwrap();
    assert_eq!(m.cpu.current_window(), 7);
    assert_eq!(m.cpu.read_reg(14), 0xfa0);
    assert_eq!(m.cpu.read_reg(30), 0x1000);
    assert_eq!(m.cpu.read_reg(24), 40);
    assert_eq!(m.cpu.read_reg(16), 0);
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.cpu.current_window(), 0);
    assert_eq!(m.cpu.read_reg(8), 41);
    assert_eq!(m.cpu.read_reg(9), 43);
    assert_eq!(m.cpu.read_reg(16), 0xdead_beef);
}

#[test]
fn windows_wrap_and_wim_faults_do_not_change_cpu_state() {
    let mut m = machine(&[alu(0x3c, 0, 0, 0); 8]);
    for i in 0..8 {
        m.cpu.write_reg(16, i);
        m.step().unwrap();
    }
    assert_eq!(m.cpu.current_window(), 0);
    assert_eq!(m.cpu.read_reg(16), 0);
    for (op, mask, expected) in [
        (0x3c, 0x80, FaultKind::WindowOverflow),
        (0x3d, 2, FaultKind::WindowUnderflow),
    ] {
        let mut m = machine(&[alu(op, 1, 0, 99)]);
        m.cpu.set_window_invalid_mask(mask);
        let before = m.cpu.clone();
        assert_eq!(m.step().unwrap_err().kind, expected);
        assert_architecture_unchanged(&m.cpu, &before);
        assert!(m.cpu.error_state().is_some());
    }
}

#[test]
fn loads_sign_extend_and_stores_preserve_adjacent_bytes() {
    let mut m = machine(&[
        format3(3, 5, 1, 0, 0x100, true),
        format3(3, 6, 2, 0, 0x102, true),
        format3(3, 9, 3, 0, 0x100, true),
        format3(3, 1, 4, 0, 0x100, true),
        format3(3, 10, 5, 0, 0x102, true),
        format3(3, 2, 6, 0, 0x102, true),
        format3(3, 0, 7, 0, 0x100, true),
    ]);
    m.cpu.write_reg(1, 0x80);
    m.cpu.write_reg(2, 0xabcd);
    for _ in 0..7 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(3), 0xffff_ff80);
    assert_eq!(m.cpu.read_reg(4), 0x80);
    assert_eq!(m.cpu.read_reg(5), 0xffff_abcd);
    assert_eq!(m.cpu.read_reg(6), 0xabcd);
    assert_eq!(m.cpu.read_reg(7), 0x8000_abcd);
}

#[test]
fn data_fault_in_delay_slot_preserves_both_pcs_and_registers() {
    let mut m = machine(&[branch(8, false, 4), format3(3, 0, 0, 1, 0, true)]);
    m.cpu.write_reg(1, 0x1000);
    m.step().unwrap();
    let before = m.cpu.clone();
    let error = m.step().unwrap_err();
    assert_eq!((error.pc, error.npc), (4, 16));
    assert!(matches!(error.kind, FaultKind::DataAccess(e) if e.kind == BusErrorKind::Unmapped));
    assert_architecture_unchanged(&m.cpu, &before); // even LD to %g0 must perform the access
    assert_eq!(m.cpu.error_state(), Some(error));
}

#[test]
fn unsupported_instruction_bad_fetch_jump_and_store_stop_precisely() {
    let mut m = machine(&[0x8070_0000]); // UDIV is not implemented yet
    let before = m.cpu.clone();
    assert_eq!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedInstruction
    );
    assert_eq!(m.cpu, before);
    for entry in [1, 4096] {
        m.cpu = Cpu::new(entry);
        let error = m.step().unwrap_err();
        assert!(matches!(error.kind, FaultKind::InstructionAccess(_)));
        assert_eq!(error.instruction, None);
        assert_eq!(m.cpu.pc(), entry);
    }
    for word in [alu(0x38, 1, 0, 3), format3(3, 4, 1, 0, 0x101, true)] {
        let mut m = machine(&[word]);
        m.cpu.write_reg(1, 0xdead_beef);
        let before = m.cpu.clone();
        assert!(m.step().is_err());
        assert_architecture_unchanged(&m.cpu, &before);
        assert!(m.cpu.error_state().is_some());
        assert_eq!(m.bus.read32(0x100).unwrap(), 0);
    }
}

#[test]
fn sequential_pc_and_effective_addresses_wrap_at_32_bits() {
    let mut bus = MemoryBus::default();
    // Virtual PC wraps at 32 bits; microSPARC-II drives 31 physical bits.
    bus.map_rom(0x7fff_fffc, &NOP.to_be_bytes()).unwrap();
    let mut cpu = Cpu::new(0xffff_fffc);
    cpu.step(&mut bus).unwrap();
    assert_eq!((cpu.pc(), cpu.npc()), (0, 4));
    let word = format3(3, 0, 2, 1, 4, true);
    let mut m = machine(&[word]);
    m.cpu.write_reg(1, 0xffff_fffc);
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(2), word);
}
