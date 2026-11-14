use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};

const MBUS: [MachineProfile; 2] = [MachineProfile::Ss20, MachineProfile::Axil320];
const NOP: u32 = 0x0100_0000;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn machine(profile: MachineProfile, count: usize, prom: &[u32]) -> Machine {
    Machine::with_profile_processors(
        profile,
        64 * 1024 * 1024,
        &bytes(prom),
        profile.default_nvram(),
        count,
    )
    .unwrap()
}

#[test]
fn host_and_guest_aer_regrant_preserve_an_outstanding_cpu_wait() {
    for profile in MBUS {
        for guest_regrant in [false, true] {
            let mut m = machine(profile, 2, &[NOP]);
            let program = if guest_regrant {
                // nop; sta g2,[g1] 0x2f; sta g3,[g1] 0x2f; ba .; nop
                vec![NOP, 0xc4a0_45e0, 0xc6a0_45e0, 0x1080_0000, NOP]
            } else {
                vec![0x1080_0000, NOP]
            };
            m.load_image(0x1000, &bytes(&program)).unwrap();
            // Controlled wait injection, not a claimed MBus PROM alias.
            m.bus
                .map_rom(0x7000_0000, &bytes(&[0x1080_0000, NOP]))
                .unwrap();
            assert!(m.set_processor_entry(0, 0x1000, 0x1004));
            assert!(m.set_processor_entry(2, 0x7000_0000, 0x7000_0004));
            m.cpu.write_reg(1, 0xe000_1008);
            m.cpu.write_reg(2, 0);
            m.cpu.write_reg(3, 4);
            assert!(m.set_processor_arbitration(2, true));
            assert_eq!(m.step_scheduled().unwrap().processor, 0);
            assert_eq!(m.step_scheduled().unwrap().processor, 2);
            assert_eq!(m.scheduler_stats().virtual_ns, 100);
            if !guest_regrant {
                assert!(m.set_processor_arbitration(2, false));
                assert!(m.set_processor_arbitration(2, true));
            }
            // CPU 2's outstanding read takes over 1 us. Revoking/restoring
            // its grant must not permit a second instruction before then.
            for _ in 0..9 {
                let event = m.step_scheduled().unwrap();
                assert_eq!(event.processor, 0, "{profile:?} guest={guest_regrant}");
            }
            assert_eq!(m.scheduler_stats().virtual_ns, 1000);
            assert_eq!(m.scheduler_stats().processor2.instructions, 1);
            assert!(m.processor_arbitration(2).is_enabled());
            assert!((0..6).any(|_| m.step_scheduled().unwrap().processor == 2));
        }
    }
}

#[test]
fn reset_store_time_reaches_retained_rtc_and_reset_is_processor_tagged() {
    for profile in [
        MachineProfile::Ss5,
        MachineProfile::Ss20,
        MachineProfile::Axil320,
    ] {
        for processor in [0, 2] {
            if processor == 2 && profile == MachineProfile::Ss5 {
                continue;
            }
            for scheduled in [false, true] {
                if processor == 2 && !scheduled {
                    continue;
                }
                for wait in [false, true] {
                    let store = if profile == MachineProfile::Ss5 {
                        0xc4a0_4400 // sta g2,[g1] ASI 0x20
                    } else {
                        0xc4a0_45e0 // sta g2,[g1] ASI 0x2f
                    };
                    let mut m = machine(profile, if processor == 0 { 1 } else { 2 }, &[store]);
                    m.load_image(0x1000, &bytes(&[0x1080_0000, NOP])).unwrap();
                    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
                    let entry = if wait {
                        if profile != MachineProfile::Ss5 {
                            m.bus.map_rom(0x7000_0000, &bytes(&[store])).unwrap();
                        }
                        0x7000_0000
                    } else {
                        m.load_image(0x3000, &bytes(&[store])).unwrap();
                        0x3000
                    };
                    assert!(m.set_processor_entry(processor, entry, entry + 4));
                    let reset_address = m.bus.layout().reset as u32;
                    let cpu = m.processor_cpu_mut(processor).unwrap();
                    cpu.write_reg(1, reset_address);
                    cpu.write_reg(2, 1);
                    if processor == 2 {
                        assert!(m.set_processor_arbitration(2, true));
                    }
                    // Normal time crosses the second with 100 ns; the wait
                    // case requires the additional PROM latency to cross it.
                    m.advance_ns(if wait { 999_999_500 } else { 999_999_950 });
                    if processor == 2 {
                        assert_eq!(m.step_scheduled().unwrap().processor, 0);
                    }
                    let step = if scheduled {
                        let event = m.step_scheduled().unwrap();
                        assert_eq!(event.processor, processor);
                        assert_eq!(event.virtual_ns, 0);
                        assert_eq!(m.scheduler_stats().total_instructions(), 1);
                        event.step
                    } else {
                        m.step().unwrap()
                    };
                    assert!(
                        matches!(step, Step::SoftwareReset { instruction: Some(word), .. } if word == store)
                    );
                    assert_eq!(
                        m.bus.read8(m.bus.layout().nvram + 0x1ff9).unwrap(),
                        1,
                        "{profile:?} cpu={processor} scheduled={scheduled} wait={wait}"
                    );
                    assert_eq!(m.bus.read32(m.bus.system_timer_base() + 4).unwrap(), 0);
                    assert_eq!(m.processor_cpu(processor).unwrap().pc(), 0);
                }
            }
        }
    }
}

#[test]
fn scheduled_contexts_are_distinct_and_cannot_consume_a_board_dma_flush() {
    // Each CPU repeatedly loads the same VA under a different context.
    let program = bytes(&[0x0300_0020, 0xc400_6000, 0x10bf_ffff, NOP]);
    for profile in MBUS {
        let mut m = machine(profile, 2, &[NOP]);
        m.bus.write32(0x1000, 0x201).unwrap(); // context 0 -> L1 at 0x2000
        m.bus.write32(0x1004, 0x301).unwrap(); // context 1 -> L1 at 0x3000
        m.bus.write32(0x2000, 0x0010_008e).unwrap(); // VA region 0 -> 16 MiB
        m.bus.write32(0x3000, 0x0020_008e).unwrap(); // VA region 0 -> 32 MiB
        for (processor, physical) in [(0, 0x0100_0000), (2, 0x0200_0000)] {
            m.load_image(physical + 0x1000, &program).unwrap();
            m.bus
                .write32(
                    u64::from(physical) + 0x8000,
                    0xa000_0000 + u32::from(processor),
                )
                .unwrap();
            assert!(m.set_processor_entry(processor, 0x1000, 0x1004));
        }
        m.cpu
            .mmu
            .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
            .unwrap();
        m.cpu
            .mmu
            .store(&mut m.bus, 0x200, 4, Width::Word, 0)
            .unwrap();
        m.cpu
            .mmu
            .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
            .unwrap();
        let (secondary, bus) = m.secondary_cpu_with_bus_mut().unwrap();
        secondary
            .cpu
            .mmu
            .store(bus, 0x100, 4, Width::Word, 0x100)
            .unwrap();
        secondary
            .cpu
            .mmu
            .store(bus, 0x200, 4, Width::Word, 1)
            .unwrap();
        secondary
            .cpu
            .mmu
            .store(bus, 0, 4, Width::Word, PCR_EN)
            .unwrap();

        let io = m.bus.layout().io_control;
        m.bus.write32(io, 1).unwrap();
        m.bus.write32(io + 4, 0x1000).unwrap();
        m.bus.write32(io + 0x1008, (1 << 20) | 4).unwrap();
        m.bus.write32(0x10000, 0x806).unwrap();
        m.bus.write32(0x8000, 0x1111_1111).unwrap();
        m.bus.write32(0x9000, 0x2222_2222).unwrap();
        assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x1111_1111));
        m.bus.write32(0x10000, 0x906).unwrap();
        m.bus.write32(io + 0x18, 0xff00_0000).unwrap();

        let context = m.bus.access_context();
        m.run_scheduled(100).unwrap();
        assert_eq!(m.scheduler_stats().processor0.instructions, 50);
        assert_eq!(m.scheduler_stats().processor2.instructions, 50);
        assert_eq!(m.cpu.read_reg(2), 0xa000_0000);
        assert_eq!(m.processor_cpu(2).unwrap().read_reg(2), 0xa000_0002);
        assert_eq!(m.bus.access_context(), context);
        let mmu0 = m.cpu.mmu.clone();
        let mmu2 = m.processor_cpu(2).unwrap().mmu.clone();
        assert_eq!(m.dma_read(5, 0xff00_0000, Width::Word), Ok(0x2222_2222));
        assert_eq!(m.cpu.mmu, mmu0);
        assert_eq!(m.processor_cpu(2).unwrap().mmu, mmu2);
        assert_eq!(m.bus.access_context(), context);
    }
}
