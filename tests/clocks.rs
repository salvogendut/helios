use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{CpuClockMode, InstalledVideo, Machine, MachineProfile};
use helios::mmu::Width;

const LOOP: [u8; 8] = [0x10, 0x80, 0, 0, 1, 0, 0, 0]; // ba .; nop

#[test]
fn fast_default_and_clock_selection_reset_both_cpus_and_preserve_ram() {
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        for cpus in 1..=if profile == MachineProfile::Ss5 { 1 } else { 2 } {
            let mut m = Machine::with_profile_processors_video(
                profile,
                16 * 1024 * 1024,
                &LOOP,
                profile.default_nvram(),
                cpus,
                InstalledVideo::None,
            )
            .unwrap();
            assert_eq!(m.clock_mode(), CpuClockMode::Fast);
            assert_eq!(m.cpu_clock_hz(), 10_000_000);
            m.load_image(0x1000, &LOOP).unwrap();
            m.bus.write32(0x3000, 0x1234_abcd).unwrap();
            for mode in [
                CpuClockMode::Fast,
                CpuClockMode::Hardware,
                CpuClockMode::Fast,
            ] {
                m.reset_with_clock_mode(mode);
                let hz = if mode == CpuClockMode::Fast {
                    10_000_000
                } else {
                    profile.cpu_clock_hz()
                };
                assert_eq!(m.cpu_clock_hz(), hz);
                assert_eq!(m.clock_mode(), mode);
                assert_eq!(m.scheduler_stats().virtual_ns, 0);
                assert_eq!(m.bus.read32(0x3000).unwrap(), 0x1234_abcd);
                m.set_processor_entry(0, 0x1000, 0x1004);
                if cpus == 2 {
                    m.set_processor_entry(2, 0x1000, 0x1004);
                    m.set_processor_arbitration(2, true);
                }
                m.run_scheduled(1500 * cpus as u64).unwrap();
                assert_eq!(m.scheduler_stats().virtual_ns, 1500 * 1_000_000_000 / hz);
                m.reset();
                assert_eq!(m.clock_mode(), mode);
                assert_eq!(m.cpu_clock_hz(), hz);
            }
        }
    }
}

fn machine(profile: MachineProfile, cpus: usize) -> Machine {
    let mut m = Machine::with_profile_processors_video(
        profile,
        16 * 1024 * 1024,
        &LOOP,
        profile.default_nvram(),
        cpus,
        InstalledVideo::None,
    )
    .unwrap();
    m.reset_with_clock_mode(CpuClockMode::Hardware);
    m.load_image(0x1000, &LOOP).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    if cpus == 2 {
        assert!(m.set_processor_entry(2, 0x1000, 0x1004));
        assert!(m.set_processor_arbitration(2, true));
    }
    m
}

#[test]
fn named_profiles_retain_fractional_cycles_and_match_the_single_cpu_path() {
    for (profile, mhz) in [
        (MachineProfile::Ss5, 85),
        (MachineProfile::Ss20, 75),
        (MachineProfile::Axil320, 75),
    ] {
        let mut scheduled = machine(profile, 1);
        let mut direct = machine(profile, 1);
        assert_eq!(scheduled.cpu_clock_hz(), mhz * 1_000_000);
        for cycles in 1..=mhz * 20 {
            scheduled.step_scheduled().unwrap();
            direct.step().unwrap();
            let ns = cycles * 1000 / mhz;
            assert_eq!(scheduled.scheduler_stats().virtual_ns, ns);
            assert_eq!(
                direct
                    .bus
                    .read32(direct.bus.system_timer_base() + 4)
                    .unwrap(),
                (ns / 500) as u32 * 512,
                "{profile:?}, {cycles} cycles"
            );
            assert_eq!(
                scheduled
                    .bus
                    .read32(scheduled.bus.system_timer_base() + 4)
                    .unwrap(),
                (ns / 500) as u32 * 512,
            );
        }
    }
}

#[test]
fn each_cpu_keeps_its_own_fractional_phase_and_shared_time_is_not_doubled() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = machine(profile, 2);
        for cycles in 0..1500 {
            let a = m.step_scheduled().unwrap();
            assert_eq!(a.processor, 0);
            assert_eq!(a.virtual_ns, cycles * 1000 / 75);
            let b = m.step_scheduled().unwrap();
            assert_eq!(b.processor, 2);
            assert_eq!(b.virtual_ns, (cycles + 1) * 1000 / 75);
        }
        assert_eq!(m.scheduler_stats().virtual_ns, 20_000);
    }
}

#[test]
fn host_stop_does_not_consume_a_fractional_cycle_and_retry_has_the_same_deadline() {
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        let mut m = machine(profile, 1);
        let load: u32 = if profile == MachineProfile::Ss5 {
            0xc488_4400 // lduba [g1] ASI 0x20,g2
        } else {
            0xc488_45e0 // lduba [g1] ASI 0x2f,g2
        };
        m.load_image(0x2000, &load.to_be_bytes()).unwrap();
        m.step_scheduled().unwrap();
        let time = m.scheduler_stats().virtual_ns;
        assert!(m.set_processor_entry(0, 0x2000, 0x2004));
        // The timer aperture supports words only. An unsupported byte load
        // is a retryable host stop, with no guest trap or time advancement.
        let timer = m.bus.system_timer_base() as u32;
        m.cpu.write_reg(1, timer);
        for _ in 0..3 {
            assert!(m.step_scheduled().is_err());
            assert_eq!(m.scheduler_stats().virtual_ns, time);
            assert!(m.cpu.error_state().is_none());
        }
        m.cpu.write_reg(1, 0x3000);
        // Switch the failing high bypass instruction to ASI 0x20 for RAM.
        m.load_image(0x2000, &0xc488_4400u32.to_be_bytes()).unwrap();
        m.step_scheduled().unwrap();
        let expected = if profile == MachineProfile::Ss5 {
            23
        } else {
            26
        };
        assert_eq!(m.scheduler_stats().virtual_ns, expected);
    }
}

#[test]
fn module_reset_resets_only_its_fraction_while_board_reset_resets_both() {
    let mut m = machine(MachineProfile::Ss20, 2);
    m.run_scheduled(4).unwrap(); // both have used two 13 ns cycles
    assert_eq!(m.scheduler_stats().virtual_ns, 26);
    assert!(m.reset_processor_module(0));
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    assert_eq!(m.step_scheduled().unwrap().virtual_ns, 26);
    assert_eq!(m.step_scheduled().unwrap().virtual_ns, 39);
    // CPU 2 still carries its third-cycle fraction: ready at 40 ns.
    assert_eq!(m.step_scheduled().unwrap().processor, 0);
    assert_eq!(m.scheduler_stats().virtual_ns, 40);

    m.reset();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    assert!(m.set_processor_entry(2, 0x1000, 0x1004));
    assert!(m.set_processor_arbitration(2, true));
    m.run_scheduled(4).unwrap();
    assert_eq!(m.scheduler_stats().virtual_ns, 26);
}

#[test]
fn software_reset_clears_fractional_phases_in_both_execution_paths() {
    for scheduled in [false, true] {
        let mut m = machine(MachineProfile::Axil320, 1);
        for _ in 0..2 {
            if scheduled {
                m.step_scheduled().unwrap();
            } else {
                m.step().unwrap();
            }
        }
        m.bus.write32(m.bus.layout().reset, 1).unwrap();
        let reset = if scheduled {
            m.step_scheduled().unwrap().step
        } else {
            m.step().unwrap()
        };
        assert!(matches!(
            reset,
            Step::SoftwareReset {
                instruction: None,
                ..
            }
        ));
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        // Start just below a timer boundary to expose a stale 14 ns cycle.
        m.advance_ns(486);
        if scheduled {
            m.step_scheduled().unwrap();
        } else {
            m.step().unwrap();
        }
        assert_eq!(m.bus.read32(m.bus.system_timer_base() + 4).unwrap(), 0);
    }
}

#[test]
fn physical_prom_wait_adds_to_the_cpu_clock_with_its_own_fractional_carry() {
    let mut m = Machine::ss5(8 * 1024 * 1024, &LOOP).unwrap();
    m.reset_with_clock_mode(CpuClockMode::Hardware);
    for cycles in 1..=85 {
        m.step_scheduled().unwrap();
        // 24 SBus cycles per word at 21.25 MHz plus one 85 MHz CPU cycle.
        let expected = cycles * 24 * 1_000_000_000 / 21_250_000 + cycles * 1000 / 85;
        assert_eq!(m.scheduler_stats().virtual_ns, expected);
    }
    assert_eq!(m.scheduler_stats().virtual_ns, 97_000);
}

#[test]
fn legacy_diagnostic_harness_keeps_its_explicit_100ns_quantum() {
    let mut m = Machine::sun4m(8192, &LOOP).unwrap();
    m.load_image(0x1000, &LOOP).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    m.run_scheduled(75).unwrap();
    assert_eq!(m.cpu_clock_hz(), 10_000_000);
    assert_eq!(m.scheduler_stats().virtual_ns, 7500);
}

#[test]
fn ss5_speed_pins_are_read_only_and_survive_both_reset_scopes() {
    let mut m = machine(MachineProfile::Ss5, 1);
    for value in [0, u32::MAX, 0x00e0_0000] {
        m.cpu
            .mmu
            .store(&mut m.bus, 0x1000, 4, Width::Word, value)
            .unwrap();
        let actual = m.cpu.mmu.load(&mut m.bus, 0x1000, 4, Width::Word).unwrap();
        assert_eq!(actual & 0x01e0_0000, 0x0120_0000);
        assert_eq!((actual >> 23) & 3, 2); // divide by four
        assert_eq!((actual >> 21) & 3, 1); // 85 MHz memory timing pins
    }
    for software in [true, false] {
        if software {
            m.bus.write32(m.bus.layout().reset, 1).unwrap();
            assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
        } else {
            m.reset();
        }
        let actual = m.cpu.mmu.load(&mut m.bus, 0x1000, 4, Width::Word).unwrap();
        assert_eq!(actual & 0x01e0_0000, 0x0120_0000);
    }
}
