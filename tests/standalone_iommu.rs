use helios::bus::Bus;
use helios::cpu::Cpu;
use helios::dma::{DmaError, DmaFault};
use helios::machine::{Machine, MachineProfile, ProcessorModuleState};
use helios::mmu::Width;
use helios::system_bus::SystemBus;
use helios::topology::ProcessorModule;

const PAGE_A: u32 = 0xff00_0000;
const PAGE_B: u32 = 0xff00_1000;

fn machine_for(profile: MachineProfile) -> Machine {
    Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap()
}

fn machine() -> Machine {
    machine_for(MachineProfile::Ss20)
}

fn io(m: &Machine) -> u64 {
    m.bus.layout().io_control
}

fn configure(m: &mut Machine) {
    let io = io(m);
    m.bus.write32(io, 1).unwrap();
    m.bus.write32(io + 4, 0x1000).unwrap();
    m.bus.write32(io + 0x1008, 1 << 20).unwrap(); // onboard master 5
    m.bus.write32(0x10000, 0x806).unwrap(); // A -> 0x8000, writable
    m.bus.write32(0x10004, 0x906).unwrap(); // B -> 0x9000, writable
    m.bus.write32(0x8000, 0x1111_1111).unwrap();
    m.bus.write32(0x9000, 0x2222_2222).unwrap();
    m.bus.write32(0xa000, 0xaaaa_aaaa).unwrap();
    m.bus.write32(0xb000, 0xbbbb_bbbb).unwrap();
}

fn cpu2() -> ProcessorModuleState {
    ProcessorModuleState::new(
        MachineProfile::Ss20,
        ProcessorModule::from_processor(2).unwrap(),
    )
    .unwrap()
}

#[test]
fn dma_uses_standalone_owner_without_stepping_or_mutating_either_cpu_mmu() {
    let mut m = machine();
    configure(&mut m);
    let mut cpu2 = cpu2();
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x7)
        .unwrap();
    cpu2.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    let cpu0_before = m.cpu.clone();
    let cpu2_before = cpu2.cpu.clone();

    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    m.dma_write(5, PAGE_A, Width::Word, 0x3333_3333).unwrap();
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0x3333_3333);
    assert_eq!(m.bus.read32(0x10000).unwrap(), 0x806);
    assert_eq!(m.cpu, cpu0_before);
    assert_eq!(cpu2.cpu, cpu2_before);
}

#[test]
fn processor_two_can_program_the_standalone_controller_without_a_scheduler() {
    let mut m = machine();
    let mut cpu2 = cpu2();
    // Reference-MMU ASI 0x2f maps VA 0xe000_0000 to PA 0xfe000_0000.
    cpu2.cpu
        .mmu
        .store(&mut m.bus, 0xe000_0000, 0x2f, Width::Word, 1)
        .unwrap();
    cpu2.cpu
        .mmu
        .store(&mut m.bus, 0xe000_1008, 0x2f, Width::Word, 1 << 20)
        .unwrap();
    assert_eq!(m.bus.read32(io(&m)).unwrap(), 0x1300_0001);
    assert_eq!(m.bus.read32(io(&m) + 0x1008).unwrap(), 0x0010_0001);
}

#[test]
fn page_flush_survives_cpu_accesses_and_global_flush_clears_the_rest() {
    let mut m = machine();
    configure(&mut m);
    let mut cpu2 = cpu2();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    assert_eq!(m.dma_read(5, PAGE_B, Width::Word), Ok(0x2222_2222));

    // Change both backing PTEs without an implicit invalidate.
    m.bus.write32(0x10000, 0xa06).unwrap();
    m.bus.write32(0x10004, 0xb06).unwrap();
    m.bus.write32(io(&m) + 0x18, PAGE_A).unwrap();

    // CPU 0 and CPU 2 accesses run between the flush write and the DMA. They
    // must neither consume the standalone flush nor change CPU-2's context.
    m.cpu
        .mmu
        .load(&mut m.bus, 0x4000, 0x20, Width::Word)
        .unwrap();
    cpu2.cpu
        .mmu
        .load(&mut m.bus, 0x4000, 0x20, Width::Word)
        .unwrap();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0xaaaa_aaaa));
    assert_eq!(m.dma_read(5, PAGE_B, Width::Word), Ok(0x2222_2222));

    m.bus.write32(io(&m) + 0x14, 1).unwrap();
    assert_eq!(m.dma_read(5, PAGE_B, Width::Word), Ok(0xbbbb_bbbb));
}

#[test]
fn cpu_local_flushes_do_not_clear_standalone_entries() {
    let mut m = machine();
    configure(&mut m);
    let mut cpu2 = cpu2();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    m.bus.write32(0x10000, 0xa06).unwrap();
    // CPU-0 and CPU-2 local flushes must not touch the standalone cache.
    m.cpu
        .mmu
        .store(&mut m.bus, 0x400, 3, Width::Word, 0)
        .unwrap();
    cpu2.cpu
        .mmu
        .store(&mut m.bus, 0x400, 3, Width::Word, 0)
        .unwrap();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    m.bus.write32(io(&m) + 0x14, 0).unwrap();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0xaaaa_aaaa));
}

#[test]
fn standalone_faults_widths_bypass_reset_and_bus_errors_are_explicit() {
    let mut m = machine();
    configure(&mut m);
    assert_eq!(m.dma_write(5, PAGE_A, Width::Word, 0), Ok(()));
    assert_eq!(
        m.dma_read(5, PAGE_A + 1, Width::Word),
        Err(DmaError::Fault {
            address: PAGE_A + 1,
            kind: DmaFault::Misaligned
        })
    );
    m.bus.write32(io(&m) + 0x18, PAGE_A).unwrap();
    m.bus.write32(0x10000, 0).unwrap();
    assert_eq!(
        m.dma_read(5, PAGE_A, Width::Word),
        Err(DmaError::Fault {
            address: PAGE_A,
            kind: DmaFault::InvalidPte
        })
    );
    m.bus.write32(0x10000, 0x902).unwrap();
    assert_eq!(
        m.dma_write(5, PAGE_A, Width::Word, 0),
        Err(DmaError::Fault {
            address: PAGE_A,
            kind: DmaFault::WriteProtected
        })
    );
    m.bus.write32(io(&m), 0).unwrap();
    assert!(matches!(
        m.dma_read(5, PAGE_A, Width::Word),
        Err(DmaError::Unsupported(_))
    ));

    m.bus.write32(io(&m), 1).unwrap();
    m.bus.write32(io(&m) + 0x1010, 1).unwrap(); // slot 0 bypass
    m.bus.write32(io(&m) + 0x1008, 1 << 16).unwrap();
    m.bus.write32(0x4000, 0xfeed_face).unwrap();
    assert_eq!(m.dma_read(0, 0x4000, Width::Word), Ok(0xfeed_face));

    m.reset();
    m.bus.write32(io(&m), 1).unwrap();
    m.bus.write32(io(&m) + 4, 0x1000).unwrap();
    m.bus.write32(io(&m) + 0x1008, 1 << 20).unwrap();
    m.bus.write32(0x10000, 0xa06).unwrap();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0xaaaa_aaaa));

    m.bus.write32(io(&m) + 0x14, 0).unwrap();
    m.bus.write32(0x10000, 0x0100_0006).unwrap();
    assert_eq!(
        m.dma_read(5, PAGE_A, Width::Word),
        Err(DmaError::Unsupported("non-DRAM DVMA target"))
    );
    m.bus.write32(io(&m) + 0x14, 0).unwrap();
    m.bus.write32(io(&m) + 4, 0x2000000).unwrap(); // PTE table above DRAM
    let result = m.dma_read(5, PAGE_A, Width::Word);
    assert!(
        matches!(
            result,
            Err(DmaError::Unsupported("I/O page table outside DRAM"))
        ),
        "{result:?}"
    );
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}

#[test]
fn axil_uses_the_same_standalone_owner() {
    let mut m = machine_for(MachineProfile::Axil320);
    configure(&mut m);
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    assert_eq!(m.dma_read(5, PAGE_B, Width::Word), Ok(0x2222_2222));
}

#[test]
fn table_base_mask_is_profile_specific() {
    let mut ss5 = Machine::with_profile(
        MachineProfile::Ss5,
        16 * 1024 * 1024,
        &0x0100_0000u32.to_be_bytes(),
        MachineProfile::Ss5.default_nvram(),
    )
    .unwrap();
    ss5.bus.write32(0x1000_0004, u32::MAX).unwrap();
    assert_eq!(ss5.bus.read32(0x1000_0004).unwrap(), 0x07ff_fc00);

    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = machine_for(profile);
        m.bus.write32(io(&m) + 4, u32::MAX).unwrap();
        assert_eq!(
            m.bus.read32(io(&m) + 4).unwrap(),
            0xffff_fc00,
            "{profile:?}"
        );
    }
}

#[test]
fn high_table_base_does_not_alias_a_low_ram_table() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = machine_for(profile);
        configure(&mut m);
        m.bus.write32(io(&m) + 4, 0x0800_1000).unwrap();
        let result = m.dma_write(5, PAGE_A, Width::Word, 0xdead_beef);
        assert_eq!(
            result,
            Err(DmaError::Unsupported("I/O page table outside DRAM")),
            "{profile:?}"
        );
        assert_eq!(m.bus.read32(0x8000).unwrap(), 0x1111_1111);
    }
}

#[test]
fn page_flush_compares_fa_bits_30_to_12() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        for flush in [0x7f00_05a5u32, 0xff00_05a5] {
            let mut m = machine_for(profile);
            configure(&mut m);
            assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
            assert_eq!(m.dma_read(5, PAGE_B, Width::Word), Ok(0x2222_2222));
            m.bus.write32(0x10000, 0xa06).unwrap();
            m.bus.write32(0x10004, 0xb06).unwrap();
            m.bus.write32(io(&m) + 0x18, flush).unwrap();
            assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0xaaaa_aaaa));
            assert_eq!(m.dma_read(5, PAGE_B, Width::Word), Ok(0x2222_2222));
        }
    }
}

#[test]
fn standalone_cache_has_sixteen_lru_entries() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = machine_for(profile);
        m.bus.write32(io(&m), 1).unwrap();
        m.bus.write32(io(&m) + 4, 0x1000).unwrap();
        m.bus.write32(io(&m) + 0x1008, 1 << 20).unwrap();
        for page in 0..17u32 {
            let physical = 0x80000 + page * 0x1000;
            m.bus
                .write32(0x10000 + u64::from(page * 4), (physical >> 4) | 6)
                .unwrap();
            m.bus.write32(u64::from(physical), page).unwrap();
        }
        for page in 0..16u32 {
            assert_eq!(
                m.dma_read(5, 0xff00_0000 + page * 0x1000, Width::Word),
                Ok(page)
            );
        }
        m.bus.write32(0x10000, 0xb006).unwrap();
        m.bus.write32(0x10004, 0xb106).unwrap();
        m.bus.write32(0xb0000, 0xaaaa).unwrap();
        m.bus.write32(0xb1000, 0xbbbb).unwrap();
        assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0));
        assert_eq!(
            m.dma_read(5, 0xff00_0000 + 16 * 0x1000, Width::Word),
            Ok(16)
        );
        assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0));
        assert_eq!(m.dma_read(5, 0xff00_1000, Width::Word), Ok(0xbbbb));
    }
}

fn cpu_mapping(cpu: &mut Cpu, bus: &mut SystemBus, base: u64, context: u32, physical: u32) {
    bus.write32(
        base + u64::from(context) * 4,
        ((base + 0x1000) >> 4) as u32 | 1,
    )
    .unwrap();
    bus.write32(base + 0x1000, ((base + 0x2000) >> 4) as u32 | 1)
        .unwrap();
    bus.write32(base + 0x2000, ((base + 0x3000) >> 4) as u32 | 1)
        .unwrap();
    bus.write32(base + 0x3010, (physical >> 4) | 0x8e).unwrap();
    cpu.mmu
        .store(bus, 0x100, 4, Width::Word, (base >> 4) as u32)
        .unwrap();
    cpu.mmu.store(bus, 0x200, 4, Width::Word, context).unwrap();
    cpu.mmu.store(bus, 0, 4, Width::Word, 1).unwrap();
}

#[test]
fn populated_cpu_translation_flushes_remain_independent_of_standalone_io() {
    let mut m = machine();
    configure(&mut m);
    let mut second = cpu2();
    for (address, value) in [
        (0x20000, 0x1111),
        (0x21000, 0x2222),
        (0x22000, 0x3333),
        (0x23000, 0x4444),
    ] {
        m.bus.write32(address, value).unwrap();
    }
    cpu_mapping(&mut m.cpu, &mut m.bus, 0x100000, 0, 0x20000);
    cpu_mapping(&mut second.cpu, &mut m.bus, 0x110000, 0x100, 0x21000);
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x4000, 11, Width::Word).unwrap(),
        0x1111
    );
    assert_eq!(
        second
            .cpu
            .mmu
            .load(&mut m.bus, 0x4000, 11, Width::Word)
            .unwrap(),
        0x2222
    );
    let first_before = m.cpu.clone();
    let second_before = second.cpu.clone();
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    assert_eq!(m.cpu, first_before);
    assert_eq!(second.cpu, second_before);

    m.bus.write32(0x103010, 0x228e).unwrap();
    m.bus.write32(0x113010, 0x238e).unwrap();
    m.bus.write32(0x10000, 0xa06).unwrap();
    second
        .cpu
        .mmu
        .store(&mut m.bus, 0xe000_0018, 0x2f, Width::Word, PAGE_A)
        .unwrap();
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x4000, 11, Width::Word).unwrap(),
        0x1111
    );
    assert_eq!(
        second
            .cpu
            .mmu
            .load(&mut m.bus, 0x4000, 11, Width::Word)
            .unwrap(),
        0x2222
    );
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0xaaaa_aaaa));
}

#[test]
fn standalone_dma_attributes_do_not_inherit_cpu0_state() {
    let mut m = machine();
    configure(&mut m);
    assert_ne!(m.cpu.psr() & 0x80, 0);
    assert!(m.cpu.mmu.boot_mode());
    assert_eq!(m.dma_read(5, PAGE_A, Width::Word), Ok(0x1111_1111));
    let attributes = m.bus.read32(io(&m) + 0x1050).unwrap();
    assert_eq!(
        attributes & ((1 << 24) | (1 << 23) | (1 << 12) | (1 << 11)),
        0
    );
}
