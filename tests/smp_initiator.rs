use helios::bus::{AccessContext, Bus, BusErrorKind, Initiator};
use helios::devices::nvram::Nvram;
use helios::machine::{Machine, MachineProfile, ProcessorModuleState};
use helios::mmu::Width;
use helios::system_bus::SystemBus;
use helios::topology::{ProcessorArbitration, ProcessorModule, ProcessorRunState};

fn module(processor: u8) -> ProcessorModule {
    ProcessorModule::from_processor(processor).unwrap()
}

fn pair(profile: MachineProfile) -> (ProcessorModuleState, ProcessorModuleState, SystemBus) {
    let cpu0 = ProcessorModuleState::new(profile, module(0)).unwrap();
    let cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
    let mut bus = SystemBus::with_layout(profile.layout());
    bus.install_sun4m(Nvram::default()).unwrap();
    bus.map_ram(0, 0x0200_0000).unwrap();
    bus.install_mxcc_module(cpu0.mxcc.clone());
    bus.install_mxcc_module(cpu2.mxcc.clone());
    assert!(!bus.configure_processor(0)); // boot CPU is configured at construction
    assert!(bus.configure_processor(2));
    (cpu0, cpu2, bus)
}

fn machine(profile: MachineProfile) -> Machine {
    Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap()
}

/// Physical 0xfe0002000 is the Sun-4M MID register. ASI 0x2f supplies the
/// high physical nibble; the low 32 bits are the register address.
const MID_VA: u32 = 0xe000_2000;

fn mid_register(bus: &mut SystemBus, initiator: &mut ProcessorModuleState) -> u32 {
    initiator
        .cpu
        .mmu
        .load(bus, MID_VA, 0x2f, Width::Word)
        .unwrap()
}

#[test]
fn modules_have_distinct_mids_and_mxcc_windows_select_the_addressed_module() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (mut cpu0, mut cpu2, mut bus) = pair(profile);
        let (module0, module2) = (module(0), module(2));

        assert_eq!(mid_register(&mut bus, &mut cpu0), 8, "{profile:?}");
        assert_eq!(mid_register(&mut bus, &mut cpu2), 10, "{profile:?}");

        // Host/debug access has no MBus owner and cannot inherit either CPU.
        assert_eq!(
            bus.with_host_access(|bus| bus.read32(0xfe00_02000))
                .unwrap(),
            0
        );

        // Local ASI 2 selects the executing module.
        cpu0.cpu
            .mmu
            .store(&mut bus, 0x01c0_0a04, 2, Width::Word, 0x10)
            .unwrap();
        cpu2.cpu
            .mmu
            .store(&mut bus, 0x01c0_0a04, 2, Width::Word, 0x20)
            .unwrap();
        assert_eq!(cpu0.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);
        assert_eq!(cpu2.mxcc.borrow().read_u64(0x01c0_0a00), 0x20);

        // A CPU-0 physical access to the MID-A control window targets CPU 2.
        cpu0.cpu
            .mmu
            .store(&mut bus, 0xfac0_0a04, 0x2f, Width::Word, 0x34)
            .unwrap();
        assert_eq!(cpu2.mxcc.borrow().read_u64(0x01c0_0a00), 0x34);
        assert_eq!(cpu0.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);

        // CPU 2 reaches MID 8's window symmetrically.
        cpu2.cpu
            .mmu
            .store(&mut bus, 0xf8c0_0a04, 0x2f, Width::Word, 0x08)
            .unwrap();
        assert_eq!(cpu0.mxcc.borrow().read_u64(0x01c0_0a00), 0x08);
        assert_eq!(cpu2.mxcc.borrow().read_u64(0x01c0_0a00), 0x34);

        // Port identities stay tied to the addressed module, not the caller.
        let (p0, p2) = (module0.mxcc_port, module2.mxcc_port);
        assert_eq!(bus.read32(p0).unwrap(), module0.mxcc_identity);
        assert_eq!(bus.read32(p2).unwrap(), module2.mxcc_identity);
        assert_eq!(
            cpu0.cpu.mmu.load(&mut bus, MID_VA, 0x2f, Width::Word),
            Ok(8)
        );
        assert_eq!(
            cpu2.cpu.mmu.load(&mut bus, MID_VA, 0x2f, Width::Word),
            Ok(10)
        );
    }
}

#[test]
fn ss5_keeps_fixed_integrated_mid_and_has_no_mbus_arbitration() {
    let mut machine = machine(MachineProfile::Ss5);
    let io = machine.bus.layout().io_control;
    assert_eq!(machine.bus.read32(io + 0x2000).unwrap(), 8);
    // The AER exists in the shared model but has no SS-5 processor effect.
    assert_eq!(machine.bus.read32(io + 0x1008).unwrap(), 1);
    machine.bus.write32(io + 0x1008, 1 << 2).unwrap();
    assert_eq!(
        machine.processor_arbitration(0),
        ProcessorArbitration::Enabled
    );
    assert_eq!(
        machine.processor_arbitration(2),
        ProcessorArbitration::Absent
    );
    assert!(!machine.set_processor_arbitration(2, true));
    assert!(!machine.configure_processor(2));
}

#[test]
fn standalone_aer_reset_reserved_bits_and_processor_two_transitions() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let io = machine.bus.layout().io_control;
        let aer = io + 0x1008;

        // Sun-4M 5.1.2 POR: bit 0 reads one, MID-9 enable set, MID-A/B clear.
        assert_eq!(machine.bus.read32(aer).unwrap(), 0x0000_0003, "{profile:?}");
        assert_eq!(
            machine.processor_arbitration(0),
            ProcessorArbitration::Enabled
        );
        // Processor 2 is absent until the board is configured, regardless of AER.
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Absent
        );
        assert!(machine.configure_processor(2));
        assert!(!machine.configure_processor(1));
        assert!(!machine.configure_processor(3));
        assert!(!machine.bus.processor_present(1));
        assert!(!machine.bus.processor_present(3));
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Disabled
        );

        // Only bits 0:3, 16:20 and 31 are defined; writes replace, not OR.
        machine.bus.write32(aer, u32::MAX).unwrap();
        assert_eq!(machine.bus.read32(aer).unwrap(), 0x801f_000f);
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Enabled
        );
        machine.bus.write32(aer, 0x7fff_fffe).unwrap();
        assert_eq!(machine.bus.read32(aer).unwrap(), 0x001f_000f);
        machine.bus.write32(aer, 0).unwrap();
        assert_eq!(machine.bus.read32(aer).unwrap(), 1);
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Disabled
        );

        // Reset MID-9 does not instantiate an absent processor 1.
        assert_eq!(
            machine.processor_arbitration(1),
            ProcessorArbitration::Absent
        );
        machine.bus.write32(aer, 1 << 1).unwrap();
        assert_eq!(
            machine.processor_arbitration(1),
            ProcessorArbitration::Absent
        );
        // MID-B is likewise absent on this two-module topology.
        machine.bus.write32(aer, 1 << 3).unwrap();
        assert_eq!(
            machine.processor_arbitration(3),
            ProcessorArbitration::Absent
        );
        for processor in [4, 15, 255] {
            assert_eq!(
                machine.processor_arbitration(processor),
                ProcessorArbitration::Absent
            );
        }

        // Host-side transitions affect only the addressed installed module.
        assert!(machine.set_processor_arbitration(2, true));
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Enabled
        );
        assert!(machine.set_processor_arbitration(2, false));
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Disabled
        );
        assert!(!machine.set_processor_arbitration(0, false));
        assert!(!machine.set_processor_arbitration(1, true));
        assert_eq!(
            machine.processor_arbitration(0),
            ProcessorArbitration::Enabled
        );
    }
}

#[test]
fn interleaved_cpu_dma_and_debug_accesses_do_not_leak_identity_or_privilege() {
    let profile = MachineProfile::Ss20;
    let mut machine = machine(profile);
    let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
    machine.bus.install_mxcc_module(cpu2.mxcc.clone());
    assert!(machine.configure_processor(2));
    assert!(machine.install_processor_banks(2));
    let io = machine.bus.layout().io_control;
    let mfsr = io + 0x1050;

    // Prepare one standalone IOMMU mapping for master 5.
    machine.bus.write32(io, 1).unwrap();
    machine.bus.write32(io + 4, 0x1000).unwrap();
    machine.bus.write32(io + 0x1008, 1 << 20).unwrap();
    machine.bus.write32(0x10000, 0x806).unwrap();
    machine.bus.write32(0x8000, 0x1111_1111).unwrap();

    // CPU 0 is supervisor at reset; its request capture reports S=1.
    machine.bus.write32(io + 0x1050, 0).unwrap(); // clear/unlock for the test
    assert_eq!(
        machine
            .cpu
            .mmu
            .load(&mut machine.bus, 0x8000, 0x20, Width::Word),
        Ok(0x1111_1111)
    );
    assert_eq!(machine.bus.read32(mfsr).unwrap() & 0x0180_0010, 0x0180_0010);

    // CPU 2 still presents MID A after CPU 0; local identity is not global.
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(0x0a)
    );
    // A direct CPU-2 error path must restore the caller's context.
    assert!(cpu2
        .cpu
        .mmu
        .store(&mut machine.bus, 0x7cc0_0000, 0x20, Width::Word, 1)
        .is_err());
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(0x0a)
    );

    // A nested MXCC stream rejected before mutation must also restore the
    // previous context; its target is another MBus module's control window.
    assert!(cpu2
        .cpu
        .mmu
        .store_doubleword(&mut machine.bus, 0x01c0_0200, 2, [0x0000_000f, 0xfac0_0000],)
        .is_err());
    assert_eq!(machine.bus.access_context().initiator, Initiator::Host);
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(0x0a)
    );

    // DMA is a device initiator: S=0, no boot mode and no CPU identity.
    machine.bus.write32(mfsr, 0).unwrap();
    assert_eq!(
        machine.dma_read(5, 0xff00_0000, Width::Word),
        Ok(0x1111_1111)
    );
    assert_eq!(machine.bus.read32(mfsr).unwrap() & 0x0180_1010, 0x0000_0010);
    // Host/debug access has MID 0 and does not inherit CPU-2's MID.
    assert_eq!(
        machine
            .bus
            .with_host_access(|bus| bus.read32(io + 0x2000))
            .unwrap(),
        0
    );
    // An error in an explicit host scope still restores the previous context.
    assert_eq!(
        machine
            .bus
            .with_host_access(|bus| bus.read32(io + 0x2001))
            .unwrap_err()
            .kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(0x0a)
    );
    assert_eq!(
        machine
            .cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(8)
    );

    // A DMA fault is reported to the device and leaves both CPUs and the
    // standalone owner in their previous translation state.
    machine.bus.write32(0x10000, 0).unwrap(); // clear V and W
    machine.bus.write32(io + 0x18, 0xff00_0000).unwrap();
    assert!(matches!(
        machine.dma_read(5, 0xff00_0000, Width::Word),
        Err(helios::dma::DmaError::Fault { .. })
    ));
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(0x0a)
    );
    assert_eq!(
        machine
            .cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(8)
    );
}

#[test]
fn pending_interrupts_and_timer_banks_do_not_grant_processor_two_arbitration() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        assert!(machine.configure_processor(2));
        assert!(machine.install_processor_banks(2));
        let io = machine.bus.layout().io_control;
        let system_interrupt = machine.bus.system_interrupt_base();
        let processor_interrupt = machine.bus.processor_interrupt_base(2);
        let processor_timer = machine.bus.processor_timer_base(2);

        machine.bus.write32(system_interrupt + 8, u32::MAX).unwrap();
        machine
            .bus
            .write32(processor_interrupt + 8, 1 << 17)
            .unwrap();
        machine.bus.write32(processor_timer, 2 << 9).unwrap();
        machine.advance_ns(500);
        let devices = machine.bus.devices_mut().unwrap();
        devices
            .interrupts
            .set_source(helios::devices::interrupt::Source::ModuleError, true);
        machine.bus.write32(system_interrupt + 8, u32::MAX).unwrap();
        let levels = machine.bus.interrupt_levels_for(2);
        assert_ne!(levels & (1 << 1), 0, "{profile:?}");
        assert_ne!(levels & (1 << 14), 0, "{profile:?}");
        assert_ne!(levels & (1 << 15), 0, "{profile:?}");
        // The directed software and timer levels stay in the CPU-2 bank; only
        // the broadcast level-15 source reaches CPU 0.
        let levels0 = machine.bus.interrupt_levels_for(0);
        assert_eq!(levels0 & (1 << 1), 0, "{profile:?}");
        assert_eq!(levels0 & (1 << 14), 0, "{profile:?}");
        assert_ne!(levels0 & (1 << 15), 0, "{profile:?}");
        {
            let timers = &machine.bus.devices().unwrap().timers;
            assert!(!timers.processor_irq_for(0), "{profile:?}");
            assert!(timers.processor_irq_for(2), "{profile:?}");
        }
        assert_eq!(
            machine.bus.read32(io + 0x1008).unwrap() & (1 << 2),
            0,
            "{profile:?}: pending events granted arbitration"
        );
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Disabled
        );
        assert_eq!(
            machine.processor_arbitration(0),
            ProcessorArbitration::Enabled
        );

        // Enable through the board transition, not through an interrupt.
        assert!(machine.set_processor_arbitration(2, true));
        assert_eq!(
            machine.processor_arbitration(2),
            ProcessorArbitration::Enabled
        );
        // The banks remain independent and the shared clock is untouched.
        assert_ne!(
            machine.bus.interrupt_levels_for(2) & (1 << 14),
            0,
            "{profile:?}"
        );
    }
}

#[test]
fn module_local_reset_preserves_board_state_and_run_state_distinguishes_denial() {
    let profile = MachineProfile::Ss20;
    let mut machine = machine(profile);
    let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
    machine.bus.install_mxcc_module(cpu2.mxcc.clone());
    assert!(machine.configure_processor(2));
    assert!(machine.install_processor_banks(2));
    let io = machine.bus.layout().io_control;
    machine.bus.write32(0x10000, 0x806).unwrap();
    machine.bus.write32(0x8000, 0x1111_1111).unwrap();
    machine.bus.write32(0xa000, 0xaaaa_aaaa).unwrap();
    machine.bus.write32(io, 1).unwrap();
    machine.bus.write32(io + 4, 0x1000).unwrap();
    machine
        .bus
        .write32(io + 0x1008, (1 << 20) | (1 << 2))
        .unwrap();
    assert!(machine.set_processor_arbitration(2, true));
    assert_eq!(
        machine.dma_read(5, 0xff00_0000, Width::Word),
        Ok(0x1111_1111)
    );

    machine.cpu.write_reg(1, 0x1111_1111);
    cpu2.cpu.write_reg(1, 0x2222_2222);
    cpu2.cpu
        .mmu
        .store(&mut machine.bus, 0x01c0_0a04, 2, Width::Word, 0x20)
        .unwrap();
    assert_eq!(
        cpu2.run_state(&machine.bus),
        ProcessorRunState::Ready,
        "processor 2 should be eligible"
    );

    // Module-local CPU-0 reset touches only CPU/MXCC 0.
    assert!(machine.reset_processor_module(0));
    assert_eq!(machine.cpu.read_reg(1), 0);
    assert_eq!(cpu2.cpu.read_reg(1), 0x2222_2222);
    assert_eq!(cpu2.mxcc.borrow().read_u64(0x01c0_0a00), 0x20);
    assert_eq!(machine.bus.read32(io + 0x1008).unwrap() & (1 << 2), 1 << 2);
    // The standalone IOMMU cache survived: a changed PTE still hits old data.
    machine.bus.write32(0x10000, 0xa06).unwrap();
    assert_eq!(
        machine.dma_read(5, 0xff00_0000, Width::Word),
        Ok(0x1111_1111)
    );

    // A module's own reset clears its CPU/MXCC and its error latch.
    cpu2.cpu
        .mmu
        .store(&mut machine.bus, 0, 4, Width::Word, 0)
        .unwrap(); // translation and boot mode off
    machine.bus.write32(0, 0x91d0_2007).unwrap(); // ta 7 with ET=0
    assert!(cpu2.cpu.step(&mut machine.bus).is_err());
    assert_eq!(cpu2.run_state(&machine.bus), ProcessorRunState::CpuError);
    assert!(machine.set_processor_arbitration(2, false));
    assert_eq!(
        cpu2.run_state(&machine.bus),
        ProcessorRunState::ArbitrationDenied
    );
    assert!(machine.set_processor_arbitration(2, true));
    cpu2.reset();
    assert_eq!(cpu2.run_state(&machine.bus), ProcessorRunState::Ready);
    assert_eq!(cpu2.cpu.read_reg(1), 0);
    assert_eq!(cpu2.mxcc.borrow().read_u64(0x01c0_0a00), 0);
    assert_eq!(
        cpu2.cpu
            .mmu
            .load(&mut machine.bus, MID_VA, 0x2f, Width::Word),
        Ok(0x0a),
        "module reset must retain the MID-A initiator identity"
    );
    assert_eq!(machine.bus.read32(io + 0x1008).unwrap() & (1 << 2), 1 << 2);

    // Whole-machine reset clears shared devices, AER and the standalone IOMMU.
    machine.cpu.write_reg(1, 0x3333_3333);
    machine.bus.write32(0x8000, 0x4444_4444).unwrap();
    machine.reset();
    assert_eq!(machine.cpu.read_reg(1), 0);
    assert_eq!(machine.bus.read32(0x8000).unwrap(), 0x4444_4444);
    assert_eq!(machine.bus.read32(io + 0x1008).unwrap(), 0x0000_0003);
    assert_eq!(
        machine.processor_arbitration(2),
        ProcessorArbitration::Disabled
    );
    // The standalone cache was reset, so the changed PTE is now used after
    // the board configuration is restored.
    machine.bus.write32(io, 1).unwrap();
    machine.bus.write32(io + 4, 0x1000).unwrap();
    machine.bus.write32(io + 0x1008, 1 << 20).unwrap();
    assert_eq!(
        machine.dma_read(5, 0xff00_0000, Width::Word),
        Ok(0xaaaa_aaaa)
    );
    // The independently constructed CPU-2 integer state is not Machine-owned
    // yet; its MXCC is bus-installed and does take the whole-machine reset.
    assert_eq!(cpu2.mxcc.borrow().read_u64(0x01c0_0a00), 0);
}

#[test]
fn explicit_access_context_selects_cpu_dma_or_host_identity() {
    let (_, _cpu2, mut bus) = pair(MachineProfile::Ss20);
    let io = bus.layout().io_control;
    let cpu2 = module(2);
    assert_eq!(
        bus.with_access_context(AccessContext::cpu(cpu2, false), |bus| {
            bus.read32(io + 0x2000)
        })
        .unwrap(),
        u32::from(cpu2.mid)
    );
    assert_eq!(
        bus.with_access_context(AccessContext::dma(5), |bus| { bus.read32(io + 0x2000) })
            .unwrap(),
        0
    );
    let before = bus.access_context();
    assert_eq!(
        bus.with_host_access(|bus| bus.read32(io + 0x2000)).unwrap(),
        0
    );
    assert_eq!(bus.access_context(), before);
    assert_eq!(bus.access_context().initiator, Initiator::Host);
    assert_eq!(bus.read32(io + 0x2000).unwrap(), 0);
}

#[test]
fn mbus_async_fault_captures_the_faulting_module_mid() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
        machine.bus.install_mxcc_module(cpu2.mxcc.clone());
        assert!(machine.configure_processor(2));
        assert!(machine.install_processor_banks(2));
        assert!(machine.set_processor_arbitration(2, true));
        let io = machine.bus.layout().io_control;
        let floppy = machine.bus.layout().floppy as u32;
        let baseline = AccessContext::dma(5);
        machine.bus.set_access_context(baseline);
        cpu2.cpu
            .mmu
            .store(&mut machine.bus, floppy, 0x2f, Width::Word, 0x1234)
            .unwrap();
        assert_eq!(machine.bus.access_context(), baseline);
        // A second fault while locked retains the first MID and reports ME.
        machine
            .cpu
            .mmu
            .store(&mut machine.bus, floppy, 0x2f, Width::Word, 0x5678)
            .unwrap();
        let locked = machine.bus.read32(io + 0x1000).unwrap();
        assert_ne!(locked & (1 << 31), 0);
        assert_eq!(locked & (1 << 19), 1 << 19);
        assert_eq!((locked >> 20) & 0xf, 0xa, "{profile:?}: {locked:08x}");

        // Reading through a different processor does not rewrite the MID.
        assert_eq!(
            machine
                .cpu
                .mmu
                .load(&mut machine.bus, io as u32 + 0x1000, 0x2f, Width::Word)
                .unwrap(),
            locked
        );
        machine.bus.write32(io + 0x1000, 0).unwrap();
        machine
            .cpu
            .mmu
            .store(&mut machine.bus, floppy, 0x2f, Width::Word, 0x9abc)
            .unwrap();
        let cpu0 = machine.bus.read32(io + 0x1000).unwrap();
        assert_eq!((cpu0 >> 20) & 0xf, 8, "{profile:?}: {cpu0:08x}");
    }
}

fn fault_cpu2(machine: &mut Machine, cpu2: &mut ProcessorModuleState) {
    let floppy = machine.bus.layout().floppy as u32;
    cpu2.cpu
        .mmu
        .store(&mut machine.bus, floppy, 0x2f, Width::Word, 0x1234)
        .unwrap();
}

fn fault_cpu0(machine: &mut Machine, offset: u32, width: Width) {
    let floppy = machine.bus.layout().floppy as u32 + offset;
    machine
        .cpu
        .mmu
        .store(&mut machine.bus, floppy, 0x2f, width, 0x5678)
        .unwrap();
}

#[test]
fn standalone_afsr_read_and_afar_do_not_release_capture() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
        machine.bus.install_mxcc_module(cpu2.mxcc.clone());
        assert!(machine.configure_processor(2));
        assert!(machine.install_processor_banks(2));
        assert!(machine.set_processor_arbitration(2, true));
        let io = machine.bus.layout().io_control;
        fault_cpu2(&mut machine, &mut cpu2);
        let original = machine.bus.read32(io + 0x1000).unwrap();
        let address = machine.bus.read32(io + 0x1004).unwrap();
        assert_eq!((original >> 20) & 0xf, 0xa, "{profile:?}");

        fault_cpu0(&mut machine, 4, Width::Half);
        let captured = machine.bus.read32(io + 0x1000).unwrap();
        assert_eq!(captured, original | (1 << 19), "{profile:?}");
        assert_eq!(machine.bus.read32(io + 0x1004).unwrap(), address);
        // AFAR is read-only; a guest write does not replace the capture.
        machine.bus.write32(io + 0x1004, 0x1234_5678).unwrap();
        assert_eq!(machine.bus.read32(io + 0x1004).unwrap(), address);
    }
}

#[test]
fn any_standalone_afsr_write_acknowledges_without_synthetic_fields() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for value in [u32::MAX, 1 << 31, 0x5a5a_a5a5, 0] {
            let mut machine = machine(profile);
            let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
            machine.bus.install_mxcc_module(cpu2.mxcc.clone());
            assert!(machine.configure_processor(2));
            assert!(machine.install_processor_banks(2));
            assert!(machine.set_processor_arbitration(2, true));
            let io = machine.bus.layout().io_control;
            fault_cpu2(&mut machine, &mut cpu2);
            machine.bus.write32(io + 0x1000, value).unwrap();
            assert_eq!(machine.bus.read32(io + 0x1000).unwrap(), 0);
            fault_cpu0(&mut machine, 4, Width::Word);
            assert_eq!(
                (machine.bus.read32(io + 0x1000).unwrap() >> 20) & 0xf,
                8,
                "{profile:?}"
            );
        }
    }
}

#[test]
fn m_to_s_source_and_per_cpu_level15_acknowledgements_are_independent() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
        machine.bus.install_mxcc_module(cpu2.mxcc.clone());
        assert!(machine.configure_processor(2));
        assert!(machine.install_processor_banks(2));
        assert!(machine.set_processor_arbitration(2, true));
        let io = machine.bus.layout().io_control;
        let irq = machine.bus.system_interrupt_base();
        let cpu0_bank = machine.bus.processor_interrupt_base(0);
        let cpu2_bank = machine.bus.processor_interrupt_base(2);
        const WRITE_BUFFER: u32 = 1 << 29;
        const HARD15: u32 = 1 << 15;
        machine
            .bus
            .write32(irq + 8, (1 << 31) | WRITE_BUFFER)
            .unwrap();
        fault_cpu2(&mut machine, &mut cpu2);
        assert_eq!(
            machine.bus.read32(irq).unwrap() & WRITE_BUFFER,
            WRITE_BUFFER
        );
        assert_eq!(machine.bus.read32(cpu0_bank).unwrap() & HARD15, HARD15);
        assert_eq!(machine.bus.read32(cpu2_bank).unwrap() & HARD15, HARD15);

        machine.bus.write32(cpu0_bank + 4, HARD15).unwrap();
        assert_eq!(machine.bus.read32(cpu0_bank).unwrap() & HARD15, 0);
        assert_eq!(machine.bus.read32(cpu2_bank).unwrap() & HARD15, HARD15);
        assert_eq!(
            machine.bus.read32(irq).unwrap() & WRITE_BUFFER,
            WRITE_BUFFER
        );

        machine.bus.write32(io + 0x1000, u32::MAX).unwrap();
        assert_eq!(machine.bus.read32(irq).unwrap() & WRITE_BUFFER, 0);
        assert_eq!(machine.bus.read32(cpu2_bank).unwrap() & HARD15, HARD15);
        machine.bus.write32(cpu2_bank + 4, HARD15).unwrap();
        assert_eq!(machine.bus.read32(cpu2_bank).unwrap() & HARD15, 0);
    }
}

#[test]
fn m_to_s_source_is_masked_at_event_time_and_reset_clears_it() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
        machine.bus.install_mxcc_module(cpu2.mxcc.clone());
        assert!(machine.configure_processor(2));
        assert!(machine.install_processor_banks(2));
        assert!(machine.set_processor_arbitration(2, true));
        let irq = machine.bus.system_interrupt_base();
        let cpu0_bank = machine.bus.processor_interrupt_base(0);
        let cpu2_bank = machine.bus.processor_interrupt_base(2);
        const WRITE_BUFFER: u32 = 1 << 29;
        const HARD15: u32 = 1 << 15;
        // Leave the M-to-S source masked while the event is asserted.
        machine.bus.write32(irq + 8, 1 << 31).unwrap();
        fault_cpu2(&mut machine, &mut cpu2);
        assert_eq!(
            machine.bus.read32(irq).unwrap() & WRITE_BUFFER,
            WRITE_BUFFER
        );
        assert_eq!(machine.bus.read32(cpu0_bank).unwrap() & HARD15, 0);
        assert_eq!(machine.bus.read32(cpu2_bank).unwrap() & HARD15, 0);
        machine.bus.write32(irq + 8, WRITE_BUFFER).unwrap();
        assert_eq!(machine.bus.read32(cpu0_bank).unwrap() & HARD15, 0);
        assert_eq!(machine.bus.read32(cpu2_bank).unwrap() & HARD15, 0);
        machine.reset();
        assert_eq!(machine.bus.read32(irq).unwrap() & WRITE_BUFFER, 0);
    }
}

#[test]
fn standalone_plain_store_fields_match_sun4m_encodings() {
    use helios::bus::BusError;

    // Sun-4M 5.2.1/5.2.2: expected words independently encode SIZ, S, MID,
    // SA, SSIZ and PA[35:32]. AFAR must retain bit 31 of the low address.
    let cases = [
        (
            2,
            false,
            1,
            0x000e_8000_001f,
            BusErrorKind::SBusError,
            0x90a1_f20e,
        ),
        (
            0,
            true,
            2,
            0x000e_8000_001e,
            BusErrorKind::SBusTimeout,
            0xa381_e40e,
        ),
        (
            2,
            true,
            4,
            0x000e_f000_001c,
            BusErrorKind::SBusError,
            0x95a1_c00e,
        ),
    ];
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let io = machine.bus.layout().io_control;
        for (processor, supervisor, bytes, address, kind, expected) in cases {
            machine.bus.write32(io + 0x1000, u32::MAX).unwrap();
            let previous = machine.bus.access_context();
            machine
                .bus
                .with_access_context(AccessContext::cpu(module(processor), supervisor), |bus| {
                    assert!(bus.capture_sbus_write_error(BusError { address, kind }, bytes))
                });
            assert_eq!(machine.bus.access_context(), previous);
            assert_eq!(
                machine.bus.read32(io + 0x1000).unwrap(),
                expected,
                "{profile:?}"
            );
            assert_eq!(machine.bus.read32(io + 0x1004).unwrap(), address as u32);
        }
    }
}

#[test]
fn m_to_s_reassertion_keeps_simultaneous_module_error_and_cpu_captures() {
    use helios::devices::interrupt::Source;

    const M_TO_S: u32 = 1 << 29;
    const MODULE_ERROR: u32 = 1 << 30;
    const HARD15: u32 = 1 << 15;
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = machine(profile);
        let mut cpu2 = ProcessorModuleState::new(profile, module(2)).unwrap();
        assert!(machine.configure_processor(2));
        assert!(machine.install_processor_banks(2));
        assert!(machine.set_processor_arbitration(2, true));
        let io = machine.bus.layout().io_control;
        let irq = machine.bus.system_interrupt_base();
        let bank0 = machine.bus.processor_interrupt_base(0);
        let bank2 = machine.bus.processor_interrupt_base(2);
        machine
            .bus
            .write32(irq + 8, (1 << 31) | M_TO_S | MODULE_ERROR)
            .unwrap();

        fault_cpu2(&mut machine, &mut cpu2);
        machine
            .bus
            .devices_mut()
            .unwrap()
            .interrupts
            .set_source(Source::ModuleError, true);
        assert_eq!(machine.bus.read32(irq).unwrap(), M_TO_S | MODULE_ERROR);
        machine.bus.write32(io + 0x1000, u32::MAX).unwrap();
        assert_eq!(machine.bus.read32(irq).unwrap(), MODULE_ERROR);
        for bank in [bank0, bank2] {
            assert_eq!(machine.bus.read32(bank).unwrap() & HARD15, HARD15);
            machine.bus.write32(bank + 4, HARD15).unwrap();
            assert_eq!(machine.bus.read32(bank).unwrap() & HARD15, 0);
        }

        // The held ModuleError input does not prevent a new M-to-S event
        // from recapturing hard level 15 after both CPUs acknowledged it.
        fault_cpu2(&mut machine, &mut cpu2);
        for bank in [bank0, bank2] {
            assert_eq!(machine.bus.read32(bank).unwrap() & HARD15, HARD15);
        }
        machine.bus.write32(irq + 12, M_TO_S).unwrap();
        assert_eq!(machine.bus.read32(irq).unwrap(), M_TO_S | MODULE_ERROR);
        machine.bus.write32(bank0 + 4, HARD15).unwrap();
        machine.bus.write32(io + 0x1000, 0).unwrap();
        assert_eq!(machine.bus.read32(irq).unwrap(), MODULE_ERROR);
        assert_eq!(machine.bus.read32(bank0).unwrap() & HARD15, 0);
        assert_eq!(machine.bus.read32(bank2).unwrap() & HARD15, HARD15);
        machine
            .bus
            .devices_mut()
            .unwrap()
            .interrupts
            .set_source(Source::ModuleError, false);
        assert_eq!(machine.bus.read32(irq).unwrap(), 0);
        assert_eq!(machine.bus.read32(bank2).unwrap() & HARD15, HARD15);

        machine.reset();
        assert_eq!(machine.bus.read32(irq).unwrap(), 0);
        assert_eq!(machine.bus.read32(bank2).unwrap() & HARD15, 0);
    }
}
