use helios::bus::{Bus, BusErrorKind};
use helios::cpu::{FaultKind, Step, PSR_EF, PSR_ET, PSR_S};
use helios::fpu::Fpu;
use helios::machine::Machine;
use helios::mmu::{Width, PCR_EN, PCR_NF};
use helios::system_bus::{IO_CONTROL_BASE, RESET_BASE, SYSTEM_INTERRUPT_BASE};

const NOP: u32 = 0x0100_0000;
const START: u32 = 20;
const ENABLED: u32 = PSR_S | PSR_EF | PSR_ET;

fn memory(op3: u32, rd: u32, rs1: u32, operand: u32, immediate: bool) -> u32 {
    0xc000_0000
        | (rd << 25)
        | (op3 << 19)
        | (rs1 << 14)
        | if immediate {
            0x2000 | (operand & 0x1fff)
        } else {
            operand & 31
        }
}
fn fpop(op3: u32, opf: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
    0x8000_0000 | (rd << 25) | (op3 << 19) | (rs1 << 14) | (opf << 5) | rs2
}
fn put(m: &mut Machine, address: u32, words: &[u32]) {
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    m.load_image(address, &bytes).unwrap();
}
fn setup(program: &[u32], psr: u32) -> Machine {
    let mut m = Machine::bare_metal(0x10000, 0).unwrap();
    put(&mut m, 0, &[0x8198_6000, 0x8188_a000, NOP, NOP, NOP]); // wr g1,tbr; wr g2,psr
    put(&mut m, START, program);
    m.cpu.write_reg(1, 0x1000);
    m.cpu.write_reg(2, psr);
    for _ in 0..5 {
        m.step().unwrap();
    }
    m
}
fn trap(m: &mut Machine, kind: FaultKind, tt: u8) {
    let pc = m.cpu.pc();
    let npc = m.cpu.npc();
    assert!(
        matches!(m.step().unwrap(), Step::TrapTaken { fault, trap_type, vector }
        if fault.kind == kind && trap_type == tt && vector == 0x1000 + u32::from(tt) * 16)
    );
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (pc, npc));
}

#[test]
fn single_transfers_preserve_all_word_patterns_and_keep_f0_writable() {
    for index in 0..32 {
        for bits in [0, 0x8000_0000, 0x7f80_0000, 0x7fc1_2345, 0xff81_2345, 1] {
            let mut m = setup(
                &[
                    memory(0x20, index, 1, 0x1ffc, true), // signed offset -4
                    memory(0x24, index, 1, 2, false),
                ],
                ENABLED | 0xa0_0000,
            );
            m.cpu.write_reg(1, 0x8004);
            m.cpu.write_reg(2, 4);
            m.bus.write32(0x8000, bits).unwrap();
            let icc = m.cpu.icc();
            m.step().unwrap();
            assert_eq!(m.cpu.fpu.read_reg(index as u8), bits);
            m.step().unwrap();
            assert_eq!(m.bus.read32(0x8008).unwrap(), bits);
            assert_eq!(m.cpu.read_reg(0), 0);
            assert_eq!(m.cpu.read_reg(1), 0x8004);
            assert_eq!(m.cpu.icc(), icc);
            assert_eq!(m.cpu.fpu.fsr(), 0x0008_0000);
        }
    }
    let mut m = setup(&[memory(0x20, 0, 1, 8, true)], ENABLED);
    m.cpu.write_reg(1, 0xffff_fffc);
    m.step().unwrap(); // effective address wraps to 4, containing WRPSR
    assert_eq!(m.cpu.fpu.read_reg(0), 0x8188_a000);
}

#[test]
fn double_transfers_use_even_odd_pairs_in_guest_address_order() {
    for rd in 0..32 {
        let mut m = setup(
            &[memory(0x23, rd, 1, 0, true), memory(0x27, rd, 1, 8, true)],
            ENABLED,
        );
        m.cpu.write_reg(1, 0x8000);
        m.bus
            .write_doubleword(0x8000, [0x89ab_cdef, 0x0123_4567])
            .unwrap();
        m.step().unwrap();
        // V8 reserves rd[0]; this implementation ignores it, as MAME does.
        assert_eq!(m.cpu.fpu.read_reg(rd as u8 & !1), 0x89ab_cdef);
        assert_eq!(m.cpu.fpu.read_reg(rd as u8 | 1), 0x0123_4567);
        m.step().unwrap();
        assert_eq!(
            m.bus.read_doubleword(0x8008).unwrap(),
            [0x89ab_cdef, 0x0123_4567]
        );
        assert_eq!(m.bus.read8(0x8008).unwrap(), 0x89);
        assert_eq!(m.bus.read8(0x800f).unwrap(), 0x67);
    }
}

#[test]
fn fsr_load_masks_fields_and_status_store_does_not_touch_f_registers() {
    let mut m = setup(
        &[
            memory(0x21, 31, 1, 0, true), // rd ignored for FSR transfers
            memory(0x25, 7, 1, 4, true),
            memory(0x21, 0, 1, 8, true),
        ],
        ENABLED,
    );
    m.cpu.write_reg(1, 0x8000);
    m.cpu.fpu.write_reg(31, 0xdead_beef);
    m.bus.write32(0x8000, u32::MAX).unwrap();
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.fsr(), 0xcf88_0fff); // RD/TEM/FCC/AEXC/CEXC + version4; NS=0
    m.step().unwrap();
    assert_eq!(m.bus.read32(0x8004).unwrap(), 0xcf88_0fff);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.fsr(), 0x0008_0000);
    assert_eq!(m.cpu.fpu.read_reg(31), 0xdead_beef);
}

#[test]
fn moves_change_only_requested_bits_clear_current_exceptions_and_preserve_accrued_state() {
    for (opf, xor, and) in [
        (1, 0, u32::MAX),
        (5, 0x8000_0000, u32::MAX),
        (9, 0, 0x7fff_ffff),
    ] {
        for bits in [0, 0x8000_0000, 0x7f80_0000, 0xff80_0001, 0xffc1_2345, 1] {
            for (src, dst) in [(0, 0), (0, 31), (31, 0)] {
                let mut m = setup(
                    &[memory(0x21, 0, 1, 0, true), fpop(0x34, opf, dst, 0, src)],
                    ENABLED,
                );
                m.cpu.write_reg(1, 0x8000);
                m.bus.write32(0x8000, u32::MAX).unwrap();
                m.cpu.fpu.write_reg(src as u8, bits);
                m.step().unwrap();
                m.step().unwrap();
                assert_eq!(m.cpu.fpu.read_reg(dst as u8), (bits ^ xor) & and);
                assert_eq!(m.cpu.fpu.fsr(), 0xcf88_0fe0);
            }
        }
    }
}

#[test]
fn every_floating_branch_condition_obeys_fcc_delay_and_annul_rules() {
    // Bit N is taken for FCC=N: E,L,G,U. Independent table from V8 B.22.
    let masks = [0, 14, 6, 10, 2, 12, 4, 8, 15, 1, 9, 5, 13, 3, 11, 7];
    for (condition, mask) in masks.into_iter().enumerate() {
        for fcc in 0..4 {
            for annul in [false, true] {
                for displacement in [16i32, -16] {
                    let branch = 0x0180_0000
                        | ((condition as u32) << 25)
                        | (u32::from(annul) << 29)
                        | (((displacement >> 2) as u32) & 0x3f_ffff);
                    let mut m = setup(
                        &[memory(0x21, 0, 1, 0, true), NOP, NOP, NOP, branch, NOP],
                        ENABLED,
                    );
                    m.cpu.write_reg(1, 0x8000);
                    m.bus.write32(0x8000, fcc << 10).unwrap();
                    for _ in 0..4 {
                        m.step().unwrap();
                    }
                    let pc = m.cpu.pc();
                    let fpu = m.cpu.fpu.clone();
                    m.step().unwrap();
                    let taken = mask & (1 << fcc) != 0;
                    let target = if taken {
                        pc.wrapping_add(displacement as u32)
                    } else {
                        pc + 8
                    };
                    let skipped = annul && (!taken || condition == 8);
                    assert_eq!(
                        (m.cpu.pc(), m.cpu.npc()),
                        if skipped {
                            (target, target + 4)
                        } else {
                            (pc + 4, target)
                        }
                    );
                    assert_eq!(m.cpu.fpu, fpu);
                    if !skipped {
                        m.step().unwrap();
                        assert_eq!(m.cpu.pc(), target);
                    }
                }
            }
        }
    }
}

#[test]
fn disabled_fpu_traps_before_memory_effects_or_branch_annul_even_for_missing_fpops() {
    let words = [0x20, 0x21, 0x23, 0x24, 0x25, 0x26, 0x27].map(|op| memory(op, 0, 1, 1, true));
    for word in words.into_iter().chain([
        0x3180_0004,
        fpop(0x34, 1, 0, 0, 0),
        fpop(0x34, 0x41, 0, 0, 0),
        fpop(0x35, 0x51, 0, 0, 0),
    ]) {
        let mut m = setup(&[word], PSR_S | PSR_ET);
        m.cpu.write_reg(1, 0x20000);
        let fpu = m.cpu.fpu.clone();
        trap(&mut m, FaultKind::FpDisabled, 4);
        assert_eq!(m.cpu.fpu, fpu);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
    }
}

#[test]
fn disabled_trap_in_delay_slot_preserves_the_pending_branch_target() {
    let mut m = setup(&[0x1080_0008, memory(0x20, 0, 1, 0, true)], PSR_S | PSR_ET);
    m.step().unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (START + 4, START + 32));
    trap(&mut m, FaultKind::FpDisabled, 4);
    assert_eq!(
        (m.cpu.read_reg(17), m.cpu.read_reg(18)),
        (START + 4, START + 32)
    );
}

#[test]
fn floating_memory_alignment_preserves_registers_fsr_and_cpu_mmu_fault_state() {
    for op in [0x20, 0x21, 0x23, 0x24, 0x25, 0x26, 0x27] {
        let address = if matches!(op, 0x23 | 0x26 | 0x27) {
            0x8004
        } else {
            0x8001
        };
        let mut m = setup(&[memory(op, 0, 1, 0, true)], ENABLED);
        m.cpu.write_reg(1, address);
        m.cpu.fpu.write_reg(0, 0x1234);
        let fpu = m.cpu.fpu.clone();
        assert!(matches!(
            m.step().unwrap(),
            Step::TrapTaken { trap_type: 7, .. }
        ));
        assert_eq!(m.cpu.fpu, fpu);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
        assert_eq!(m.bus.read32(0x8000).unwrap(), 0);
    }
}

#[test]
fn empty_queue_store_has_privilege_ef_alignment_sequence_priority_without_a_memory_access() {
    for (psr, address, expected, tt) in [
        (PSR_ET, 0x20001, FaultKind::PrivilegedInstruction, 3),
        (PSR_S | PSR_ET, 0x20001, FaultKind::FpDisabled, 4),
        (
            ENABLED,
            0x20001,
            FaultKind::DataAccess(helios::bus::BusError {
                address: 0x20001,
                kind: BusErrorKind::Misaligned,
            }),
            7,
        ),
        (ENABLED, 0x20000, FaultKind::FpException, 8),
    ] {
        let mut m = setup(&[memory(0x26, 31, 1, 0, true)], psr);
        m.cpu.write_reg(1, address);
        trap(&mut m, expected, tt);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
        assert_eq!(
            m.cpu.fpu.fsr(),
            if tt == 8 { 0x0009_0000 } else { 0x0008_0000 }
        );
    }
}

#[test]
fn sequence_error_ftt_survives_status_load_store_and_clears_on_a_completed_fpop() {
    let mut m = setup(&[memory(0x26, 0, 1, 0, true)], ENABLED);
    m.cpu.write_reg(1, 0x8000);
    put(
        &mut m,
        0x1080,
        &[
            memory(0x25, 0, 1, 0, true),
            memory(0x21, 0, 1, 4, true),
            fpop(0x34, 1, 0, 0, 0),
            memory(0x25, 0, 1, 8, true),
        ],
    );
    trap(&mut m, FaultKind::FpException, 8);
    m.step().unwrap();
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0x0009_0000);
    m.step().unwrap();
    assert_eq!(m.cpu.fpu.fsr(), 0x0009_0000); // LDFSR cannot clear FTT
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.bus.read32(0x8008).unwrap(), 0x0008_0000);
}

#[test]
fn unsupported_fpops_remain_retryable_host_stops_when_enabled() {
    for word in [
        fpop(0x34, 0x41, 4, 0, 2),
        fpop(0x35, 0x53, 0, 0, 4), // quad comparison trap support is pending
        fpop(0x34, 0x43, 0, 0, 4),
    ] {
        let mut m = setup(&[word], ENABLED);
        let before = m.cpu.clone();
        let error = m.step().unwrap_err();
        assert_eq!(error.kind, FaultKind::UnsupportedInstruction);
        assert_eq!(m.cpu, before);
        assert_eq!(m.step().unwrap_err(), error);
    }
}

#[test]
fn failed_pair_transfers_do_not_commit_half_a_register_or_half_a_store() {
    for op in [0x23, 0x27] {
        let mut m = setup(&[memory(op, 30, 1, 0, true)], ENABLED);
        m.cpu.write_reg(1, 0x20000);
        m.cpu.fpu.write_reg(30, 0x1111_2222);
        m.cpu.fpu.write_reg(31, 0x3333_4444);
        m.bus.map_ram(0x20000, 4).unwrap(); // second word unmapped
        m.bus.write32(0x20000, 0xdead_beef).unwrap();
        let before = m.cpu.fpu.clone();
        assert!(matches!(
            m.step().unwrap(),
            Step::TrapTaken {
                trap_type: 0x29,
                ..
            }
        ));
        assert_eq!(m.cpu.fpu, before);
        assert_eq!(m.bus.read32(0x20000).unwrap(), 0xdead_beef);
        assert_eq!(m.cpu.mmu.fault_address(), 0x20000);
    }
}

#[test]
fn floating_transfers_use_supervisor_or_user_srmmu_permissions_and_preserve_failed_destinations() {
    for supervisor in [false, true] {
        let mut m = setup(
            &[memory(0x23, 0, 1, 0, true), memory(0x27, 0, 1, 8, true)],
            PSR_EF | PSR_ET | if supervisor { PSR_S } else { 0 },
        );
        m.cpu.write_reg(1, 0x8000);
        // Identity level3 mapping for code page0; data page8 supervisor-only.
        for (addr, value) in [
            (0x1000, 0x201),
            (0x2000, 0x301),
            (0x3000, 0x401),
            (0x4000, 0xe),
            (0x4020, 0x81e),
        ] {
            m.bus.write32(addr, value).unwrap();
        }
        m.cpu
            .mmu
            .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
            .unwrap();
        m.cpu
            .mmu
            .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
            .unwrap();
        m.bus
            .write_doubleword(0x8000, [0x1122_3344, 0x5566_7788])
            .unwrap();
        if supervisor {
            m.step().unwrap();
            m.step().unwrap();
            assert_eq!(
                m.bus.read_doubleword(0x8008).unwrap(),
                [0x1122_3344, 0x5566_7788]
            );
            assert_ne!(m.bus.read32(0x4020).unwrap() & 0x60, 0); // R/M updates
        } else {
            m.cpu.fpu.write_reg(0, 123);
            assert!(matches!(
                m.step().unwrap(),
                Step::TrapTaken { trap_type: 9, .. }
            ));
            assert_eq!(m.cpu.fpu.read_reg(0), 123);
            assert_eq!(m.cpu.mmu.fault_address(), 0x8000);
            assert_eq!(m.cpu.mmu.fault_status() & 0x1c, 0xc); // privilege FT3
        }
    }
}

#[test]
fn no_fault_mode_zeroes_failed_floating_load_and_does_not_hide_unsupported_mmio() {
    let mut m = setup(&[memory(0x23, 0, 1, 0, true)], ENABLED);
    m.cpu.write_reg(1, 0x20000);
    m.cpu.fpu.write_reg(0, u32::MAX);
    m.cpu.fpu.write_reg(1, u32::MAX);
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_NF)
        .unwrap();
    m.step().unwrap();
    assert_eq!((m.cpu.fpu.read_reg(0), m.cpu.fpu.read_reg(1)), (0, 0));
    assert_eq!(m.cpu.mmu.fault_address(), 0x20000);

    let prom: Vec<_> = [0x8188_6000, NOP, NOP, NOP, memory(0x23, 0, 2, 0, true)]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    let mut m = Machine::sun4m(0x4000, &prom).unwrap();
    m.cpu.write_reg(1, ENABLED);
    m.cpu.write_reg(2, (IO_CONTROL_BASE + 0x1000) as u32);
    for _ in 0..4 {
        m.step().unwrap();
    }
    let fpu = m.cpu.fpu.clone();
    assert!(matches!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedMemory(_)
    ));
    assert_eq!(m.cpu.fpu, fpu);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert!(m.cpu.error_state().is_none());
}

#[test]
fn floating_word_store_uses_asynchronous_sbus_error_capture() {
    let prom: Vec<_> = [0x8188_6000, NOP, NOP, NOP, memory(0x24, 0, 2, 0, true)]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    let mut m = Machine::sun4m(0x4000, &prom).unwrap();
    m.cpu.write_reg(1, ENABLED);
    m.cpu.write_reg(2, 0x7cc0_0000);
    m.cpu.fpu.write_reg(0, 0x1234);
    for _ in 0..5 {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
    assert_eq!(m.bus.read32(IO_CONTROL_BASE + 0x1000).unwrap(), 0xa182_0000);
    assert_eq!(m.bus.read32(SYSTEM_INTERRUPT_BASE).unwrap(), 1 << 30);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}

#[test]
fn disabled_fpu_with_et_clear_latches_error_and_reset_clears_floating_state() {
    let mut m = Machine::sun4m(0x4000, &fpop(0x34, 1, 0, 0, 0).to_be_bytes()).unwrap();
    m.cpu.fpu.write_reg(0, 0x1234);
    let fault = m.step().unwrap_err();
    assert_eq!(fault.kind, FaultKind::FpDisabled);
    assert_eq!(m.cpu.fpu.read_reg(0), 0x1234);
    assert_eq!(m.cpu.pc(), 0);
    assert_eq!(m.step().unwrap_err(), fault);
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0x10);
    for software in [false, true] {
        if software {
            m.cpu.software_reset_sun4m();
        } else {
            m.reset();
        }
        assert_eq!(m.cpu.fpu, Fpu::default());
        assert_eq!(m.cpu.fpu.fsr(), 0x0008_0000);
        assert_eq!(m.cpu.psr() & PSR_EF, 0);
        m.cpu.fpu.write_reg(31, 0x1234);
    }
}
