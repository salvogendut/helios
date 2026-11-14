use helios::bus::Bus;
use helios::devices::interrupt::{
    InterruptController, Source, TargetPolicy, MASK_ALL, MODULE_ERROR,
};
use helios::devices::timer::Timers;
use helios::machine::Machine;

#[test]
fn directed_software_bits_and_clears_are_per_processor_bank() {
    let mut ic = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(ic.install_processor(2));
    ic.write_system(8, u32::MAX).unwrap();

    ic.write_cpu(0, 8, 1 << 17).unwrap();
    ic.write_cpu(2, 8, 1 << 18).unwrap();
    assert_eq!(ic.levels_for(0), 1 << 1);
    assert_eq!(ic.levels_for(2), 1 << 2);
    assert_eq!(ic.read_cpu(0, 0).unwrap(), 1 << 17);
    assert_eq!(ic.read_cpu(2, 0).unwrap(), 1 << 18);

    ic.write_cpu(0, 4, 1 << 17).unwrap();
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 1 << 2);
    assert_eq!(ic.read_cpu(2, 0).unwrap(), 1 << 18);

    assert_eq!(ic.levels_for(1), 0);
    ic.write_cpu(1, 8, 1 << 19).unwrap();
    assert_eq!(ic.levels_for(1), 0);
    assert_eq!(ic.read_cpu(1, 0).unwrap(), 0);
}

#[test]
fn current_interrupt_target_routes_shared_sources_and_accepts_absent_targets() {
    let mut ic = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(ic.install_processor(2));
    ic.write_system(8, u32::MAX).unwrap();
    ic.set_source(Source::Scsi, true); // level 4

    assert_eq!(ic.levels_for(0), 1 << 4);
    assert_eq!(ic.levels_for(2), 0);
    assert_eq!(ic.read_cpu(0, 0).unwrap(), 1 << 4);
    assert_eq!(ic.read_cpu(2, 0).unwrap(), 0);

    ic.write_system(12, 1 << 18).unwrap();
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.read_system(0).unwrap(), 1 << 18);
    ic.write_system(8, 1 << 18).unwrap();
    assert_eq!(ic.levels_for(0), 1 << 4);

    ic.set_target(2);
    assert_eq!(ic.target(), 2);
    assert_eq!(ic.read_system(16).unwrap(), 2);
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 1 << 4);
    assert_eq!(ic.read_cpu(0, 0).unwrap(), 0);
    assert_eq!(ic.read_cpu(2, 0).unwrap(), 1 << 4);

    ic.set_target(1); // absent hardware processor bank
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 0);
    assert_eq!(ic.read_system(0).unwrap(), 1 << 18);
    ic.set_target(0);
    assert_eq!(ic.levels_for(0), 1 << 4);

    ic.set_source(Source::Scsi, false);
    assert_eq!(ic.levels_for(0), 0);
}

#[test]
fn level15_capture_and_acknowledgement_are_separate_from_live_source_status() {
    let mut ic = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(ic.install_processor(2));
    ic.write_system(8, u32::MAX).unwrap();
    ic.set_source(Source::ModuleError, true);

    assert_eq!(ic.levels_for(0), 1 << 15);
    assert_eq!(ic.levels_for(2), 1 << 15);
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 1 << 30);

    ic.write_cpu(0, 4, 1 << 15).unwrap();
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 1 << 15);
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 1 << 30);

    ic.write_cpu(2, 4, 1 << 15).unwrap();
    assert_eq!(ic.levels_for(2), 0);
    // Last acknowledgement does not fabricate deassertion of a held source.
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 1 << 30);

    ic.set_source(Source::ModuleError, false);
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 0);
}

#[test]
fn masked_level15_event_is_not_captured_retroactively() {
    let mut ic = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(ic.install_processor(2));
    // Enable delivery globally, leaving only the source mask to prevent capture.
    ic.write_system(8, MASK_ALL).unwrap();
    ic.set_source(Source::ModuleError, true);
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 1 << 30);
    assert_eq!(ic.read_cpu(0, 0).unwrap() & (1 << 15), 0);
    assert_eq!(ic.read_cpu(2, 0).unwrap() & (1 << 15), 0);
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 0);
    ic.write_system(8, MODULE_ERROR).unwrap();
    assert_eq!(ic.read_cpu(0, 0).unwrap() & (1 << 15), 0);
    assert_eq!(ic.read_cpu(2, 0).unwrap() & (1 << 15), 0);
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 0);

    ic.set_source(Source::ModuleError, false);
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 0);
}

#[test]
fn mbus_bus_errors_use_a_held_write_buffer_source_and_keep_module_error_independent() {
    use helios::bus::{BusError, BusErrorKind};
    use helios::machine::MachineProfile;

    const WRITE_BUFFER: u32 = 1 << 29;
    const HARD15: u32 = 1 << 15;
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = Machine::with_profile(
            profile,
            16 * 1024 * 1024,
            &0x0100_0000u32.to_be_bytes(),
            profile.default_nvram(),
        )
        .unwrap();
        assert!(m.install_processor_banks(2));
        let io = m.bus.layout().io_control;
        let irq = m.bus.system_interrupt_base();
        let cpu0 = m.bus.processor_interrupt_base(0);
        let cpu2 = m.bus.processor_interrupt_base(2);
        let pulse = |m: &mut Machine| {
            assert!(m.bus.capture_sbus_write_error(
                BusError {
                    address: 0xff6000000,
                    kind: BusErrorKind::SBusTimeout,
                },
                4,
            ));
        };
        m.bus.write32(irq + 8, u32::MAX).unwrap();
        pulse(&mut m);
        assert_eq!(m.bus.read32(irq).unwrap() & WRITE_BUFFER, WRITE_BUFFER);
        assert_eq!(m.bus.read32(irq).unwrap() & MODULE_ERROR, 0);
        assert_eq!(m.bus.read32(cpu0).unwrap() & HARD15, HARD15);
        assert_eq!(m.bus.read32(cpu2).unwrap() & HARD15, HARD15);

        // CPU acknowledgement is per bank and leaves the held source.
        m.bus.write32(cpu0 + 4, HARD15).unwrap();
        assert_eq!(m.bus.read32(cpu0).unwrap() & HARD15, 0);
        assert_eq!(m.bus.read32(cpu2).unwrap() & HARD15, HARD15);
        assert_eq!(m.bus.read32(irq).unwrap() & WRITE_BUFFER, WRITE_BUFFER);

        // AFSR acknowledgement releases the source but not CPU 2's capture.
        m.bus.write32(io + 0x1000, 0).unwrap();
        assert_eq!(m.bus.read32(irq).unwrap() & WRITE_BUFFER, 0);
        assert_eq!(m.bus.read32(cpu2).unwrap() & HARD15, HARD15);
        m.bus.write32(cpu2 + 4, HARD15).unwrap();
        assert_eq!(m.bus.read32(cpu2).unwrap() & HARD15, 0);

        // An independently asserted ModuleError is a separate level-15 source.
        m.bus.write32(irq + 8, u32::MAX).unwrap();
        m.bus
            .devices_mut()
            .unwrap()
            .interrupts
            .set_source(Source::ModuleError, true);
        assert_eq!(m.bus.read32(irq).unwrap() & MODULE_ERROR, MODULE_ERROR);
        assert_eq!(m.bus.read32(cpu0).unwrap() & HARD15, HARD15);
        assert_eq!(m.bus.read32(cpu2).unwrap() & HARD15, HARD15);
        m.bus.write32(cpu0 + 4, HARD15).unwrap();
        m.bus.write32(cpu2 + 4, HARD15).unwrap();
        assert_eq!(m.bus.read32(irq).unwrap() & MODULE_ERROR, MODULE_ERROR);
        m.bus
            .devices_mut()
            .unwrap()
            .interrupts
            .set_source(Source::ModuleError, false);
        assert_eq!(m.bus.read32(irq).unwrap() & MODULE_ERROR, 0);
    }
}

#[test]
fn source_deassert_and_mask_write_preserve_other_processor_latches() {
    let mut ic = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(ic.install_processor(2));
    ic.write_system(8, u32::MAX).unwrap();
    ic.set_source(Source::ModuleError, true);
    ic.write_cpu(0, 4, 1 << 15).unwrap();
    assert_eq!(ic.levels_for(2), 1 << 15);

    ic.write_system(12, MODULE_ERROR).unwrap();
    assert_eq!(ic.levels_for(2), 1 << 15);
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 1 << 30);

    ic.set_source(Source::ModuleError, false);
    assert_eq!(ic.levels_for(0), 0);
    assert_eq!(ic.levels_for(2), 1 << 15);
    assert_eq!(ic.read_system(0).unwrap() & (1 << 30), 0);
}

#[test]
fn mask_all_gates_software_and_timer_delivery_but_not_pending_state() {
    let mut ic = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(ic.install_processor(2));
    ic.write_system(8, MASK_ALL).unwrap();
    ic.write_cpu(2, 8, 1 << 17).unwrap();
    ic.set_processor_timer(2, true);
    assert_eq!(ic.levels_for(2), (1 << 1) | (1 << 14));
    ic.write_system(12, MASK_ALL).unwrap();
    assert_eq!(ic.levels_for(2), 0);
    assert_eq!(ic.read_cpu(2, 0).unwrap(), (1 << 17) | (1 << 14));
    ic.write_system(8, MASK_ALL).unwrap();
    assert_eq!(ic.levels_for(2), (1 << 1) | (1 << 14));
}

#[test]
fn timer_banks_share_time_but_have_independent_limits_acknowledgements_and_modes() {
    let mut timers = Timers::default();
    assert!(timers.install_processor(2));
    timers.install_processor(2);

    timers.write_cpu(0, 0, 2 << 9).unwrap();
    timers.write_cpu(2, 0, 3 << 9).unwrap();
    timers.advance_ns(500);
    assert!(timers.processor_irq_for(0));
    assert!(!timers.processor_irq_for(2));
    timers.advance_ns(500);
    assert!(timers.processor_irq_for(2));

    assert_eq!(timers.read_cpu(0, 0).unwrap() & (1 << 31), 1 << 31);
    assert!(!timers.processor_irq_for(0));
    assert!(timers.processor_irq_for(2));

    timers.write_system(16, (1 << 0) | (1 << 2)).unwrap();
    assert_eq!(timers.read_system(16).unwrap(), (1 << 0) | (1 << 2));
    assert!(!timers.processor_irq_for(2));
    timers.write_cpu(0, 12, 1).unwrap();
    timers.write_cpu(2, 12, 1).unwrap();
    timers.advance_ns(500);
    assert_eq!(timers.read_user_doubleword_for(0).unwrap(), [0, 0x200]);
    assert_eq!(timers.read_user_doubleword_for(2).unwrap(), [0, 0x200]);
    timers.write_cpu(2, 12, 0).unwrap();
    timers.advance_ns(500);
    assert_eq!(timers.read_user_doubleword_for(0).unwrap(), [0, 0x400]);
    assert_eq!(timers.read_user_doubleword_for(2).unwrap(), [0, 0x200]);
}

#[test]
fn bus_exposes_hardware_processor_banks_and_keeps_absent_banks_zero() {
    let mut m = Machine::with_profile(
        helios::machine::MachineProfile::Ss20,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        helios::machine::MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    assert!(m.install_processor_banks(2));
    assert!(!m.install_processor_banks(2));

    let timer0 = m.bus.processor_timer_base(0);
    let timer2 = m.bus.processor_timer_base(2);
    let interrupt0 = m.bus.processor_interrupt_base(0);
    let interrupt2 = m.bus.processor_interrupt_base(2);
    let system_interrupt = m.bus.system_interrupt_base();
    assert_eq!(timer2, timer0 + 0x2000);
    assert_eq!(interrupt2, interrupt0 + 0x2000);

    m.bus.write32(timer2, 2 << 9).unwrap();
    m.advance_ns(500);
    assert!(m.bus.devices().unwrap().timers.processor_irq_for(2));
    assert_eq!(m.bus.interrupt_levels_for(2), 0); // level 14 is masked at reset
    m.bus.write32(interrupt2 + 8, 1 << 17).unwrap();
    m.bus.write32(system_interrupt + 8, MASK_ALL).unwrap();
    assert_eq!(m.bus.interrupt_levels_for(2), (1 << 1) | (1 << 14));

    m.bus.write32(system_interrupt + 0x10, 2).unwrap();
    assert_eq!(m.bus.read32(system_interrupt + 0x10).unwrap(), 2);
    m.bus.write32(system_interrupt + 8, 1 << 18).unwrap();
    let devices = m.bus.devices_mut().unwrap();
    devices.interrupts.set_source(Source::Scsi, true);
    assert_eq!(m.bus.interrupt_levels_for(0), 0);
    assert_eq!(
        m.bus.interrupt_levels_for(2),
        (1 << 1) | (1 << 4) | (1 << 14)
    );

    assert_eq!(m.bus.read32(timer0 + 0x1000).unwrap(), 0);
    assert_eq!(m.bus.read32(interrupt0 + 0x1000).unwrap(), 0);
    assert_eq!(m.bus.read32(timer0 + 0x3000).unwrap(), 0);
    assert_eq!(m.bus.read32(interrupt0 + 0x3000).unwrap(), 0);
}

#[test]
fn bus_user_timer_doubleword_is_available_on_processor_two_bank() {
    let mut m = Machine::with_profile(
        helios::machine::MachineProfile::Ss20,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        helios::machine::MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    assert!(m.install_processor_banks(2));
    let timer0 = m.bus.processor_timer_base(0);
    let timer2 = m.bus.processor_timer_base(2);
    let system_timer = m.bus.system_timer_base();
    m.bus.write32(system_timer + 0x10, 1 << 2).unwrap();
    m.bus
        .write_doubleword(timer2, [0x1234_5678, 0x7fff_ffff])
        .unwrap();
    assert_eq!(
        m.bus.read_doubleword(timer2).unwrap(),
        [0x1234_5678, 0x7fff_fe00]
    );
    assert_eq!(
        m.bus.read_doubleword(timer0).unwrap_err().kind,
        helios::bus::BusErrorKind::Unsupported("timer doubleword requires processor user mode")
    );
}

#[test]
fn target_register_policy_is_profile_specific() {
    let mut ncr = InterruptController::with_policy(TargetPolicy::Ncr);
    ncr.write_system(8, u32::MAX).unwrap();
    ncr.write_system(16, 2).unwrap();
    assert_eq!(ncr.read_system(16).unwrap(), 0);
    ncr.set_target(2);
    assert_eq!(ncr.target(), 0);
    ncr.set_source(Source::Serial, true);
    assert_eq!(ncr.levels_for(0), 1 << 12);

    let mut mbus = InterruptController::with_policy(TargetPolicy::Mbus);
    assert!(mbus.install_processor(2));
    mbus.write_system(8, u32::MAX).unwrap();
    mbus.write_system(16, 0xffff_fffe).unwrap();
    assert_eq!(mbus.read_system(16).unwrap(), 2);
    mbus.set_source(Source::Scsi, true);
    assert_eq!(mbus.levels_for(0), 0);
    assert_eq!(mbus.levels_for(2), 1 << 4);
}

#[test]
fn invalid_bank_ids_are_rejected_without_state_changes() {
    let mut ncr = InterruptController::with_policy(TargetPolicy::Ncr);
    let ncr_before = ncr.clone();
    assert!(!ncr.install_processor(2));
    assert!(!ncr.install_processor(16));
    assert!(!ncr.install_processor(32));
    assert_eq!(ncr, ncr_before);
    let mut mbus = InterruptController::with_policy(TargetPolicy::Mbus);
    let mut timers = Timers::default();
    assert!(mbus.install_processor(3));
    assert!(timers.install_processor(3));
    let mbus_before = mbus.clone();
    let timers_before = timers.clone();
    for processor in [4, 15, 16, 32, 255] {
        assert!(!mbus.install_processor(processor));
        assert!(!timers.install_processor(processor));
        assert_eq!(mbus, mbus_before);
        assert_eq!(timers, timers_before);
    }

    let mut m = Machine::with_profile(
        helios::machine::MachineProfile::Ss5,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        helios::machine::MachineProfile::Ss5.default_nvram(),
    )
    .unwrap();
    assert!(!m.install_processor_banks(2));
    assert!(!m.bus.devices().unwrap().timers.contains_processor(2));
    assert!(!m.bus.devices().unwrap().interrupts.contains_processor(2));

    let mut m = Machine::with_profile(
        helios::machine::MachineProfile::Ss20,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        helios::machine::MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    let timers_before = m.bus.devices().unwrap().timers.clone();
    let interrupts_before = m.bus.devices().unwrap().interrupts.clone();
    for processor in [1, 3, 4, 15, 16, 32, 255] {
        assert!(!m.install_processor_banks(processor));
        assert_eq!(m.bus.devices().unwrap().timers, timers_before);
        assert_eq!(m.bus.devices().unwrap().interrupts, interrupts_before);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        m.bus.write32(0xff1310010, 0).unwrap();
    }));
    assert!(result.is_ok());
}

#[test]
fn duplicate_bank_installation_is_idempotent_and_reset_keeps_population() {
    let mut m = Machine::with_profile(
        helios::machine::MachineProfile::Ss20,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        helios::machine::MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    assert!(m.install_processor_banks(2));
    let timer2 = m.bus.processor_timer_base(2);
    m.bus.write32(timer2, 2 << 9).unwrap();
    m.advance_ns(500);
    assert!(m.bus.devices().unwrap().timers.processor_irq_for(2));
    let timers_before = m.bus.devices().unwrap().timers.clone();
    let interrupts_before = m.bus.devices().unwrap().interrupts.clone();
    assert!(!m.install_processor_banks(2));
    assert_eq!(m.bus.devices().unwrap().timers, timers_before);
    assert_eq!(m.bus.devices().unwrap().interrupts, interrupts_before);
    m.reset();
    assert!(m.bus.devices().unwrap().timers.contains_processor(2));
    assert!(m.bus.devices().unwrap().interrupts.contains_processor(2));
}
