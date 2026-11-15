use helios::bus::Bus;
use helios::devices::interrupt::MASK_ALL;
use helios::devices::serial::Channel;
use helios::iommu::StandaloneIommu;
use helios::machine::{Machine, MachineProfile, SchedulerError, STEP_NS};
use helios::mmu::Width;

fn dual(profile: MachineProfile) -> Machine {
    Machine::with_profile_processors(
        profile,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        profile.default_nvram(),
        2,
    )
    .unwrap()
}

fn loop_image(cpu0: bool) -> [u8; 8] {
    let add = if cpu0 { 0x8200_6001u32 } else { 0x8400_6001 };
    [
        0x10,
        0x80,
        0,
        0,
        add.to_be_bytes()[0],
        add.to_be_bytes()[1],
        add.to_be_bytes()[2],
        add.to_be_bytes()[3],
    ]
}

#[test]
fn two_cpu_construction_is_explicit_and_ss5_rejects_two() {
    assert!(Machine::with_profile_processors(
        MachineProfile::Ss5,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        MachineProfile::Ss5.default_nvram(),
        2,
    )
    .is_err());
    let m = dual(MachineProfile::Ss20);
    assert!(m.secondary_cpu().is_some());
    assert_eq!(m.secondary_cpu().unwrap().module.processor, 2);
}

#[test]
fn both_hardware_processors_progress_with_tagged_counts_and_repeatable_order() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        m.load_image(0x1000, &loop_image(true)).unwrap();
        m.load_image(0x2000, &loop_image(false)).unwrap();
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        assert!(m.set_processor_arbitration(2, true));

        let mut trace = Vec::new();
        for _ in 0..2000 {
            let event = m.step_scheduled().unwrap();
            trace.push((event.processor, event.step, event.virtual_ns));
        }
        assert!(m.cpu.read_reg(1) > 0, "{profile:?}");
        assert!(
            m.secondary_cpu().unwrap().cpu.read_reg(2) > 0,
            "{profile:?}"
        );
        let stats = m.scheduler_stats();
        assert_eq!(stats.processor0.processor, 0);
        assert_eq!(stats.processor2.processor, 2);
        assert_eq!(stats.total_instructions(), 2000);
        assert_eq!(stats.processor0.instructions, 1000);
        assert_eq!(stats.processor2.instructions, 1000);
        assert_eq!(stats.total_errors(), 0);
        assert!(trace.iter().any(|(processor, _, _)| *processor == 0));
        assert!(trace.iter().any(|(processor, _, _)| *processor == 2));
        assert_eq!(m.scheduler_stats().virtual_ns, 100_000);

        let mut replay = dual(profile);
        replay.load_image(0x1000, &loop_image(true)).unwrap();
        replay.load_image(0x2000, &loop_image(false)).unwrap();
        assert!(replay.set_processor_entry(0, 0x1000, 0x1004));
        assert!(replay.set_processor_entry(2, 0x2000, 0x2004));
        assert!(replay.set_processor_arbitration(2, true));
        let mut trace2 = Vec::new();
        for _ in 0..2000 {
            let event = replay.step_scheduled().unwrap();
            trace2.push((event.processor, event.step, event.virtual_ns));
        }
        assert_eq!(trace, trace2, "{profile:?}");
    }
}

#[test]
fn denied_processor_does_not_execute_or_count_and_shared_timer_progress_matches_virtual_time() {
    let mut single = dual(MachineProfile::Ss20);
    single.load_image(0x1000, &loop_image(true)).unwrap();
    single.load_image(0x2000, &loop_image(false)).unwrap();
    assert!(single.set_processor_entry(0, 0x1000, 0x1004));
    assert!(single.set_processor_entry(2, 0x2000, 0x2004));
    // CPU 2 is present but arbitration-disabled at reset.
    assert!(!single.processor_arbitration(2).is_enabled());
    for _ in 0..100 {
        let event = single.step_scheduled().unwrap();
        assert_eq!(event.processor, 0);
    }
    assert_eq!(single.scheduler_stats().processor2.instructions, 0);
    let single_timer = single
        .bus
        .read32(single.bus.system_timer_base() + 4)
        .unwrap();
    let single_time = single.scheduler_stats().virtual_ns;

    let mut both = dual(MachineProfile::Ss20);
    both.load_image(0x1000, &loop_image(true)).unwrap();
    both.load_image(0x2000, &loop_image(false)).unwrap();
    assert!(both.set_processor_entry(0, 0x1000, 0x1004));
    assert!(both.set_processor_entry(2, 0x2000, 0x2004));
    assert!(both.set_processor_arbitration(2, true));
    for _ in 0..200 {
        both.step_scheduled().unwrap();
    }
    assert_eq!(both.scheduler_stats().processor0.instructions, 100);
    assert_eq!(both.scheduler_stats().processor2.instructions, 100);
    assert_eq!(both.scheduler_stats().virtual_ns, single_time);
    assert_eq!(
        both.bus.read32(both.bus.system_timer_base() + 4).unwrap(),
        single_timer
    );
}

#[test]
fn secondary_error_is_tagged_and_stops_without_advancing_virtual_time() {
    let mut m = dual(MachineProfile::Ss20);
    m.load_image(0x1000, &loop_image(true)).unwrap();
    // Zero word at the second entry is an illegal instruction.
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    assert!(m.set_processor_entry(2, 0x3000, 0x3004));
    assert!(m.set_processor_arbitration(2, true));
    let before = m.scheduler_stats().virtual_ns;
    let error = loop {
        match m.step_scheduled() {
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert!(matches!(error, SchedulerError::Cpu { processor: 2, .. }));
    assert!(m.scheduler_stats().virtual_ns >= before);
    assert_eq!(m.scheduler_stats().processor2.errors, 1);
    let repeated = loop {
        match m.step_scheduled() {
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert!(matches!(repeated, SchedulerError::Cpu { processor: 2, .. }));
    assert_eq!(m.scheduler_stats().processor2.errors, 1);
    assert!(m.cpu.error_state().is_none());
}

#[test]
fn directed_software_ipi_reaches_cpu_two_and_returns_through_trap_code() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        m.load_image(0x1000, &loop_image(true)).unwrap();
        let setup = [
            0x0300_0004u32, // sethi %hi(0x1000), %g1
            0x8198_6000,    // wr %g1, 0, %tbr
            0x0100_0000,
            0x0100_0000,
            0x8188_20a0, // wr %g0, 0xa0, %psr (S+ET)
            0x0100_0000,
            0x0100_0000,
            0x1080_0000, // ba .
            0x0100_0000,
        ];
        m.load_image(
            0x2000,
            &setup
                .into_iter()
                .flat_map(u32::to_be_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        // Level-1 software vector: TBR 0x1000 + 0x11 << 4 = 0x1110.
        let handler = [0x8a01_6001u32, 0x81c4_6000, 0x81cc_a000];
        m.load_image(
            0x1110,
            &handler
                .into_iter()
                .flat_map(u32::to_be_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        m.secondary_cpu_mut()
            .unwrap()
            .cpu
            .set_window_invalid_mask(0);
        assert!(m.set_processor_arbitration(2, true));
        // Allow CPU 2 to install its TBR and enable traps.
        for _ in 0..40 {
            m.step_scheduled().unwrap();
        }
        m.bus
            .write32(m.bus.system_interrupt_base() + 8, MASK_ALL)
            .unwrap();
        let bank = m.bus.processor_interrupt_base(2);
        m.bus.write32(bank + 8, 1 << 17).unwrap();
        let mut taken = None;
        for _ in 0..40 {
            let event = m.step_scheduled().unwrap();
            if let helios::cpu::Step::InterruptTaken { level, .. } = event.step {
                if event.processor == 2 && level == 1 {
                    taken = Some(event);
                    break;
                }
            }
        }
        assert!(taken.is_some(), "{profile:?}");
        m.bus.write32(bank + 4, 1 << 17).unwrap();
        for _ in 0..20 {
            m.step_scheduled().unwrap();
        }
        assert_eq!(m.secondary_cpu().unwrap().cpu.read_reg(5), 1, "{profile:?}");
        assert!(m.scheduler_stats().processor2.interrupts >= 1);
    }
}

#[test]
fn guest_aer_write_admits_cpu_two_only_at_the_next_scheduler_boundary() {
    let aer_program = [
        0x0338_0000u32, // sethi %hi(0xe0000000), %g1
        0x8200_6fff,    // add %g1, 0xfff, %g1
        0x8200_6009,    // add %g1, 9, %g1 -> 0xe0001008
        0x8410_2004,    // or %g0, 4, %g2 (AER bit 2)
        0xc4a0_45e0,    // sta %g2, [%g1] 0x2f
        0x1080_0000,    // ba .
        0x0100_0000,
    ];
    let bytes =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_be_bytes()).collect() };
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        m.load_image(0x1000, &bytes(&aer_program)).unwrap();
        m.load_image(0x2000, &loop_image(false)).unwrap();
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        assert!(!m.processor_arbitration(2).is_enabled());
        let mut saw_aer = false;
        let mut first_cpu2_after_store = false;
        for _ in 0..30 {
            let event = m.step_scheduled().unwrap();
            if let helios::cpu::Step::Executed { pc, .. } = event.step {
                if event.processor == 0 && pc == 0x1010 {
                    saw_aer = true;
                    assert!(m.processor_arbitration(2).is_enabled());
                }
            }
            if saw_aer && event.processor == 2 {
                first_cpu2_after_store = true;
                break;
            }
        }
        assert!(saw_aer, "{profile:?}");
        assert!(first_cpu2_after_store, "{profile:?}");
        assert!(m.secondary_cpu().unwrap().cpu.pc() != 0x2000);
    }
}

#[test]
fn module_reset_preserves_the_other_cpu_and_whole_reset_resets_both_once() {
    let mut m = dual(MachineProfile::Ss20);
    m.load_image(0x1000, &loop_image(true)).unwrap();
    m.load_image(0x2000, &loop_image(false)).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    assert!(m.set_processor_entry(2, 0x2000, 0x2004));
    m.cpu.write_reg(3, 0x1234_5678);
    m.secondary_cpu_mut().unwrap().cpu.write_reg(3, 0x8765_4321);
    assert!(m.reset_processor_module(2));
    assert_eq!(m.cpu.read_reg(3), 0x1234_5678);
    assert_eq!(m.secondary_cpu().unwrap().cpu.read_reg(3), 0);
    assert!(m.reset_processor_module(0));
    assert_eq!(m.cpu.read_reg(3), 0);
    m.cpu.write_reg(3, 0x1111_2222);
    m.secondary_cpu_mut().unwrap().cpu.write_reg(3, 0x3333_4444);
    m.reset();
    assert_eq!(m.cpu.read_reg(3), 0);
    assert_eq!(m.secondary_cpu().unwrap().cpu.read_reg(3), 0);
}

#[test]
fn guest_swap_and_ldstub_locks_are_shared_without_starvation() {
    fn mem(op3: u32, rd: u32, rs1: u32) -> u32 {
        0xc000_0000 | (rd << 25) | (op3 << 19) | (rs1 << 14) | 0x2000
    }
    for (op3, operation) in [(0x0d, "ldstub"), (0x0f, "swap")] {
        let lock = [
            0x0300_0020u32, // sethi %hi(0x8000), %g1
            0x8410_2001,    // or %g0, 1, %g2
            mem(op3, 2, 1),
            0x80a0_8000, // subcc %g2, %g0, %g0
            0x12bf_fffc, // bne retry
            0x0100_0000,
            0xc020_6000, // st %g0, [%g1] (release)
            0x8600_e001, // add %g3, 1, %g3
            0x10bf_fff8, // ba retry
            0x0100_0000,
        ];
        let bytes: Vec<u8> = lock.iter().flat_map(|word| word.to_be_bytes()).collect();
        for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
            let mut m = dual(profile);
            m.load_image(0x1000, &bytes).unwrap();
            m.load_image(0x2000, &bytes).unwrap();
            assert!(m.set_processor_entry(0, 0x1000, 0x1004));
            assert!(m.set_processor_entry(2, 0x2000, 0x2004));
            assert!(m.set_processor_arbitration(2, true));
            assert_eq!(m.bus.read32(0x8000).unwrap(), 0);
            for _ in 0..2000 {
                m.step_scheduled().unwrap();
            }
            assert!(m.cpu.read_reg(3) > 0, "{profile:?} {operation}");
            assert!(
                m.secondary_cpu().unwrap().cpu.read_reg(3) > 0,
                "{profile:?} {operation}"
            );
            assert_eq!(m.bus.read32(0x8000).unwrap(), 0, "{profile:?} {operation}");
        }
    }
}

#[test]
fn cpu_tlbs_are_independent_and_flushing_one_cpu_preserves_the_other() {
    use helios::mmu::PCR_EN;
    fn configure(cpu: &mut helios::cpu::Cpu, bus: &mut helios::system_bus::SystemBus) {
        cpu.mmu.store(bus, 1 << 8, 4, Width::Word, 0x100).unwrap();
        cpu.mmu.store(bus, 0, 4, Width::Word, PCR_EN).unwrap();
    }
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = Machine::with_profile_processors(
            profile,
            // Both mappings must have distinct backing storage. The Axil
            // 48 MiB population is three 16 MiB banks, so +16/+32 MiB in
            // bank 0 mirror its base instead of naming independent pages.
            0x0400_0000,
            &0x0100_0000u32.to_be_bytes(),
            profile.default_nvram(),
            2,
        )
        .unwrap();
        m.bus.write32(0x1000, 0x201).unwrap();
        m.bus.write32(0x2000, 0x0010_008e).unwrap();
        m.bus.write32(0x0100_0000, 0x1111_1111).unwrap();
        m.bus.write32(0x0200_0000, 0x2222_2222).unwrap();
        configure(&mut m.cpu, &mut m.bus);
        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        configure(&mut secondary.cpu, bus);
        assert_eq!(
            m.cpu.mmu.load(&mut m.bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111,
            "{profile:?}"
        );
        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        assert_eq!(
            secondary.cpu.mmu.load(bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111,
            "{profile:?}"
        );
        m.bus.write32(0x2000, 0x0020_008e).unwrap();
        m.cpu
            .mmu
            .store(&mut m.bus, 4 << 8, 3, Width::Word, 0)
            .unwrap();
        assert_eq!(
            m.cpu.mmu.load(&mut m.bus, 0, 11, Width::Word).unwrap(),
            0x2222_2222,
            "{profile:?}"
        );
        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        assert_eq!(
            secondary.cpu.mmu.load(bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111,
            "{profile:?}"
        );
        secondary
            .cpu
            .mmu
            .store(bus, 4 << 8, 3, Width::Word, 0)
            .unwrap();
        assert_eq!(
            secondary.cpu.mmu.load(bus, 0, 11, Width::Word).unwrap(),
            0x2222_2222,
            "{profile:?}"
        );
    }
}

#[test]
fn shared_ram_publication_is_visible_across_hardware_processors() {
    let writer = [
        0x0300_0020u32, // sethi %hi(0x8000), %g1
        0x8410_2001,    // or %g0, 1, %g2
        0x8610_2123,    // or %g0, 0x123, %g3
        0xc620_6004,    // st %g3, [%g1 + 4]
        0xc420_6000,    // st %g2, [%g1]
        0x1080_0000,    // ba .
        0x0100_0000,
    ];
    let reader = [
        0x0300_0020u32, // sethi %hi(0x8000), %g1
        0xc400_6000,    // ld [%g1], %g2
        0x80a0_8000,    // subcc %g2, %g0, %g0
        0x02bf_fffd,    // be spin
        0x0100_0000,
        0xc600_6004, // ld [%g1 + 4], %g3
        0x8801_2001, // add %g4, 1, %g4
        0x1080_0000, // ba .
        0x0100_0000,
    ];
    let bytes =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_be_bytes()).collect() };
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        m.bus.write32(0x8000, 0).unwrap();
        m.bus.write32(0x8004, 0).unwrap();
        m.load_image(0x1000, &bytes(&writer)).unwrap();
        m.load_image(0x2000, &bytes(&reader)).unwrap();
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        assert!(m.set_processor_arbitration(2, true));
        for _ in 0..100 {
            m.step_scheduled().unwrap();
        }
        assert_eq!(m.bus.read32(0x8000).unwrap(), 1, "{profile:?}");
        assert_eq!(m.bus.read32(0x8004).unwrap(), 0x123, "{profile:?}");
        assert_eq!(
            m.secondary_cpu().unwrap().cpu.read_reg(3),
            0x123,
            "{profile:?}"
        );
        assert!(m.secondary_cpu().unwrap().cpu.read_reg(4) >= 1);
    }
}

#[test]
fn processor_local_register_fpu_and_program_counter_state_is_not_mirrored() {
    let mut m = dual(MachineProfile::Ss20);
    m.cpu.write_reg(1, 0x1111_1111);
    m.secondary_cpu_mut().unwrap().cpu.write_reg(1, 0x2222_2222);
    m.cpu.fpu.write_reg(0, 0x3f80_0000);
    m.secondary_cpu_mut()
        .unwrap()
        .cpu
        .fpu
        .write_reg(0, 0x4000_0000);
    assert_eq!(m.cpu.read_reg(1), 0x1111_1111);
    assert_eq!(m.secondary_cpu().unwrap().cpu.read_reg(1), 0x2222_2222);
    assert_eq!(m.cpu.fpu.read_reg(0), 0x3f80_0000);
    assert_eq!(m.secondary_cpu().unwrap().cpu.fpu.read_reg(0), 0x4000_0000);
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    assert!(m.set_processor_entry(2, 0x2000, 0x2004));
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0x1000, 0x1004));
    assert_eq!(
        (
            m.secondary_cpu().unwrap().cpu.pc(),
            m.secondary_cpu().unwrap().cpu.npc()
        ),
        (0x2000, 0x2004)
    );
}

#[test]
fn per_cpu_cadence_and_system_and_processor_timers_match_up_work() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut single = dual(profile);
        single.load_image(0x1000, &loop_image(true)).unwrap();
        assert!(single.set_processor_entry(0, 0x1000, 0x1004));
        for _ in 0..1000 {
            single.step_scheduled().unwrap();
        }
        assert_eq!(single.scheduler_stats().virtual_ns, 1000 * STEP_NS);

        let mut both = dual(profile);
        both.load_image(0x1000, &loop_image(true)).unwrap();
        both.load_image(0x2000, &loop_image(false)).unwrap();
        assert!(both.set_processor_entry(0, 0x1000, 0x1004));
        assert!(both.set_processor_entry(2, 0x2000, 0x2004));
        assert!(both.set_processor_arbitration(2, true));
        for _ in 0..2000 {
            both.step_scheduled().unwrap();
        }
        let stats = both.scheduler_stats();
        assert_eq!(stats.processor0.instructions, 1000, "{profile:?}");
        assert_eq!(stats.processor2.instructions, 1000, "{profile:?}");
        assert_eq!(stats.virtual_ns, single.scheduler_stats().virtual_ns);
        assert_eq!(
            both.bus.read32(both.bus.system_timer_base() + 4).unwrap(),
            single
                .bus
                .read32(single.bus.system_timer_base() + 4)
                .unwrap(),
            "{profile:?}"
        );
        assert_eq!(
            both.bus
                .read32(both.bus.processor_timer_base(0) + 4)
                .unwrap(),
            both.bus
                .read32(both.bus.processor_timer_base(2) + 4)
                .unwrap(),
            "{profile:?}"
        );
    }
}

#[test]
fn prom_wait_is_charged_only_to_the_requesting_processor() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        m.bus.map_rom(0x7000_0000, &loop_image(true)).unwrap();
        m.load_image(0x2000, &loop_image(false)).unwrap();
        assert!(m.set_processor_entry(0, 0x7000_0000, 0x7000_0004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        assert!(m.set_processor_arbitration(2, true));
        let first = m.step_scheduled().unwrap();
        let second = m.step_scheduled().unwrap();
        let third = m.step_scheduled().unwrap();
        assert_eq!(first.processor, 0, "{profile:?}");
        assert_eq!(second.processor, 2, "{profile:?}");
        assert_eq!(third.processor, 2, "{profile:?}");
        assert!(m.secondary_cpu().unwrap().cpu.read_reg(2) > 0);
    }
}

fn warm_standalone_iommu(m: &mut Machine) {
    let io = m.bus.layout().io_control;
    m.bus.write32(io, 1).unwrap();
    m.bus.write32(io + 4, 0x1000).unwrap();
    m.bus.write32(io + 0x1008, 1 << 20).unwrap();
    m.bus.write32(0x10000, 0x806).unwrap();
    m.bus.write32(0x8000, 0x1111_1111).unwrap();
    m.bus.write32(0x9000, 0x2222_2222).unwrap();
    assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1111_1111));
    assert_ne!(m.standalone_iommu().unwrap(), &StandaloneIommu::default());
}

#[test]
fn software_reset_clears_standalone_iommu_for_both_step_paths() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut scheduled = dual(profile);
        warm_standalone_iommu(&mut scheduled);
        scheduled
            .bus
            .write32(scheduled.bus.layout().reset, 1)
            .unwrap();
        assert!(matches!(
            scheduled.step_scheduled().unwrap().step,
            helios::cpu::Step::SoftwareReset { .. }
        ));
        scheduled.bus.write32(0x10000, 0x906).unwrap();
        let io = scheduled.bus.layout().io_control;
        scheduled.bus.write32(io, 1).unwrap();
        scheduled.bus.write32(io + 4, 0x1000).unwrap();
        scheduled.bus.write32(io + 0x1008, 1 << 20).unwrap();
        assert_eq!(
            scheduled.dma_read(5, 0xff00_0000, Width::Word),
            Ok(0x2222_2222),
            "{profile:?} scheduled"
        );

        let mut legacy = dual(profile);
        warm_standalone_iommu(&mut legacy);
        legacy.bus.write32(legacy.bus.layout().reset, 1).unwrap();
        assert!(matches!(
            legacy.step().unwrap(),
            helios::cpu::Step::SoftwareReset { .. }
        ));
        legacy.bus.write32(0x10000, 0x906).unwrap();
        legacy.bus.write32(io, 1).unwrap();
        legacy.bus.write32(io + 4, 0x1000).unwrap();
        legacy.bus.write32(io + 0x1008, 1 << 20).unwrap();
        assert_eq!(
            legacy.dma_read(5, 0xff00_0000, Width::Word),
            Ok(0x2222_2222),
            "{profile:?} legacy"
        );
    }
}

#[test]
fn whole_reset_clears_and_module_reset_preserves_standalone_iommu() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        warm_standalone_iommu(&mut m);
        let before = m.standalone_iommu().unwrap().clone();
        assert!(m.reset_processor_module(2));
        assert_eq!(m.standalone_iommu(), Some(&before));
        m.reset();
        assert_eq!(m.standalone_iommu(), Some(&StandaloneIommu::default()));
    }
}

#[test]
fn guest_ipi_performs_a_local_tlb_shootdown_and_publishes_completion() {
    use helios::mmu::PCR_EN;
    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_be_bytes()).collect()
    }
    fn be(target: usize, pc: usize) -> u32 {
        0x0280_0000 | (((target as i32 - pc as i32) as u32) & 0x003f_ffff)
    }
    let mut cpu0 = vec![
        0x0338_0000u32, // sethi %hi(0xe0000000), %g1
        0x8200_6fff,    // add %g1, 0xfff, %g1
        0x8200_6009,    // add %g1, 9, %g1 -> AER
        0x8410_2004,    // or %g0, 4, %g2
        0xc4a0_45e0,    // sta %g2, [%g1] 0x2f (guest AER enable)
        0x0300_0024,    // sethi %hi(0x9000), %g1
        0xc400_6000,    // ld [%g1], %g2
        0x80a0_8000,    // subcc %g2, %g0, %g0
        0,
        0x0100_0000,
        0xc400_6008, // ld [%g1 + 8], %g2 (host gate)
        0x80a0_8000,
        0,
        0x0100_0000,
        0x033c_5008, // sethi %hi(0xf1402000), %g1
        0x8210_6008, // or %g1, 8, %g1 (CPU 2 set bank)
        0x0500_0080, // sethi %hi(0x20000), %g2
        0xc4a0_45e0, // sta %g2, [%g1] 0x2f (guest IPI)
        0x0300_0024, // sethi %hi(0x9000), %g1
        0x8200_6004, // add %g1, 4, %g1
        0xc400_6000, // ld [%g1], %g2
        0x80a0_8000,
        0,
        0x0100_0000,
        0x1080_0000, // ba .
        0x0100_0000,
    ];
    cpu0[8] = be(6, 8);
    cpu0[12] = be(10, 12);
    cpu0[22] = be(20, 22);

    let mut setup = vec![
        0x0300_0018u32, // sethi %hi(0x6000), %g1
        0x8198_6000,    // wr %g1, 0, %tbr
        0x0100_0000,
        0x0100_0000,
        0x8188_20a0, // wr %g0, 0xa0, %psr
        0x0100_0000,
        0x0100_0000,
        0x0300_0024, // sethi %hi(0x9000), %g1
        0x8410_2001, // or %g0, 1, %g2
        0xc4a0_4400, // sta %g2, [%g1] 0x20 (ready)
        0x8600_e001, // add %g3, 1, %g3
        0,
        0x0100_0000,
    ];
    setup[11] = 0x10bf_ffff; // ba back to add

    // Every instruction in the replacement page increments g4, so a return
    // to any old loop PC proves that the handler's local flush took effect.
    let new_page = vec![0x8801_2001u32; 1024]; // add %g4, 1, %g4

    let handler = [
        0x8210_2300u32, // or %g0, 0x300, %g1
        0xc0a0_4060,    // sta %g0, [%g1] 3 (local context flush)
        0x033c_5008,    // sethi %hi(0xf1402000), %g1
        0x8210_6004,    // or %g1, 4, %g1 (CPU 2 clear bank)
        0x0500_0080,    // sethi %hi(0x20000), %g2
        0xc4a0_45e0,    // sta %g2, [%g1] 0x2f (IPI acknowledge)
        0x0300_0024,    // sethi %hi(0x9000), %g1
        0x8200_6004,    // add %g1, 4, %g1
        0x8410_2001,    // or %g0, 1, %g2
        0xc4a0_4400,    // sta %g2, [%g1] 0x20 (completion)
        0x81c4_6000,    // jmpl %l1, %g0
        0x81cc_a000,    // rett %l2
    ];

    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        // VA 0x5000 -> PA 0x5000, VA 0x6000 -> PA 0x6000.
        m.bus.write32(0x1000, 0x201).unwrap();
        m.bus.write32(0x2000, 0x301).unwrap();
        m.bus.write32(0x3000, 0x401).unwrap();
        m.bus.write32(0x4014, 0x0000_058e).unwrap();
        m.bus.write32(0x4018, 0x0000_068e).unwrap();
        m.load_image(0x7000, &bytes(&cpu0)).unwrap();
        m.load_image(0x5000, &bytes(&setup)).unwrap();
        m.load_image(0x8000, &bytes(&new_page)).unwrap();
        m.load_image(0x6110, &bytes(&handler)).unwrap();
        m.bus.write32(0x9000, 0).unwrap();
        m.bus.write32(0x9004, 0).unwrap();
        m.bus.write32(0x9008, 0).unwrap();
        assert!(m.set_processor_entry(0, 0x7000, 0x7004));
        assert!(m.set_processor_entry(2, 0x5000, 0x5004));
        {
            let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
            secondary.cpu.set_window_invalid_mask(0);
            secondary
                .cpu
                .mmu
                .store(bus, 1 << 8, 4, Width::Word, 0x100)
                .unwrap();
            secondary
                .cpu
                .mmu
                .store(bus, 2 << 8, 4, Width::Word, 0)
                .unwrap();
            secondary
                .cpu
                .mmu
                .store(bus, 0, 4, Width::Word, PCR_EN)
                .unwrap();
        }
        m.bus
            .write32(m.bus.system_interrupt_base() + 8, MASK_ALL)
            .unwrap();

        let mut saw_aer = false;
        let mut saw_ipi = false;
        let mut ready = false;
        for _ in 0..400 {
            let event = m.step_scheduled().unwrap();
            if let helios::cpu::Step::Executed { pc, .. } = event.step {
                if event.processor == 0 && pc == 0x7010 {
                    saw_aer = true;
                }
                if event.processor == 0 && pc == 0x7044 {
                    saw_ipi = true;
                }
            }
            if m.bus.read32(0x9000).unwrap() != 0 {
                ready = true;
                break;
            }
        }
        assert!(saw_aer, "{profile:?}");
        assert!(ready, "{profile:?}");

        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        let old = secondary
            .cpu
            .mmu
            .load(bus, 0x5000, 11, Width::Word)
            .unwrap();
        assert_eq!(old, setup[0], "{profile:?}");
        m.bus.write32(0x4014, 0x0000_088e).unwrap();
        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        assert_eq!(
            secondary
                .cpu
                .mmu
                .load(bus, 0x5000, 11, Width::Word)
                .unwrap(),
            old,
            "{profile:?} stale before shootdown"
        );

        m.bus.write32(0x9008, 1).unwrap();
        for _ in 0..1000 {
            let event = m.step_scheduled().unwrap();
            if let helios::cpu::Step::Executed { pc, .. } = event.step {
                if event.processor == 0 && pc == 0x7044 {
                    saw_ipi = true;
                }
            }
            if m.bus.read32(0x9004).unwrap() != 0 {
                break;
            }
        }
        assert!(saw_ipi, "{profile:?}");
        assert_eq!(m.bus.read32(0x9004).unwrap(), 1, "{profile:?}");
        for _ in 0..20 {
            m.step_scheduled().unwrap();
        }
        assert!(
            m.secondary_cpu().unwrap().cpu.read_reg(4) > 0,
            "{profile:?}"
        );
        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        assert_eq!(
            secondary
                .cpu
                .mmu
                .load(bus, 0x5028, 11, Width::Word)
                .unwrap(),
            new_page[0],
            "{profile:?} fresh after shootdown"
        );
    }
}

#[test]
fn guest_reset_store_clears_standalone_iommu_translation() {
    let reset_program = [
        0x033c_7c00u32, // sethi %hi(0xf1f00000), %g1
        0x8410_2001,    // or %g0, 1, %g2
        0xc4a0_45e0,    // sta %g2, [%g1] 0x2f (system reset)
        0x1080_0000,
        0x0100_0000,
    ];
    let bytes =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_be_bytes()).collect() };
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        warm_standalone_iommu(&mut m);
        m.load_image(0x2000, &bytes(&reset_program)).unwrap();
        assert!(m.set_processor_entry(0, 0x2000, 0x2004));
        assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1111_1111));
        let mut reset = None;
        for _ in 0..20 {
            let event = m.step_scheduled().unwrap();
            if let helios::cpu::Step::SoftwareReset { instruction, .. } = event.step {
                reset = Some(instruction);
                break;
            }
        }
        assert_eq!(reset, Some(Some(0xc4a0_45e0)), "{profile:?}");
        m.bus.write32(0x10000, 0x906).unwrap();
        let io = m.bus.layout().io_control;
        m.bus.write32(io, 1).unwrap();
        m.bus.write32(io + 4, 0x1000).unwrap();
        m.bus.write32(io + 0x1008, 1 << 20).unwrap();
        assert_eq!(
            m.dma_read(5, 0xff00_0000, Width::Word),
            Ok(0x2222_2222),
            "{profile:?}"
        );
    }
}

#[test]
fn equal_virtual_time_advances_retrace_like_up_execution() {
    use helios::devices::cgsix::Cgsix;
    fn machine(profile: MachineProfile, processors: usize) -> Machine {
        let mut m = Machine::with_profile_processors(
            profile,
            16 * 1024 * 1024,
            &loop_image(true),
            profile.default_nvram(),
            processors,
        )
        .unwrap();
        m.load_image(0x1000, &loop_image(true)).unwrap();
        if processors == 2 {
            m.load_image(0x2000, &loop_image(false)).unwrap();
            assert!(m.set_processor_entry(2, 0x2000, 0x2004));
            assert!(m.set_processor_arbitration(2, true));
        }
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        m.install_cgsix(Cgsix::new(&vec![0xff; 32768]).unwrap())
            .unwrap();
        let base = m.bus.layout().cgsix;
        m.bus.write32(base + 0x301818, 0x420).unwrap();
        m
    }
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut single = machine(profile, 1);
        let mut both = machine(profile, 2);
        for _ in 0..150_000 {
            single.step_scheduled().unwrap();
        }
        for _ in 0..300_000 {
            both.step_scheduled().unwrap();
        }
        assert_eq!(single.scheduler_stats().virtual_ns, 15_000_000);
        assert_eq!(both.scheduler_stats().virtual_ns, 15_000_000);
        let base = single.bus.layout().cgsix;
        assert_eq!(
            single.bus.read32(base + 0x301818).unwrap(),
            both.bus.read32(base + 0x301818).unwrap(),
            "{profile:?}"
        );
    }
}

#[test]
fn secondary_synchronous_trap_is_processor_tagged() {
    let secondary = [
        0x8188_20a0u32, // wr %g0, 0xa0, %psr (ET=1, S=1)
        0x0100_0000,
        0x0100_0000,
        0x0000_0000, // illegal instruction -> trap type 2
    ];
    let handler = [0x1080_0000u32, 0x0100_0000];
    let bytes =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_be_bytes()).collect() };
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = dual(profile);
        m.load_image(0x1000, &loop_image(true)).unwrap();
        m.load_image(0x2000, &bytes(&secondary)).unwrap();
        m.load_image(0x20, &bytes(&handler)).unwrap();
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        assert!(m.set_processor_arbitration(2, true));
        let mut trap = None;
        for _ in 0..20 {
            let event = m.step_scheduled().unwrap();
            if let helios::cpu::Step::TrapTaken { trap_type, .. } = event.step {
                if event.processor == 2 {
                    trap = Some(trap_type);
                    break;
                }
            }
        }
        assert_eq!(trap, Some(2), "{profile:?}");
        assert!(m.cpu.error_state().is_none(), "{profile:?}");
        assert_eq!(m.scheduler_stats().processor2.traps, 1, "{profile:?}");
    }
}

#[test]
fn scc_state_and_output_are_shared_identically_with_one_or_two_cpus() {
    fn snapshot(m: &mut Machine) -> (Vec<u8>, Vec<u8>) {
        let serial = &mut m.bus.devices_mut().unwrap().serial;
        let registers: Vec<u8> = (0..16).map(|reg| serial.read(reg).unwrap_or(0)).collect();
        let mut output = serial.take_output(Channel::A);
        output.extend(serial.take_output(Channel::B));
        (registers, output)
    }
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut single = dual(profile);
        single.load_image(0x1000, &loop_image(true)).unwrap();
        assert!(single.set_processor_entry(0, 0x1000, 0x1004));
        for _ in 0..1000 {
            single.step_scheduled().unwrap();
        }
        let mut both = dual(profile);
        both.load_image(0x1000, &loop_image(true)).unwrap();
        both.load_image(0x2000, &loop_image(false)).unwrap();
        assert!(both.set_processor_entry(0, 0x1000, 0x1004));
        assert!(both.set_processor_entry(2, 0x2000, 0x2004));
        assert!(both.set_processor_arbitration(2, true));
        for _ in 0..2000 {
            both.step_scheduled().unwrap();
        }
        assert_eq!(single.scheduler_stats().virtual_ns, 100_000);
        assert_eq!(both.scheduler_stats().virtual_ns, 100_000);
        assert_eq!(snapshot(&mut single), snapshot(&mut both), "{profile:?}");
    }
}
