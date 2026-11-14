use helios::{
    bus::{Bus, BusErrorKind},
    machine::{Machine, MachineProfile, ProcessorModuleState},
    mmu::{MemoryError, Width},
    system_bus::SystemBus,
    topology::ProcessorModule,
};
use std::process::Command;
use std::time::{Duration, Instant};

fn machine_for(profile: MachineProfile) -> Machine {
    Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &0u32.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap()
}

fn machine() -> Machine {
    machine_for(MachineProfile::Ss20)
}

fn module(mid: u8) -> ProcessorModule {
    ProcessorModule::from_mid(mid).unwrap()
}

fn module_pair(profile: MachineProfile) -> (ProcessorModuleState, ProcessorModuleState, SystemBus) {
    let low = ProcessorModuleState::new(profile, module(8)).unwrap();
    let high = ProcessorModuleState::new(profile, module(10)).unwrap();
    let mut bus = SystemBus::with_layout(profile.layout());
    bus.map_ram(0, 0x1000).unwrap();
    bus.install_mxcc_module(low.mxcc.clone());
    bus.install_mxcc_module(high.mxcc.clone());
    (low, high, bus)
}

#[test]
fn processor_stream_read_of_module_control_never_panics_the_host() {
    let mut m = machine();
    // Either modeled completion or an explicit error is acceptable; host
    // unwinding is not.
    let _ = m
        .cpu
        .mmu
        .store_doubleword(&mut m.bus, 0x01c0_0100, 2, [0xf, 0xf8c0_0a00]);
}

#[test]
fn physical_stream_write_to_module_control_never_panics_the_host() {
    let mut m = machine();
    let _ = m.bus.write_doubleword(0x000f_f8c0_0200, [0xf, 0xf8c0_0a00]);
}

#[test]
fn module_port_identity_ignores_guest_writes() {
    let mut m = machine();
    let before = m
        .cpu
        .mmu
        .load_doubleword(&mut m.bus, 0x01c0_0f00, 2)
        .unwrap();
    m.cpu
        .mmu
        .store_doubleword(&mut m.bus, 0x01c0_0f00, 2, [0xffff_ffff, 0xffff_ffff])
        .unwrap();
    let after = m
        .cpu
        .mmu
        .load_doubleword(&mut m.bus, 0x01c0_0f00, 2)
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(before, [0, 0x0800_0104]);
}

#[test]
fn populated_module_has_the_architected_mbus_port_address() {
    let mut m = machine();
    assert_eq!(m.bus.read32(0x000f_f8ff_fffc).unwrap(), 0x0800_0104);
}

#[test]
fn word_stream_source_from_ram_completes_on_both_modules() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for mid in [8u8, 10] {
            let (mut low, mut high, mut bus) = module_pair(profile);
            let (target, other) = if mid == 8 {
                (&mut low, &mut high)
            } else {
                (&mut high, &mut low)
            };
            let other_before = other.mxcc.borrow().clone();
            for index in 0..8u32 {
                bus.write32(0x100 + u64::from(index) * 4, 0x1234_0000 + index)
                    .unwrap();
            }
            // Low word of the ASI 2 source register; the high word stays zero.
            target
                .cpu
                .mmu
                .store(&mut bus, 0x01c0_0104, 2, Width::Word, 0x100)
                .unwrap();
            assert_eq!(
                target
                    .cpu
                    .mmu
                    .load_doubleword(&mut bus, 0x01c0_0000, 2)
                    .unwrap(),
                [0x1234_0000, 0x1234_0001]
            );
            assert_eq!(*other.mxcc.borrow(), other_before);
        }
    }
}

#[test]
fn word_stream_destination_to_ram_completes_on_both_modules() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for mid in [8u8, 10] {
            let (mut low, mut high, mut bus) = module_pair(profile);
            let (target, other) = if mid == 8 {
                (&mut low, &mut high)
            } else {
                (&mut high, &mut low)
            };
            let other_before = other.mxcc.borrow().clone();
            target
                .cpu
                .mmu
                .store_doubleword(&mut bus, 0x01c0_0000, 2, [0x1234_5678, 0x9abc_def0])
                .unwrap();
            target
                .cpu
                .mmu
                .store(&mut bus, 0x01c0_0204, 2, Width::Word, 0x200)
                .unwrap();
            assert_eq!(
                bus.read_doubleword(0x200).unwrap(),
                [0x1234_5678, 0x9abc_def0]
            );
            assert_eq!(*other.mxcc.borrow(), other_before);
        }
    }
}

#[test]
fn word_stream_failure_keeps_completion_clear_and_data_unchanged() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for mid in [8u8, 10] {
            let (mut low, mut high, mut bus) = module_pair(profile);
            let target = if mid == 8 { &mut low } else { &mut high };
            target
                .cpu
                .mmu
                .store_doubleword(&mut bus, 0x01c0_0000, 2, [0x1122_3344, 0x5566_7788])
                .unwrap();
            let data_before = target
                .cpu
                .mmu
                .load_doubleword(&mut bus, 0x01c0_0000, 2)
                .unwrap();

            // Unmapped RAM source: the address is retained, RDY stays clear
            // and the data bank is not replaced.
            let source = target
                .cpu
                .mmu
                .store(&mut bus, 0x01c0_0104, 2, Width::Word, 0x40000);
            assert!(matches!(
                source,
                Err(MemoryError::Bus(error)) if error.kind == BusErrorKind::Unmapped
            ));
            assert_eq!(
                target
                    .cpu
                    .mmu
                    .load_doubleword(&mut bus, 0x01c0_0100, 2)
                    .unwrap(),
                [0, 0x40000]
            );
            assert_eq!(
                target
                    .cpu
                    .mmu
                    .load_doubleword(&mut bus, 0x01c0_0000, 2)
                    .unwrap(),
                data_before
            );

            // Unmapped RAM destination: the address is retained without RDY.
            let destination = target
                .cpu
                .mmu
                .store(&mut bus, 0x01c0_0204, 2, Width::Word, 0x40000);
            assert!(matches!(
                destination,
                Err(MemoryError::Bus(error)) if error.kind == BusErrorKind::Unmapped
            ));
            assert_eq!(
                target
                    .cpu
                    .mmu
                    .load_doubleword(&mut bus, 0x01c0_0200, 2)
                    .unwrap(),
                [0, 0x40000]
            );
            assert_eq!(
                target
                    .cpu
                    .mmu
                    .load_doubleword(&mut bus, 0x01c0_0000, 2)
                    .unwrap(),
                data_before
            );
        }
    }
}

fn seed_nested_effects(
    initiator: &mut ProcessorModuleState,
    other: &mut ProcessorModuleState,
    bus: &mut SystemBus,
) {
    initiator
        .cpu
        .mmu
        .store_doubleword(bus, 0x01c0_0000, 2, [0, 0x200])
        .unwrap();
    other
        .cpu
        .mmu
        .store_doubleword(bus, 0x01c0_0000, 2, [0x1122_3344, 0x5566_7788])
        .unwrap();
    bus.write_doubleword(0x200, [0; 2]).unwrap();
}

fn assert_no_stream_effects(
    initiator: &ProcessorModuleState,
    other: &ProcessorModuleState,
    initiator_before: &helios::devices::mxcc::MxccState,
    other_before: &helios::devices::mxcc::MxccState,
    bus: &mut SystemBus,
) {
    assert_eq!(*initiator.mxcc.borrow(), *initiator_before);
    assert_eq!(*other.mxcc.borrow(), *other_before);
    assert_eq!(bus.read_doubleword(0x200).unwrap(), [0, 0]);
}

#[test]
fn cross_module_physical_stream_commands_are_rejected_without_partial_effects() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for (initiator_mid, register_offset) in
            [(8u8, 0x100u64), (8, 0x200), (10, 0x100), (10, 0x200)]
        {
            let (mut low, mut high, mut bus) = module_pair(profile);
            let target_mid = if initiator_mid == 8 { 10 } else { 8 };
            let (initiator, other) = if initiator_mid == 8 {
                (&mut low, &mut high)
            } else {
                (&mut high, &mut low)
            };
            seed_nested_effects(initiator, other, &mut bus);
            let initiator_before = initiator.mxcc.borrow().clone();
            let other_before = other.mxcc.borrow().clone();
            let target = module(target_mid).mxcc_base + register_offset;
            let result = bus.write_doubleword(
                initiator.module.mxcc_base + register_offset,
                [(target >> 32) as u32, target as u32],
            );
            assert!(
                matches!(result, Err(error) if matches!(error.kind, BusErrorKind::Unsupported(_))),
                "physical stream offset {register_offset:#x} on MID {initiator_mid}: {result:?}"
            );
            assert_no_stream_effects(initiator, other, &initiator_before, &other_before, &mut bus);
        }
    }
}

#[test]
fn cross_module_asi_stream_commands_are_rejected_without_partial_effects() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for (initiator_mid, register_offset) in
            [(8u8, 0x100u64), (8, 0x200), (10, 0x100), (10, 0x200)]
        {
            let (mut low, mut high, mut bus) = module_pair(profile);
            let target_mid = if initiator_mid == 8 { 10 } else { 8 };
            let (initiator, other) = if initiator_mid == 8 {
                (&mut low, &mut high)
            } else {
                (&mut high, &mut low)
            };
            seed_nested_effects(initiator, other, &mut bus);
            let initiator_before = initiator.mxcc.borrow().clone();
            let other_before = other.mxcc.borrow().clone();
            let target = module(target_mid).mxcc_base + register_offset;
            let result = initiator.cpu.mmu.store_doubleword(
                &mut bus,
                0x01c0_0000 + register_offset as u32,
                2,
                [(target >> 32) as u32, target as u32],
            );
            assert!(
                matches!(result, Err(MemoryError::Bus(error)) if matches!(error.kind, BusErrorKind::Unsupported(_))),
                "ASI stream offset {register_offset:#x} on MID {initiator_mid}: {result:?}"
            );
            assert_no_stream_effects(initiator, other, &initiator_before, &other_before, &mut bus);
        }
    }
}

#[test]
fn stream_span_overlapping_control_window_or_port_is_rejected_before_effects() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for mid in [8u8, 10] {
            let (mut low, mut high, mut bus) = module_pair(profile);
            let (initiator, other) = if mid == 8 {
                (&mut low, &mut high)
            } else {
                (&mut high, &mut low)
            };
            let module = module(mid);
            // The last 32-byte block of the register window and the 32-byte
            // block containing the architected port must both be rejected.
            for target in [module.mxcc_base + 0xfe0, module.mxcc_port] {
                seed_nested_effects(initiator, other, &mut bus);
                let initiator_before = initiator.mxcc.borrow().clone();
                let other_before = other.mxcc.borrow().clone();
                let result = bus.write_doubleword(
                    initiator.module.mxcc_base + 0x200,
                    [(target >> 32) as u32, target as u32],
                );
                assert!(
                    matches!(result, Err(error) if matches!(error.kind, BusErrorKind::Unsupported(_))),
                    "stream target {target:#x} on MID {mid}: {result:?}"
                );
                assert_no_stream_effects(
                    initiator,
                    other,
                    &initiator_before,
                    &other_before,
                    &mut bus,
                );
            }
        }
    }
}

#[test]
fn cross_module_stream_cycle_is_rejected_in_an_isolated_child() {
    const CHILD_ENV: &str = "HELIOS_SMP_CYCLE_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        let (low, high, mut bus) = module_pair(MachineProfile::Ss20);
        bus.write_doubleword(0xff8c00000, [0xf, 0xf8c0_0200])
            .unwrap();
        bus.write_doubleword(0xffac00000, [0xf, 0xfac0_0200])
            .unwrap();
        let low_before = low.mxcc.borrow().clone();
        let high_before = high.mxcc.borrow().clone();
        assert!(bus
            .write_doubleword(0xff8c00200, [0xf, 0xfac0_0200])
            .is_err());
        assert_eq!(*low.mxcc.borrow(), low_before);
        assert_eq!(*high.mxcc.borrow(), high_before);
        return;
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cross_module_stream_cycle_is_rejected_in_an_isolated_child",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "cross-module stream cycle aborted isolated child: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("cross-module stream cycle did not terminate within five seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn populated_module_port_decodes_word_and_narrow_lanes_on_both_profiles() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (low, high, mut bus) = module_pair(profile);
        let _ = (&low, &high);
        for mid in [8u8, 10] {
            let module = module(mid);
            let identity = module.mxcc_identity;
            assert_eq!(bus.read32(module.mxcc_port).unwrap(), identity);
            let bytes = identity.to_be_bytes();
            for (offset, expected) in bytes.into_iter().enumerate() {
                assert_eq!(
                    bus.read8(module.mxcc_port + offset as u64).unwrap(),
                    expected,
                    "MID {mid} byte lane {offset}"
                );
            }
            assert_eq!(bus.read16(module.mxcc_port + 2).unwrap(), 0x0104);
            assert!(matches!(
                bus.read16(module.mxcc_port + 1),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert!(matches!(
                bus.read32(module.mxcc_port + 1),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));

            bus.write8(module.mxcc_port + 3, 0xff).unwrap();
            bus.write16(module.mxcc_port + 2, 0xffff).unwrap();
            bus.write32(module.mxcc_port, 0).unwrap();
            assert_eq!(bus.read32(module.mxcc_port).unwrap(), identity);
        }
    }
}

#[test]
fn absent_module_ports_and_control_windows_are_consistent_zero_windows() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine_for(profile);
        // Sun-4M B.III.8 architected ports for MIDs 9, A and B. Only MID 8 is
        // populated by the default machine, so all three are absent here.
        for (mid, port) in [
            (9u8, 0x000f_f9ff_fffc_u64),
            (10, 0x000f_faff_fffc),
            (11, 0x000f_fbff_fffc),
        ] {
            assert_eq!(machine.bus.read32(port), Ok(0), "MID {mid} port word");
            for offset in 0..4 {
                assert_eq!(
                    machine.bus.read8(port + offset),
                    Ok(0),
                    "MID {mid} port byte {offset}"
                );
            }
            assert_eq!(machine.bus.read16(port + 2), Ok(0));
            assert!(matches!(
                machine.bus.read16(port + 1),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert!(matches!(
                machine.bus.read32(port + 1),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            for offset in 0..4 {
                machine.bus.write8(port + offset, 0xff).unwrap();
            }
            machine.bus.write16(port + 2, 0xffff).unwrap();
            machine.bus.write32(port, 0xdead_beef).unwrap();
            assert_eq!(
                machine.bus.read32(port),
                Ok(0),
                "MID {mid} port after writes"
            );
        }

        // The absent control windows also accept every narrow lane as zero.
        for control in [0xff9c00a04u64, 0xffac00a04, 0xffbc00a04] {
            assert_eq!(machine.bus.read32(control), Ok(0));
            assert_eq!(machine.bus.read16(control), Ok(0));
            assert_eq!(machine.bus.read8(control + 3), Ok(0));
            machine.bus.write32(control, 0xdead_beef).unwrap();
            machine.bus.write16(control, 0xffff).unwrap();
            machine.bus.write8(control + 3, 0xff).unwrap();
            assert_eq!(machine.bus.read32(control), Ok(0));
        }
    }
}

#[test]
fn absent_port_doubleword_accesses_are_explicit_errors() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine_for(profile);
        for port in [0x000f_f9ff_fffc_u64, 0x000f_faff_fffc, 0x000f_fbff_fffc] {
            assert!(matches!(
                machine.bus.read_doubleword(port),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert!(matches!(
                machine.bus.write_doubleword(port, [0xdead_beef, 0xdead_beef]),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            // The aligned 8-byte block containing a four-byte port overlaps
            // the port word and must not be zero-filled or discarded.
            let block = port - 4;
            assert!(matches!(
                machine.bus.read_doubleword(block),
                Err(error) if error.kind == BusErrorKind::Unmapped
            ));
            assert!(matches!(
                machine.bus.write_doubleword(block, [0xdead_beef, 0xdead_beef]),
                Err(error) if error.kind == BusErrorKind::Unmapped
            ));
            assert_eq!(machine.bus.read32(port), Ok(0));
        }
    }
}

#[test]
fn absent_control_window_doubleword_accesses_are_unmapped() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine_for(profile);
        for control in [0x000f_f9c0_0a00_u64, 0x000f_fac0_0a00, 0x000f_fbc0_0a00] {
            assert!(matches!(
                machine.bus.read_doubleword(control),
                Err(error) if error.kind == BusErrorKind::Unmapped
            ));
            assert!(matches!(
                machine.bus.write_doubleword(control, [0xdead_beef, 0xdead_beef]),
                Err(error) if error.kind == BusErrorKind::Unmapped
            ));
            assert_eq!(machine.bus.read32(control), Ok(0));
            assert!(matches!(
                machine.bus.read_doubleword(control + 4),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert!(matches!(
                machine.bus.write_doubleword(control + 4, [0, 0]),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
        }

        // The zero window ends exactly at 0xf_f9c0_2000. The last word stays
        // zero, the first word outside it must not be zeroed, and a contained
        // doubleword in the window is still a wider unsupported access.
        let last_word = 0x000f_f9c0_1ffc_u64;
        let outside = 0x000f_f9c0_2000_u64;
        assert_eq!(machine.bus.read32(last_word), Ok(0));
        assert_eq!(machine.bus.write32(last_word, 0xdead_beef), Ok(()));
        assert_eq!(machine.bus.read32(last_word), Ok(0));
        assert!(matches!(
            machine.bus.read32(outside),
            Err(error) if error.kind == BusErrorKind::Unmapped
        ));
        assert!(matches!(
            machine.bus.write32(outside, 0xdead_beef),
            Err(error) if error.kind == BusErrorKind::Unmapped
        ));
        assert!(matches!(
            machine.bus.read_doubleword(0x000f_f9c0_1ff8),
            Err(error) if error.kind == BusErrorKind::Unmapped
        ));
        assert!(matches!(
            machine.bus.write_doubleword(0x000f_f9c0_1ff8, [0, 0]),
            Err(error) if error.kind == BusErrorKind::Unmapped
        ));
    }
}

#[test]
fn populated_port_doubleword_accesses_follow_width_policy() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (_, _, mut bus) = module_pair(profile);
        for mid in [8u8, 10] {
            let module = module(mid);
            assert!(matches!(
                bus.read_doubleword(module.mxcc_port),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert!(matches!(
                bus.write_doubleword(module.mxcc_port, [0xdead_beef, 0xdead_beef]),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert_eq!(bus.read32(module.mxcc_port), Ok(module.mxcc_identity));
            let block = module.mxcc_port - 4;
            assert!(matches!(
                bus.read_doubleword(block),
                Err(error) if error.kind == BusErrorKind::Unmapped
            ));
            assert!(matches!(
                bus.write_doubleword(block, [0xdead_beef, 0xdead_beef]),
                Err(error) if error.kind == BusErrorKind::Unmapped
            ));
            assert_eq!(bus.read32(module.mxcc_port), Ok(module.mxcc_identity));
        }
    }
}

#[test]
fn populated_control_window_decodes_narrow_lanes() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (_, _, mut bus) = module_pair(profile);
        for (base, identity) in [
            (0x000f_f8c0_0000_u64, 0x0800_0104_u32),
            (0x000f_fac0_0000, 0x0a00_0104),
        ] {
            let alias = base + 0x0f04;
            assert_eq!(bus.read32(alias), Ok(identity));
            let bytes = identity.to_be_bytes();
            for (offset, expected) in bytes.into_iter().enumerate() {
                assert_eq!(
                    bus.read8(alias + offset as u64),
                    Ok(expected),
                    "identity byte lane {offset} at {alias:#x}"
                );
            }
            assert_eq!(bus.read16(alias + 2), Ok(0x0104));
            assert!(matches!(
                bus.read16(alias + 1),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));
            assert!(matches!(
                bus.read32(alias + 1),
                Err(error) if error.kind == BusErrorKind::Misaligned
            ));

            bus.write32(base + 0x0a04, 0x38).unwrap();
            assert_eq!(bus.read8(base + 0x0a07), Ok(0x38));
            assert_eq!(bus.read16(base + 0x0a06), Ok(0x0038));
            assert_eq!(bus.read32(base + 0x0a04), Ok(0x38));
            bus.write8(base + 0x0a07, 0x3c).unwrap();
            assert_eq!(bus.read32(base + 0x0a04), Ok(0x3c));
            bus.write16(base + 0x0a06, 0x0004).unwrap();
            assert_eq!(bus.read32(base + 0x0a04), Ok(0x04));
        }
    }
}
