use helios::{
    bus::Bus,
    machine::{MachineProfile, ProcessorModuleState},
    mmu::Width,
    system_bus::SystemBus,
    topology::{ProcessorModule, ProcessorTopology, PROCESSOR_MODULES},
};

fn modules() -> (ProcessorModule, ProcessorModule) {
    (
        ProcessorModule::from_storage_index(0).unwrap(),
        ProcessorModule::from_storage_index(1).unwrap(),
    )
}

fn module_pair(profile: MachineProfile) -> (ProcessorModuleState, ProcessorModuleState, SystemBus) {
    let (module0, module1) = modules();
    let processor0 = ProcessorModuleState::new(profile, module0).unwrap();
    let processor1 = ProcessorModuleState::new(profile, module1).unwrap();
    let mut bus = SystemBus::with_layout(profile.layout());
    bus.install_mxcc_module(processor0.mxcc.clone());
    bus.install_mxcc_module(processor1.mxcc.clone());
    (processor0, processor1, bus)
}

#[test]
fn processor_table_preserves_mbus_numbering_and_profile_limits() {
    assert_eq!(PROCESSOR_MODULES.len(), 2);
    assert_eq!(
        ProcessorModule::from_storage_index(0),
        Some(ProcessorModule {
            storage_index: 0,
            connector: 0,
            processor: 0,
            mid: 8,
            mxcc_base: 0x000f_f8c0_0000,
            mxcc_port: 0x000f_f8ff_fffc,
            mxcc_identity: 0x0800_0104,
        })
    );
    assert_eq!(
        ProcessorModule::from_storage_index(1),
        Some(ProcessorModule {
            storage_index: 1,
            connector: 1,
            processor: 2,
            mid: 10,
            mxcc_base: 0x000f_fac0_0000,
            mxcc_port: 0x000f_faff_fffc,
            mxcc_identity: 0x0a00_0104,
        })
    );
    assert_eq!(ProcessorModule::from_storage_index(2), None);
    assert_eq!(
        ProcessorModule::from_processor(0),
        Some(PROCESSOR_MODULES[0])
    );
    assert_eq!(ProcessorModule::from_processor(1), None);
    assert_eq!(
        ProcessorModule::from_processor(2),
        Some(PROCESSOR_MODULES[1])
    );
    assert_eq!(ProcessorModule::from_processor(3), None);
    assert_eq!(ProcessorModule::from_mid(8), Some(PROCESSOR_MODULES[0]));
    assert_eq!(ProcessorModule::from_mid(9), None);
    assert_eq!(ProcessorModule::from_mid(10), Some(PROCESSOR_MODULES[1]));
    assert_eq!(ProcessorModule::from_mid(11), None);

    let dual = ProcessorTopology::dual();
    assert_eq!(dual.len(), 2);
    assert_eq!(dual.module_for_processor(0), Some(PROCESSOR_MODULES[0]));
    assert_eq!(dual.module_for_processor(1), None);
    assert_eq!(dual.module_for_processor(2), Some(PROCESSOR_MODULES[1]));
    assert_eq!(dual.module_for_mid(8), Some(PROCESSOR_MODULES[0]));
    assert_eq!(dual.module_for_mid(10), Some(PROCESSOR_MODULES[1]));
    assert_eq!(dual.module_for_mid(9), None);

    assert_eq!(
        MachineProfile::Ss5.processor_topology(1),
        Some(ProcessorTopology::empty())
    );
    assert_eq!(MachineProfile::Ss5.processor_topology(2), None);
    assert_eq!(
        MachineProfile::Ss20.processor_topology(1),
        Some(ProcessorTopology::single())
    );
    assert_eq!(MachineProfile::Ss20.processor_topology(2), Some(dual));
    assert_eq!(MachineProfile::Axil320.processor_topology(2), Some(dual));
    assert!(ProcessorModuleState::new(MachineProfile::Ss5, PROCESSOR_MODULES[0]).is_none());
    assert!(ProcessorModuleState::new(MachineProfile::Ss20, PROCESSOR_MODULES[0]).is_some());
    assert!(ProcessorModuleState::new(MachineProfile::Ss20, PROCESSOR_MODULES[1]).is_some());
}

#[test]
fn module_identity_and_register_state_are_independent() {
    let (mut processor0, mut processor1, mut bus) = module_pair(MachineProfile::Ss20);
    let (module0, module1) = modules();

    assert_eq!(processor0.mxcc.borrow().mid(), 8);
    assert_eq!(processor1.mxcc.borrow().mid(), 10);
    assert_eq!(bus.read32(module0.mxcc_port).unwrap(), 0x0800_0104);
    assert_eq!(bus.read32(module1.mxcc_port).unwrap(), 0x0a00_0104);

    processor0.cpu.write_reg(1, 0x1111_1111);
    processor1.cpu.write_reg(1, 0x2222_2222);
    assert_eq!(processor0.cpu.read_reg(1), 0x1111_1111);
    assert_eq!(processor1.cpu.read_reg(1), 0x2222_2222);

    bus.write32(module0.mxcc_base + 0x0a04, 0x3c).unwrap();
    assert_eq!(processor0.mxcc.borrow().read_u64(0x01c0_0a00), 0x3c);
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0a00), 0);

    processor0
        .cpu
        .mmu
        .store_doubleword(&mut bus, 0x01c0_0a00, 2, [0, 0x10])
        .unwrap();
    assert_eq!(processor0.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0a00), 0);

    bus.write32(module1.mxcc_base + 0x0a04, 0x0c).unwrap();
    assert_eq!(processor0.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0a00), 0x0c);

    processor0
        .cpu
        .mmu
        .store(&mut bus, 0, 6, Width::Word, 0x1337)
        .unwrap();
    processor1
        .cpu
        .mmu
        .store(&mut bus, 0, 6, Width::Word, 0x2448)
        .unwrap();
    assert_eq!(
        processor0
            .cpu
            .mmu
            .load(&mut bus, 0, 6, Width::Word)
            .unwrap(),
        0x1337
    );
    assert_eq!(
        processor1
            .cpu
            .mmu
            .load(&mut bus, 0, 6, Width::Word)
            .unwrap(),
        0x2448
    );

    let before = processor1.mxcc.borrow().clone();
    assert!(processor1
        .mxcc
        .borrow_mut()
        .write_u64(&mut bus, 0x01c0_0100, module1.mxcc_base)
        .is_err());
    assert_eq!(*processor1.mxcc.borrow(), before);
    assert!(processor1
        .mxcc
        .borrow_mut()
        .write_u64(&mut bus, 0x01c0_0200, module1.mxcc_base + 0x800)
        .is_err());
    assert_eq!(*processor1.mxcc.borrow(), before);

    bus.write32(module1.mxcc_port, 0xdead_beef).unwrap();
    assert_eq!(bus.read32(module1.mxcc_port).unwrap(), 0x0a00_0104);
    assert_eq!(bus.read32(0x000f_f9c0_0a04).unwrap(), 0);
    assert_eq!(bus.read32(0x000f_fbc0_0a04).unwrap(), 0);

    processor1.cpu.write_reg(1, 0x3333_3333);
    bus.write32(module1.mxcc_base + 0x0a04, 0x20).unwrap();
    processor1
        .cpu
        .mmu
        .store(&mut bus, 0, 6, Width::Word, 0x3559)
        .unwrap();
    processor0.reset();
    assert_eq!(processor0.cpu.read_reg(1), 0);
    assert_eq!(processor0.mxcc.borrow().read_u64(0x01c0_0a00), 0);
    assert_eq!(
        processor0
            .cpu
            .mmu
            .load(&mut bus, 0, 6, Width::Word)
            .unwrap(),
        0
    );
    assert_eq!(processor1.cpu.read_reg(1), 0x3333_3333);
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0a00), 0x20);
    assert_eq!(
        processor1
            .cpu
            .mmu
            .load(&mut bus, 0, 6, Width::Word)
            .unwrap(),
        0x3559
    );
    processor1.reset();
    assert_eq!(processor1.cpu.read_reg(1), 0);
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0a00), 0);
    assert_eq!(bus.read32(module0.mxcc_port).unwrap(), 0x0800_0104);
    assert_eq!(bus.read32(module1.mxcc_port).unwrap(), 0x0a00_0104);
}

#[test]
fn mxcc_widths_and_aliases_route_by_module() {
    let (processor0, processor1, mut bus) = module_pair(MachineProfile::Ss20);
    let (module0, module1) = modules();

    bus.write8(module0.mxcc_base + 0x0c07, 0x5a).unwrap();
    assert_eq!(processor0.mxcc.borrow().read_u64(0x01c0_0c00), 0x5a);
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0c00), 0);

    bus.write16(module0.mxcc_base + 0x0c06, 0x1234).unwrap();
    assert_eq!(processor0.mxcc.borrow().read_u64(0x01c0_0c00), 0x1234);
    assert_eq!(bus.read32(module0.mxcc_base + 0x0c04).unwrap(), 0x1234);
    assert_eq!(bus.read8(module0.mxcc_base + 0x0c07).unwrap(), 0x34);

    bus.write32(module0.mxcc_base + 0x0c04, 0x1122_3344)
        .unwrap();
    assert_eq!(
        bus.read_doubleword(module0.mxcc_base + 0x0c00).unwrap(),
        [0, 0x1122_3344]
    );

    bus.write_doubleword(module0.mxcc_base + 0x0c00, [0x5566_7788, 0x99aa_bbcc])
        .unwrap();
    assert_eq!(
        processor0.mxcc.borrow().read_u64(0x01c0_0c00),
        0x5566_7788_99aa_bbcc
    );
    assert_eq!(processor1.mxcc.borrow().read_u64(0x01c0_0c00), 0);

    bus.write_doubleword(module1.mxcc_base + 0x0c00, [0x0102_0304, 0x0506_0708])
        .unwrap();
    assert_eq!(
        processor1.mxcc.borrow().read_u64(0x01c0_0c00),
        0x0102_0304_0506_0708
    );
    assert_eq!(
        processor0.mxcc.borrow().read_u64(0x01c0_0c00),
        0x5566_7788_99aa_bbcc
    );
}

fn module_pair_with_ram(
    profile: MachineProfile,
    bytes: usize,
) -> (ProcessorModuleState, ProcessorModuleState, SystemBus) {
    let (low, high, mut bus) = module_pair(profile);
    bus.map_ram(0, bytes).unwrap();
    (low, high, bus)
}

fn configure_reference_translation(
    processor: &mut ProcessorModuleState,
    bus: &mut SystemBus,
    context: u32,
) {
    use helios::mmu::PCR_EN;
    processor
        .cpu
        .mmu
        .store(bus, 1 << 8, 4, Width::Word, 0x100)
        .unwrap();
    processor
        .cpu
        .mmu
        .store(bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    processor
        .cpu
        .mmu
        .store(bus, 2 << 8, 4, Width::Word, context)
        .unwrap();
}

fn install_distinct_context_pages(bus: &mut SystemBus) {
    // Context 0 maps VA 0 to PA 0x01000000; context 0x100 maps it to
    // PA 0x02000000 through a separate level-1 table. Expected values follow
    // the reference-MMU walk, not the MXCC or bus decoder.
    bus.write32(0x1000, 0x201).unwrap();
    bus.write32(0x1400, 0x501).unwrap();
    bus.write32(0x2000, 0x0010_008e).unwrap();
    bus.write32(0x5000, 0x0020_008e).unwrap();
    bus.write32(0x0100_0000, 0x1111_1111).unwrap();
    bus.write32(0x0200_0000, 0x2222_2222).unwrap();
}

#[test]
fn mmu_contexts_are_independent_across_a_shared_bus_on_both_profiles() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (mut low, mut high, mut bus) = module_pair_with_ram(profile, 0x0300_0000);
        install_distinct_context_pages(&mut bus);
        configure_reference_translation(&mut low, &mut bus, 0);
        configure_reference_translation(&mut high, &mut bus, 0x100);
        assert_eq!(
            low.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111,
            "{profile:?} module MID 8"
        );
        assert_eq!(
            high.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x2222_2222,
            "{profile:?} module MID 10"
        );
    }
}

#[test]
fn resetting_one_module_preserves_the_other_modules_translations() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (mut low, mut high, mut bus) = module_pair_with_ram(profile, 0x0300_0000);
        install_distinct_context_pages(&mut bus);
        configure_reference_translation(&mut low, &mut bus, 0);
        configure_reference_translation(&mut high, &mut bus, 0x100);
        assert_eq!(
            low.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111
        );
        assert_eq!(
            high.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x2222_2222
        );
        high.cpu
            .mmu
            .store_doubleword(&mut bus, 0x01c0_0a00, 2, [0, 0x20])
            .unwrap();

        low.reset();
        assert_eq!(
            high.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x2222_2222,
            "resetting MID 8 cleared MID 10's cached translation"
        );
        assert_eq!(
            high.cpu.mmu.load(&mut bus, 0x200, 4, Width::Word).unwrap(),
            0x100
        );
        assert_eq!(high.mxcc.borrow().read_u64(0x01c0_0a00), 0x20);

        configure_reference_translation(&mut low, &mut bus, 0);
        assert_eq!(
            low.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111
        );
        high.reset();
        assert_eq!(
            low.cpu.mmu.load(&mut bus, 0, 11, Width::Word).unwrap(),
            0x1111_1111,
            "resetting MID 10 cleared MID 8's cached translation"
        );
    }
}

#[test]
fn local_asi_and_remote_physical_mxcc_access_work_from_both_modules() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let (mut low, mut high, mut bus) = module_pair(profile);
        let (module0, module1) = modules();

        low.cpu
            .mmu
            .store(&mut bus, 0x01c0_0a04, 2, Width::Word, 0x10)
            .unwrap();
        assert_eq!(low.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);
        assert_eq!(high.mxcc.borrow().read_u64(0x01c0_0a00), 0);

        bus.write32(module1.mxcc_base + 0x0a04, 0x0c).unwrap();
        assert_eq!(high.mxcc.borrow().read_u64(0x01c0_0a00), 0x0c);
        assert_eq!(low.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);

        high.cpu
            .mmu
            .store(&mut bus, 0x01c0_0a04, 2, Width::Word, 0x20)
            .unwrap();
        assert_eq!(high.mxcc.borrow().read_u64(0x01c0_0a00), 0x20);
        assert_eq!(low.mxcc.borrow().read_u64(0x01c0_0a00), 0x10);

        bus.write32(module0.mxcc_base + 0x0a04, 0x04).unwrap();
        assert_eq!(low.mxcc.borrow().read_u64(0x01c0_0a00), 0x04);
        assert_eq!(high.mxcc.borrow().read_u64(0x01c0_0a00), 0x20);
    }
}
