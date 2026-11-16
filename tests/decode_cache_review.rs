use helios::bus::{Bus, MemoryBus};
use helios::cpu::{Cpu, Step};
use helios::machine::Machine;
use helios::mmu::{Width, PCR_EN};
use helios::system_bus::IO_CONTROL_BASE as IO;

fn pair() -> (Cpu, MemoryBus, Cpu, MemoryBus) {
    let mut cached = Cpu::new(0);
    cached.set_decode_cache_enabled(true);
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x1000).unwrap();
    let mut other_bus = MemoryBus::default();
    other_bus.map_ram(0, 0x1000).unwrap();
    (cached, bus, Cpu::new(0), other_bus)
}

fn psr(cpu: &mut Cpu, bus: &mut MemoryBus, flags: u32, cwp: u32) {
    bus.write32(0x300, 0x8189_e000).unwrap(); // wr %g7, 0, %psr
    cpu.write_reg(7, flags << 20 | 0xc0 | cwp);
    cpu.set_pc_npc(0x300, 0x304);
    assert!(matches!(cpu.step(bus).unwrap(), Step::Executed { .. }));
}

#[test]
fn alu_sethi_operands_windows_and_collision_match_on_miss_and_hit() {
    let (mut cached, mut cb, mut interp, mut ib) = pair();
    let mut ops: Vec<u32> = (0..=8).collect();
    ops.push(12);
    ops.extend(16..=24);
    ops.push(28);
    for op in ops {
        for (rd, rs1, rs2) in [
            (3, 1, 2),
            (1, 1, 2),
            (2, 1, 2),
            (0, 1, 2),
            (16, 16, 24),
            (24, 16, 24),
        ] {
            for imm in [false, true] {
                for (a, b) in [
                    (0, 0),
                    (u32::MAX, 1),
                    (0x7fff_ffff, 1),
                    (0x8000_0000, u32::MAX),
                    (0x1234_5678, 0x8765_4321),
                ] {
                    let word = 0x8000_0000
                        | rd << 25
                        | op << 19
                        | rs1 << 14
                        | if imm { 0x2000 | (b & 0x1fff) } else { rs2 };
                    for flags in [0, 1, 15] {
                        // Same PC and word repeated: insertion followed by real cache hits.
                        for _ in 0..2 {
                            for (cpu, bus) in [(&mut cached, &mut cb), (&mut interp, &mut ib)] {
                                psr(cpu, bus, flags, 7);
                                cpu.write_reg(rs1 as u8, a);
                                cpu.write_reg(rs2 as u8, b);
                                bus.write32(0, word).unwrap();
                                cpu.set_pc_npc(0, 4);
                            }
                            assert_eq!(cached.step(&mut cb), interp.step(&mut ib), "{word:08x}");
                            assert_eq!(cached, interp, "{word:08x}");
                        }
                    }
                }
            }
        }
    }
    for (pc, word) in [
        (0, 0x033f_ffff),
        (0x100, 0x033f_ffff),
        (0, 0x0100_0000),
        (0, 0x2100_0001),
    ] {
        for _ in 0..2 {
            cb.write32(pc, word).unwrap();
            ib.write32(pc, word).unwrap();
            cached.set_pc_npc(pc as u32, pc as u32 + 4);
            interp.set_pc_npc(pc as u32, pc as u32 + 4);
            assert_eq!(cached.step(&mut cb), interp.step(&mut ib));
            assert_eq!(cached, interp);
        }
    }
}

#[test]
fn every_bicc_condition_annul_and_displacement_matches_on_miss_and_hit() {
    let (mut cached, mut cb, mut interp, mut ib) = pair();
    for flags in 0..16 {
        let (n, z, v, c) = (
            flags & 8 != 0,
            flags & 4 != 0,
            flags & 2 != 0,
            flags & 1 != 0,
        );
        let expected = [
            false,
            z,
            z || (n != v),
            n != v,
            c || z,
            c,
            n,
            v,
            true,
            !z,
            !z && (n == v),
            n == v,
            !c && !z,
            !c,
            !n,
            !v,
        ];
        for (cond, taken) in expected.into_iter().enumerate() {
            for annul in [false, true] {
                for displacement in [0i32, 8, -8, 0x7ffffc, -0x800000] {
                    let word = 0x0080_0000
                        | (cond as u32) << 25
                        | u32::from(annul) << 29
                        | ((displacement as u32 >> 2) & 0x3f_ffff);
                    for _ in 0..2 {
                        for (cpu, bus) in [(&mut cached, &mut cb), (&mut interp, &mut ib)] {
                            psr(cpu, bus, flags, 0);
                            bus.write32(0x200, word).unwrap();
                            cpu.set_pc_npc(0x200, 0x88); // Branch in a nonsequential delay slot.
                        }
                        assert_eq!(cached.step(&mut cb), interp.step(&mut ib));
                        let target = if taken {
                            0x200u32.wrapping_add(displacement as u32)
                        } else {
                            0x8c
                        };
                        let expected_pc = if annul && (!taken || cond == 8) {
                            (target, target.wrapping_add(4))
                        } else {
                            (0x88, target)
                        };
                        assert_eq!(
                            (cached.pc(), cached.npc()),
                            expected_pc,
                            "{word:08x} flags={flags}"
                        );
                        assert_eq!(cached, interp);
                    }
                }
            }
        }
    }
}

#[test]
fn enabled_cache_observes_guest_store_code_replacement_and_fallback_trap() {
    let (mut cached, mut cb, mut interp, mut ib) = pair();
    for (cpu, bus) in [(&mut cached, &mut cb), (&mut interp, &mut ib)] {
        bus.write32(0, 0x8200_6001).unwrap(); // add g1,1,g1
        for _ in 0..2 {
            cpu.set_pc_npc(0, 4);
            cpu.step(bus).unwrap();
        }
        assert_eq!(cpu.read_reg(1), 2);
        bus.write32(8, 0xc420_2000).unwrap(); // st g2,[g0]
        cpu.write_reg(2, 0x8200_6002);
        cpu.set_pc_npc(8, 12);
        cpu.step(bus).unwrap();
        cpu.set_pc_npc(0, 4);
        cpu.step(bus).unwrap();
        assert_eq!(cpu.read_reg(1), 4);
    }
    assert_eq!(cached, interp);
    // Replacing a cached arithmetic plan with UNIMP must report a precise error.
    cb.write32(0, 0).unwrap();
    ib.write32(0, 0).unwrap();
    cached.set_pc_npc(0, 4);
    interp.set_pc_npc(0, 4);
    assert_eq!(cached.step(&mut cb), interp.step(&mut ib));
    assert!(cached.error_state().is_some());
    assert_eq!(cached, interp);
}

fn paged(cache: bool) -> Machine {
    let mut m = Machine::bare_metal(0x20000, 0x1234_5000).unwrap();
    m.cpu.set_decode_cache_enabled(cache);
    for (addr, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x88e),
        (0x8000, 0x8200_6001),
        (0x9000, 0x8200_6002),
    ] {
        m.bus.write32(addr, word).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m
}

#[test]
fn warmed_cache_still_fetches_after_remap_and_execute_permission_removal() {
    let mut cached = paged(true);
    let mut interp = paged(false);
    for pte in [0x88e, 0x88e, 0x98e, 0x986] {
        // Final ACC=1: RW, no instruction execute.
        for m in [&mut cached, &mut interp] {
            m.bus.write32(0x4014, pte).unwrap();
            for scope in 0..=4 {
                m.cpu
                    .mmu
                    .store(&mut m.bus, 0x1234_5000 | (scope << 8), 3, Width::Word, 0)
                    .unwrap();
            }
            m.cpu.set_pc_npc(0x1234_5000, 0x1234_5004);
        }
        let a = cached.step();
        let b = interp.step();
        assert_eq!(a, b);
        assert_eq!(a.is_err(), pte == 0x986);
        assert_eq!(
            cached
                .diagnostic_snapshot()
                .differences(&interp.diagnostic_snapshot()),
            Vec::<&str>::new()
        );
    }
    assert_eq!(cached.cpu.read_reg(1), 4);
}

#[test]
fn enabled_cache_observes_iommu_dma_code_write() {
    fn fixture(cache: bool) -> Machine {
        let mut m = Machine::sun4m(0x20000, &0x0100_0000u32.to_be_bytes()).unwrap();
        m.cpu.mmu.store(&mut m.bus, 0, 4, Width::Word, 0).unwrap();
        m.cpu.set_decode_cache_enabled(cache);
        m.bus.write32(IO, 1).unwrap();
        m.bus.write32(IO + 4, 0x1000).unwrap();
        m.bus.write32(IO + 0x2000, 0x003f_0000).unwrap();
        m.bus.write32(0x10000, 0x806).unwrap();
        m.bus.write32(0x8000, 0x8200_6001).unwrap();
        m
    }
    let mut cached = fixture(true);
    let mut interp = fixture(false);
    for phase in 0..3 {
        for m in [&mut cached, &mut interp] {
            if phase == 2 {
                m.dma_write(5, 0xff00_0000, Width::Word, 0x8200_6002)
                    .unwrap();
            }
            m.cpu.set_pc_npc(0x8000, 0x8004);
        }
        assert_eq!(cached.step(), interp.step());
        assert_eq!(
            cached
                .diagnostic_snapshot()
                .differences(&interp.diagnostic_snapshot()),
            Vec::<&str>::new()
        );
    }
    assert_eq!(cached.cpu.read_reg(1), 4);
}

#[test]
fn cached_branch_does_not_fetch_annulled_faulting_slot_and_interrupt_wins() {
    let (mut cached, mut cb, mut interp, mut ib) = pair();
    for (cpu, bus) in [(&mut cached, &mut cb), (&mut interp, &mut ib)] {
        bus.write32(0xffc, 0x3080_0000).unwrap(); // ba,a .; 0x1000 unmapped
        cpu.set_pc_npc(0xffc, 0x1000);
    }
    for _ in 0..2 {
        assert_eq!(cached.step(&mut cb), interp.step(&mut ib));
        assert_eq!((cached.pc(), cached.npc()), (0xffc, 0x1000));
        assert_eq!(cached, interp);
    }
    for (cpu, bus) in [(&mut cached, &mut cb), (&mut interp, &mut ib)] {
        bus.write32(0x300, 0x8188_20e0).unwrap(); // wr g0, S|PS|ET,psr
        cpu.set_pc_npc(0x300, 0x304);
        cpu.step(bus).unwrap();
        cpu.set_pc_npc(0xffc, 0x1000);
    }
    let a = cached.step_with_interrupts(&mut cb, 1 << 10);
    let b = interp.step_with_interrupts(&mut ib, 1 << 10);
    assert_eq!(a, b);
    assert!(matches!(a.unwrap(), Step::InterruptTaken { level: 10, .. }));
    assert_eq!(cached, interp);
}
