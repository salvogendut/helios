use helios::bus::Bus;
use helios::cpu::{FaultKind, Step, PSR_EF, PSR_ET, PSR_S};
use helios::devices::nvram::Nvram;
use helios::fpu::{DeferredEntry, Fpu, FpuState};
use helios::machine::Machine;
use helios::mmu::{Width, PCR_NF};
use helios::system_bus::{IO_CONTROL_BASE, RESET_BASE};

const NOP: u32 = 0x0100_0000;
const START: u32 = 36;
const ENABLED: u32 = PSR_S | PSR_EF | PSR_ET;
const NV_MASK: u32 = 1 << 27;

fn mem(op3: u32, rd: u32, rs1: u32, offset: u32) -> u32 {
    0xc000_2000 | (op3 << 19) | (rd << 25) | (rs1 << 14) | (offset & 0x1fff)
}
fn cmp(opf: u32, rs1: u32, rs2: u32) -> u32 {
    0x81a8_0000 | (rs1 << 14) | (opf << 5) | rs2
}
fn put(m: &mut Machine, address: u32, words: &[u32]) {
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    m.load_image(address, &bytes).unwrap();
}
fn setup(program: &[u32], fsr: u32) -> Machine {
    let mut m = Machine::bare_metal(0x10000, 0).unwrap();
    put(
        &mut m,
        0,
        &[
            0x8198_6000,
            0x8188_a000,
            NOP,
            NOP,
            NOP,
            mem(0x21, 0, 3, 0),
            NOP,
            NOP,
            NOP,
        ],
    );
    put(&mut m, START, program);
    m.bus.write32(0x8000, fsr).unwrap();
    m.cpu.write_reg(1, 0x1000);
    m.cpu.write_reg(2, ENABLED);
    m.cpu.write_reg(3, 0x8000);
    for _ in 0..9 {
        m.step().unwrap();
    }
    m.cpu.write_reg(1, 0x8000);
    m
}
fn operands(m: &mut Machine, double: bool, a: u64, b: u64) {
    if double {
        for (index, bits) in [(0, a), (2, b)] {
            m.cpu.fpu.write_reg(index, (bits >> 32) as u32);
            m.cpu.fpu.write_reg(index + 1, bits as u32);
        }
    } else {
        m.cpu.fpu.write_reg(0, a as u32);
        m.cpu.fpu.write_reg(2, b as u32);
    }
}
fn pending(next: &[u32]) -> Machine {
    let mut program = vec![cmp(0x51, 0, 2)];
    program.extend_from_slice(next);
    let mut m = setup(&program, NV_MASK | 0x8a3); // FCC=G, AEXC=5, CEXC=3
    operands(&mut m, false, 0x7f80_0001, 0);
    assert!(matches!(
        m.step().unwrap(),
        Step::Executed { pc: START, .. }
    ));
    assert_eq!(m.cpu.fpu.state(), FpuState::ExceptionPending);
    assert_eq!(m.cpu.fpu.fsr(), 0x0808_68b0); // NV enable, version4, FTT1,QNE,FCC=G,AEXC5,NV
    assert_eq!(
        m.cpu.fpu.deferred_entry(),
        Some(DeferredEntry {
            pc: START,
            instruction: cmp(0x51, 0, 2)
        })
    );
    m
}
fn fp_trap(m: &mut Machine) {
    let pc = m.cpu.pc();
    let npc = m.cpu.npc();
    assert!(
        matches!(m.step().unwrap(), Step::TrapTaken {trap_type:8, fault, vector:0x1080}
        if fault.kind == FaultKind::FpException)
    );
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (pc, npc));
    assert_eq!(m.cpu.fpu.state(), FpuState::Exception);
}

#[test]
fn ordered_single_comparisons_cover_zeros_subnormals_signs_and_infinities() {
    for (a, b, fcc) in [
        (0, 0x8000_0000, 0),
        (0x8000_0000, 0x8000_0000, 0),
        (0x7f80_0000, 0x7f80_0000, 0),
        (0xff80_0000, 0xff80_0000, 0),
        (1, 0, 2),
        (0x8000_0001, 0, 1),
        (1, 0x0080_0000, 1),
        (0x007f_ffff, 0x0080_0000, 1),
        (0x8000_0001, 0x8080_0000, 2),
        (0xbf80_0000, 0xc000_0000, 2),
        (0x3f80_0000, 0x4000_0000, 1),
        (0xff80_0000, 0xff7f_ffff, 1),
        (0x7f80_0000, 0x7f7f_ffff, 2),
        (0xbf80_0000, 0x3f80_0000, 1),
        (0x4b80_0000, 0x4b80_0001, 1),
    ] {
        for opf in [0x51, 0x55] {
            for reverse in [false, true] {
                let (a, b, expected) = if reverse {
                    (b, a, if fcc == 0 { 0 } else { 3 - fcc })
                } else {
                    (a, b, fcc)
                };
                let mut m = setup(&[cmp(opf, 0, 2)], 0xcf80_0abf);
                operands(&mut m, false, a, b);
                m.step().unwrap();
                assert_eq!(m.cpu.fpu.fsr(), 0xcf88_02a0 | (expected << 10));
                assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
                assert_eq!(m.cpu.fpu.deferred_entry(), None);
                assert_eq!(m.cpu.fpu.read_reg(0), a as u32);
                assert_eq!(m.cpu.fpu.read_reg(2), b as u32);
            }
        }
    }
}

#[test]
fn ordered_double_comparisons_use_both_words_and_reserved_low_register_bits() {
    for (a, b, fcc) in [
        (0, 0x8000_0000_0000_0000, 0),
        (1, 0, 2),
        (0x8000_0000_0000_0001, 0, 1),
        (0x000f_ffff_ffff_ffff, 0x0010_0000_0000_0000, 1),
        (0x8000_0000_0000_0001, 0x8010_0000_0000_0000, 2),
        (0x3ff0_0000_0000_0000, 0x3ff0_0000_0000_0001, 1),
        (0xbff0_0000_0000_0000, 0xbff0_0000_0000_0001, 2),
        (0x7ff0_0000_0000_0000, 0x7fef_ffff_ffff_ffff, 2),
        (0xfff0_0000_0000_0000, 0xffef_ffff_ffff_ffff, 1),
        (0xfff0_0000_0000_0000, 0xfff0_0000_0000_0000, 0),
    ] {
        for opf in [0x52, 0x56] {
            for odd in [0, 1] {
                let mut m = setup(&[cmp(opf, odd, 2 + odd)], 0xcf80_0abf);
                operands(&mut m, true, a, b);
                let before = [
                    m.cpu.fpu.read_reg(0),
                    m.cpu.fpu.read_reg(1),
                    m.cpu.fpu.read_reg(2),
                    m.cpu.fpu.read_reg(3),
                ];
                m.step().unwrap();
                assert_eq!(m.cpu.fpu.fsr(), 0xcf88_02a0 | (fcc << 10));
                assert_eq!(m.cpu.fpu.deferred_entry(), None);
                for (i, v) in before.into_iter().enumerate() {
                    assert_eq!(m.cpu.fpu.read_reg(i as u8), v);
                }
            }
        }
    }
}

#[test]
fn nan_comparisons_distinguish_quiet_signaling_and_exception_enabled_variants() {
    for double in [false, true] {
        let (qnan, snan, one, sign) = if double {
            (
                0x7ff8_0000_1234_5678,
                0x7ff0_0000_0000_0001,
                0x3ff0_0000_0000_0000,
                1u64 << 63,
            )
        } else {
            (0x7fc1_2345, 0x7f80_0001, 0x3f80_0000, 1 << 31)
        };
        for (a, b, signaling) in [
            (qnan, one, false),
            (one, qnan, false),
            (qnan, qnan, false),
            (snan, one, true),
            (one, snan, true),
            (qnan, snan, true),
            (snan, qnan, true),
            (snan, snan, true),
        ] {
            for negative in [0, sign] {
                for exceptional in [false, true] {
                    for enables in [0, NV_MASK, 0x0780_0000] {
                        let opf =
                            (if double { 0x52 } else { 0x51 }) | if exceptional { 4 } else { 0 };
                        let word = cmp(opf, 0, 2);
                        let mut m = setup(&[word], 0x8000_08bf | enables);
                        operands(&mut m, double, a | negative, b);
                        m.step().unwrap();
                        let invalid = signaling || exceptional;
                        let trap_enabled = invalid && enables & NV_MASK != 0;
                        let expected = if trap_enabled {
                            0x68b0
                        } else if invalid {
                            0xeb0
                        } else {
                            0xca0
                        };
                        assert_eq!(
                            m.cpu.fpu.fsr(),
                            0x8008_0000 | enables | expected,
                            "double={double} exceptional={exceptional} a={a:x} b={b:x}"
                        );
                        assert_eq!(
                            m.cpu.fpu.state(),
                            if trap_enabled {
                                FpuState::ExceptionPending
                            } else {
                                FpuState::Execute
                            }
                        );
                        assert_eq!(
                            m.cpu.fpu.deferred_entry(),
                            if trap_enabled {
                                Some(DeferredEntry {
                                    pc: START,
                                    instruction: word,
                                })
                            } else {
                                None
                            }
                        );
                        assert_eq!((m.cpu.pc(), m.cpu.npc()), (START + 4, START + 8));
                    }
                }
            }
        }
    }
}

#[test]
fn comparison_fcc_drives_a_following_branch_with_portable_spacing() {
    let mut m = setup(&[cmp(0x51, 0, 2), NOP, 0x0980_0004, NOP], 0); // fbl +16
    operands(&mut m, false, 0xbf80_0000, 0x3f80_0000);
    m.step().unwrap();
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (START + 12, START + 24));
    m.step().unwrap();
    assert_eq!(m.cpu.pc(), START + 24);
}

#[test]
fn deferred_exception_allows_integer_work_then_saves_detecting_pc_and_original_queue_entry() {
    let mut m = pending(&[0x8801_2001, mem(0x25, 0, 1, 8)]); // add g4,1,g4; stfsr
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(4), 1);
    assert_eq!(m.cpu.fpu.state(), FpuState::ExceptionPending);
    fp_trap(&mut m);
    assert_eq!(
        (m.cpu.read_reg(17), m.cpu.read_reg(18)),
        (START + 8, START + 12)
    );
    assert_eq!(m.bus.read32(0x8008).unwrap(), 0); // trapping stfsr did not execute
    put(
        &mut m,
        0x1080,
        &[
            mem(0x25, 0, 1, 8),
            mem(0x26, 0, 1, 16),
            0x81a0_0020,
            mem(0x25, 0, 1, 24),
        ],
    );
    m.step().unwrap();
    assert_eq!(m.bus.read32(0x8008).unwrap(), 0x0808_68b0);
    m.step().unwrap();
    assert_eq!(
        m.bus.read_doubleword(0x8010).unwrap(),
        [START, cmp(0x51, 0, 2)]
    );
    assert_eq!(m.cpu.fpu.fsr(), 0x0808_48b0); // drain clears QNE, not FTT/CEXC
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.bus.read32(0x8018).unwrap(), 0x0808_08a0);
}

#[test]
fn pending_exception_is_detected_by_each_floating_instruction_class_before_its_effects() {
    let mut triggers: Vec<_> = [0x20, 0x21, 0x23, 0x24, 0x25, 0x26, 0x27]
        .into_iter()
        .map(|op| mem(op, 0, 1, 0))
        .collect();
    triggers.extend([
        0x3180_0004,
        0x0180_0004,
        0x81a0_0020,
        cmp(0x51, 0, 2),
        0x81a0_0822,
    ]);
    for trigger in triggers {
        let mut m = pending(&[trigger]);
        m.cpu.write_reg(1, 0x20000); // unmapped; pending exception outranks data translation/access
        let entry = m.cpu.fpu.deferred_entry();
        fp_trap(&mut m);
        assert_eq!(m.cpu.fpu.deferred_entry(), entry);
        assert_eq!(m.cpu.fpu.read_reg(0), 0x7f80_0001);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
        assert_eq!(m.cpu.fpu.fsr(), 0x0808_68b0);
    }
}

#[test]
fn privilege_disabled_fpu_and_alignment_outrank_pending_exception_without_acknowledging_it() {
    for (psr, word, address, tt) in [
        (PSR_ET, mem(0x26, 0, 1, 0), 0x20001, 3),
        (PSR_S | PSR_ET, mem(0x20, 0, 1, 0), 0x20001, 4),
        (ENABLED, mem(0x23, 0, 1, 0), 0x20004, 7),
        (ENABLED, mem(0x26, 0, 1, 0), 0x20004, 7),
    ] {
        let mut m = pending(&[0x8188_a000, NOP, NOP, NOP, word]);
        m.cpu.write_reg(2, psr);
        m.cpu.write_reg(1, address);
        for _ in 0..4 {
            m.step().unwrap();
        }
        let fpu = m.cpu.fpu.clone();
        assert!(matches!(m.step().unwrap(),Step::TrapTaken{trap_type,..} if trap_type==tt));
        assert_eq!(m.cpu.fpu, fpu);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
    }
}

#[test]
fn fp_load_or_operation_in_exception_state_defers_sequence_error_without_replacing_queue() {
    for word in [
        mem(0x20, 0, 1, 0),
        mem(0x21, 0, 1, 0),
        mem(0x23, 0, 1, 0),
        0x81a0_0020,
        cmp(0x51, 0, 2),
        0x81a0_0822,
    ] {
        let mut m = pending(&[mem(0x25, 0, 1, 0)]);
        fp_trap(&mut m);
        put(
            &mut m,
            0x1080,
            &[0x8188_a000, NOP, NOP, NOP, word, mem(0x25, 0, 1, 0)],
        );
        m.cpu.write_reg(1, 0x20000); // skipped load must not perform translation/access
        for _ in 0..4 {
            m.step().unwrap();
        }
        let entry = m.cpu.fpu.deferred_entry();
        m.step().unwrap();
        assert_eq!(m.cpu.fpu.state(), FpuState::ExceptionPending);
        assert_eq!(m.cpu.fpu.fsr(), 0x0809_28b0); // only FTT changes to4
        assert_eq!(m.cpu.fpu.read_reg(0), 0x7f80_0001);
        assert_eq!(m.cpu.fpu.deferred_entry(), entry);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
        fp_trap(&mut m);
        assert_eq!(m.cpu.fpu.deferred_entry(), entry);
    }
}

#[test]
fn exception_state_allows_stores_and_branches_while_preserving_the_fault() {
    let mut m = pending(&[mem(0x25, 0, 1, 0)]);
    fp_trap(&mut m);
    put(
        &mut m,
        0x1080,
        &[mem(0x24, 0, 1, 8), mem(0x27, 0, 1, 16), 0x1180_0004, NOP],
    ); // fba +16
    let fpu = m.cpu.fpu.clone();
    for _ in 0..4 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.pc(), 0x1098);
    assert_eq!(m.bus.read32(0x8008).unwrap(), 0x7f80_0001);
    assert_eq!(m.bus.read_doubleword(0x8010).unwrap(), [0x7f80_0001, 0]);
    assert_eq!(m.cpu.fpu, fpu);
}

#[test]
fn failed_queue_store_preserves_entry_and_does_not_partially_write_memory() {
    let mut m = pending(&[mem(0x25, 0, 1, 0)]);
    fp_trap(&mut m);
    put(&mut m, 0x1080, &[mem(0x26, 0, 1, 0)]);
    m.cpu.write_reg(1, 0x20000);
    m.bus.map_ram(0x20000, 4).unwrap();
    m.bus.write32(0x20000, 0xfeed_beef).unwrap();
    let fpu = m.cpu.fpu.clone();
    assert!(matches!(
        m.step().unwrap_err().kind,
        FaultKind::DataAccess(_)
    )); // ET0 in handler
    assert_eq!(m.cpu.fpu, fpu);
    assert_eq!(m.bus.read32(0x20000).unwrap(), 0xfeed_beef);
    assert_eq!(m.cpu.mmu.fault_address(), 0x20000);
}

#[test]
fn unsupported_queue_store_is_retryable_and_only_successful_retry_drains_it() {
    let mut m = pending(&[mem(0x25, 0, 1, 0)]);
    m.bus.install_sun4m(Nvram::default()).unwrap();
    fp_trap(&mut m);
    put(&mut m, 0x1080, &[mem(0x26, 0, 1, 0)]);
    m.cpu.write_reg(1, (IO_CONTROL_BASE + 0x1000) as u32); // doubleword control access unsupported
    let fpu = m.cpu.fpu.clone();
    assert!(matches!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedMemory(_)
    ));
    assert_eq!(m.cpu.fpu, fpu);
    assert!(m.cpu.error_state().is_none());
    m.cpu.write_reg(1, 0x8008);
    m.step().unwrap();
    assert_eq!(
        m.bus.read_doubleword(0x8008).unwrap(),
        [START, cmp(0x51, 0, 2)]
    );
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
}

#[test]
fn no_fault_queue_store_completes_and_drains_while_retaining_mmu_failure_status() {
    let mut m = pending(&[mem(0x25, 0, 1, 0)]);
    fp_trap(&mut m);
    put(&mut m, 0x1080, &[mem(0x26, 0, 1, 0)]);
    m.cpu.write_reg(1, 0x20000);
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_NF)
        .unwrap();
    m.step().unwrap(); // architecturally completed NF store; queue dequeue is committed
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    assert_eq!(m.cpu.mmu.fault_address(), 0x20000);
}

#[test]
fn deferred_trap_in_delay_slot_keeps_target_separate_from_faulting_fpop_address() {
    let mut m = pending(&[0x1080_0008, 0x81a0_0020]); // ba then fmovs in slot
    m.step().unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (START + 8, START + 36));
    fp_trap(&mut m);
    assert_eq!(
        (m.cpu.read_reg(17), m.cpu.read_reg(18)),
        (START + 8, START + 36)
    );
    assert_eq!(m.cpu.fpu.deferred_entry().unwrap().pc, START);
}

#[test]
fn et_zero_delays_cpu_error_until_next_fp_instruction_and_reset_clears_queue() {
    let mut m = setup(
        &[
            0x8188_a000,
            NOP,
            NOP,
            NOP,
            cmp(0x51, 0, 2),
            NOP,
            0x81a0_0020,
        ],
        NV_MASK,
    );
    m.bus.install_sun4m(Nvram::default()).unwrap();
    m.cpu.write_reg(2, PSR_S | PSR_EF);
    operands(&mut m, false, 0x7f80_0001, 0);
    for _ in 0..5 {
        m.step().unwrap();
    }
    assert!(m.cpu.error_state().is_none());
    assert_eq!(m.cpu.fpu.state(), FpuState::ExceptionPending);
    m.step().unwrap();
    let error = m.step().unwrap_err();
    assert_eq!(error.kind, FaultKind::FpException);
    assert_eq!(m.step().unwrap_err(), error);
    assert_eq!(m.bus.read32(RESET_BASE).unwrap(), 0x10);
    m.reset();
    assert_eq!(m.cpu.fpu, Fpu::default());
}

#[test]
fn integer_interrupt_entry_does_not_acknowledge_a_pending_fpu_exception() {
    let mut m = pending(&[NOP]);
    let fpu = m.cpu.fpu.clone();
    assert!(matches!(
        m.cpu.step_with_interrupts(&mut m.bus, 1 << 14).unwrap(),
        Step::InterruptTaken { level: 14, .. }
    ));
    assert_eq!(m.cpu.fpu, fpu);
    put(
        &mut m,
        0x11e0,
        &[0x8188_a000, NOP, NOP, NOP, mem(0x25, 0, 1, 8)],
    );
    for _ in 0..4 {
        m.step().unwrap();
    }
    fp_trap(&mut m);
}
