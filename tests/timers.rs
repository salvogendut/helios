use helios::bus::{Bus, BusErrorKind, MapError};
use helios::cpu::{Step, PSR_ET};
use helios::devices::interrupt::MASK_ALL;
use helios::devices::timer::Timers;
use helios::machine::{timer_demo_image, Machine};
use helios::mmu::{FaultType, MemoryError, Srmmu, Width, PCR_EN, PCR_NF};
use helios::system_bus::{
    CPU_INTERRUPT_BASE, CPU_TIMER_BASE, SYSTEM_INTERRUPT_BASE, SYSTEM_TIMER_BASE,
};

#[test]
fn counters_tick_every_500ns_and_limit_read_acknowledges_only_its_timer() {
    let mut t = Timers::default();
    assert_eq!(t.read(true, 4).unwrap(), 0);
    assert_eq!(t.read(false, 12).unwrap(), 0);
    t.write(true, 0, 4 << 9).unwrap();
    t.write(false, 0, 3 << 9).unwrap();
    t.advance_ns(499);
    assert_eq!(t.read(true, 4).unwrap(), 512);
    t.advance_ns(1);
    assert_eq!(t.read(true, 4).unwrap(), 1024);
    t.advance_ns(500);
    assert!(t.processor_irq());
    assert!(!t.system_irq());
    t.advance_ns(500);
    assert_eq!(t.read(true, 4).unwrap(), 0x8000_0200);
    assert_eq!(t.read(true, 4).unwrap(), 0x8000_0200); // non-destructive
    assert_eq!(t.read(true, 0).unwrap(), 2048);
    assert!(!t.system_irq());
    assert!(t.processor_irq());
    assert_eq!(t.read(false, 0).unwrap(), 1536);
    assert!(!t.processor_irq());
    t.advance_ns(1500);
    assert!(t.system_irq()); // repeats; acknowledgement did not restart it
}

#[test]
fn zero_limit_overflow_and_limit_one_wait_for_incremented_comparison() {
    let mut t = Timers::default();
    t.advance_ns(((1 << 22) - 1) * 500);
    assert_eq!(t.read(true, 4).unwrap(), 0x7fff_fe00);
    assert!(!t.system_irq());
    t.advance_ns(500);
    assert_eq!(t.read(true, 4).unwrap(), 0x8000_0200);
    assert_eq!(t.read(true, 0).unwrap(), 0);
    t.write(true, 0, 512).unwrap();
    t.advance_ns(((1 << 22) - 1) * 500);
    assert_eq!(t.read(true, 4).unwrap(), 0);
    assert!(!t.system_irq());
    t.advance_ns(500);
    assert_eq!(t.read(true, 4).unwrap(), 0x8000_0200);
}

#[test]
fn nonresetting_limit_behind_count_waits_for_wrap_and_retains_pending() {
    let mut t = Timers::default();
    t.advance_ns(10 * 500);
    t.write(true, 8, 5 << 9).unwrap();
    assert_eq!(t.read(true, 4).unwrap(), 10 << 9);
    t.advance_ns(((1 << 22) - 10 + 4) * 500);
    assert_eq!(t.read(true, 4).unwrap(), 4 << 9);
    assert!(!t.system_irq());
    t.advance_ns(500);
    assert_eq!(t.read(true, 4).unwrap(), 0x8000_0200);
    t.write(true, 8, 0).unwrap();
    assert!(t.system_irq());
    t.write(true, 0, 0).unwrap();
    assert!(!t.system_irq());
    assert_eq!(t.read(true, 4).unwrap(), 512);
}

#[test]
fn batched_time_matches_a_scalar_counter_oracle_at_match_and_wrap_boundaries() {
    for start in [0u32, 1, 8, (1 << 22) - 3] {
        for limit in [0u32, 1, 2, 7, 19, (1 << 22) - 1] {
            let mut t = Timers::default();
            t.advance_ns(u64::from(start) * 500);
            t.write(true, 8, limit << 9).unwrap();
            let mut count = start;
            let mut pending = false;
            for _ in 0..537 {
                count = (count + 1) & ((1 << 22) - 1);
                if count == limit {
                    count = 1;
                    pending = true;
                }
            }
            t.advance_ns(537 * 500);
            assert_eq!(
                t.read(true, 4).unwrap(),
                count << 9 | if pending { 1 << 31 } else { 0 },
                "start={start}, limit={limit}"
            );
        }
    }
}

#[test]
fn huge_time_advances_are_bounded_and_partition_independent() {
    let mut one = Timers::default();
    one.write(true, 0, 19 << 9).unwrap();
    one.write(false, 0, 0x7fff_fe00).unwrap();
    one.advance_ns(123);
    let mut split = one.clone();
    one.advance_ns(u64::MAX);
    split.advance_ns(u64::MAX / 2);
    split.advance_ns(u64::MAX - u64::MAX / 2);
    assert_eq!(one, split);
    one.advance_seconds(u64::MAX);
    split.advance_seconds(u64::MAX / 2);
    split.advance_seconds(u64::MAX - u64::MAX / 2);
    assert_eq!(one, split);
    assert!(one.system_irq() && one.processor_irq());
}

#[test]
fn reserved_bits_readonly_fields_and_run_writes_preserve_counter_state() {
    let mut t = Timers::default();
    t.write(true, 0, u32::MAX).unwrap();
    assert_eq!(t.read(true, 0).unwrap(), 0x7fff_fe00);
    t.write(true, 4, 0).unwrap();
    assert_eq!(t.read(true, 4).unwrap(), 512);
    t.write(false, 12, u32::MAX).unwrap();
    assert_eq!(t.read(false, 12).unwrap(), 1);
    t.advance_ns(500);
    assert_eq!(t.read(false, 4).unwrap(), 512); // Run never gates counter mode
    t.write(false, 12, 0).unwrap();
    t.advance_ns(500);
    assert_eq!(t.read(false, 4).unwrap(), 1024);
    let before = t.clone();
    t.write(true, 16, !1).unwrap();
    assert_eq!(t, before);
    assert_eq!(t.read(true, 16).unwrap(), 0);
}

#[test]
fn mmio_rejects_unsupported_widths_before_acknowledging_a_timer() {
    let mut m = Machine::sun4m(8192, &0x0100_0000u32.to_be_bytes()).unwrap();
    m.bus.write32(SYSTEM_TIMER_BASE, 2 << 9).unwrap();
    m.bus
        .write32(SYSTEM_INTERRUPT_BASE + 8, MASK_ALL | 1 << 19)
        .unwrap();
    m.advance_ns(500);
    assert_eq!(m.bus.interrupt_levels(), 1 << 10);
    assert!(m
        .bus
        .read8(SYSTEM_TIMER_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .read16(SYSTEM_TIMER_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .read_doubleword(SYSTEM_TIMER_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .swap32(SYSTEM_TIMER_BASE, 0)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .ldstub8(SYSTEM_TIMER_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .write32_masked(SYSTEM_TIMER_BASE, 0, 0xff)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert_eq!(m.bus.interrupt_levels(), 1 << 10);
    assert_eq!(m.bus.read32(SYSTEM_INTERRUPT_BASE).unwrap(), 1 << 19);
    assert_eq!(m.bus.read32(CPU_INTERRUPT_BASE).unwrap(), 1 << 10);
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE).unwrap(), 2 << 9);
    assert_eq!(m.bus.read32(SYSTEM_INTERRUPT_BASE).unwrap(), 0);
    assert_eq!(m.bus.interrupt_levels(), 0);
    for base in [
        CPU_INTERRUPT_BASE,
        SYSTEM_INTERRUPT_BASE,
        CPU_TIMER_BASE,
        SYSTEM_TIMER_BASE,
    ] {
        assert_eq!(m.bus.map_ram(base, 4), Err(MapError::Overlap));
        assert!(m
            .bus
            .load_image(base, &[0; 4])
            .unwrap_err()
            .kind
            .is_host_stop());
        assert_eq!(
            m.bus.read32(base + 1).unwrap_err().kind,
            BusErrorKind::Misaligned
        );
    }
}

#[test]
fn machine_time_advances_on_success_and_retains_rtc_fraction_across_reset() {
    let mut m = Machine::sun4m(8192, &0x3080_0000u32.to_be_bytes()).unwrap(); // ba,a .
    for _ in 0..4 {
        m.step().unwrap();
    }
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 4).unwrap(), 0);
    m.step().unwrap();
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 4).unwrap(), 512);
    m.advance_ns(499_999_500);
    m.reset();
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 4).unwrap(), 0);
    assert_eq!(m.bus.interrupt_levels(), 0);
    m.advance_ns(500_000_000);
    assert_eq!(m.bus.devices().unwrap().nvram.read(0x1ff9).unwrap(), 1);

    let mut stopped = Machine::sun4m(8192, &0xffff_ffffu32.to_be_bytes()).unwrap();
    let before = stopped.bus.devices().unwrap().timers.clone();
    assert!(stopped.step().is_err());
    assert!(stopped.step().is_err());
    assert_eq!(stopped.bus.devices().unwrap().timers, before);
}

#[test]
fn reset_prom_program_handles_both_timers_and_resumes_work() {
    let mut m = Machine::sun4m(8192, &timer_demo_image()).unwrap();
    let mut levels = Vec::new();
    for _ in 0..256 {
        if let Step::InterruptTaken { level, pc, npc, .. } = m.step().unwrap() {
            levels.push(level);
            assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (pc, npc));
        }
    }
    assert_eq!(levels, [10, 14]);
    assert_eq!((m.cpu.read_reg(5), m.cpu.read_reg(7)), (1, 1));
    assert!(m.cpu.read_reg(3) > 2);
    assert_eq!(m.cpu.current_window(), 0);
    assert_ne!(m.cpu.psr() & PSR_ET, 0);
    assert_eq!(m.bus.read32(SYSTEM_INTERRUPT_BASE).unwrap(), 0);
    assert_eq!(m.bus.read32(CPU_INTERRUPT_BASE).unwrap(), 0);
}

#[test]
fn translated_timer_acknowledgement_happens_only_after_permission_checks() {
    let mut m = Machine::sun4m(0x5000, &0x0100_0000u32.to_be_bytes()).unwrap();
    // User VA 0x2000 -> timer page, supervisor read/execute only (ACC=6).
    for (address, word) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4008, 0x071d_101a),
    ] {
        m.bus.write32(address, word).unwrap();
    }
    m.bus.write32(SYSTEM_TIMER_BASE, 1024).unwrap();
    m.advance_ns(500);
    let mut mmu = Srmmu::default();
    mmu.store(&mut m.bus, 0x100, 4, Width::Word, 0x100).unwrap();
    mmu.store(&mut m.bus, 0, 4, Width::Word, PCR_EN).unwrap();
    assert!(matches!(mmu.load(&mut m.bus, 0x2000, 10, Width::Word),
        Err(MemoryError::Translation(fault)) if fault.kind == FaultType::Privilege));
    assert!(m.bus.devices().unwrap().timers.system_irq());
    mmu.store(&mut m.bus, 0, 4, Width::Word, PCR_EN | PCR_NF)
        .unwrap();
    assert_eq!(mmu.load(&mut m.bus, 0x2000, 10, Width::Word).unwrap(), 0);
    assert!(m.bus.devices().unwrap().timers.system_irq());
    assert_eq!(mmu.load(&mut m.bus, 0x2000, 11, Width::Word).unwrap(), 1024);
    assert!(!m.bus.devices().unwrap().timers.system_irq());
}

#[test]
fn machine_delivers_and_guest_acknowledges_a_software_interrupt() {
    let mut prom = vec![0; 0x170];
    for (address, word) in [
        (0, 0x8188_20a0u32), // wr g0, 0xa0, psr: enable traps
        (4, 0x0100_0000),
        (0x160, 0xc420_6004), // st g2, [g1+4]: clear soft 6
        (0x164, 0x81c4_6000), // jmpl l1, g0
        (0x168, 0x81cc_a000), // rett l2
    ] {
        prom[address..address + 4].copy_from_slice(&word.to_be_bytes());
    }
    let mut m = Machine::sun4m(8192, &prom).unwrap();
    m.cpu.set_window_invalid_mask(0);
    m.cpu.write_reg(1, CPU_INTERRUPT_BASE as u32);
    m.cpu.write_reg(2, 1 << 22);
    m.bus.write32(SYSTEM_INTERRUPT_BASE + 8, MASK_ALL).unwrap();
    m.bus.write32(CPU_INTERRUPT_BASE + 8, 1 << 22).unwrap();
    assert!(matches!(m.step().unwrap(), Step::Executed { pc: 0, .. }));
    assert_eq!(
        m.step().unwrap(),
        Step::InterruptTaken {
            pc: 4,
            npc: 8,
            level: 6,
            vector: 0x160
        }
    );
    m.step().unwrap();
    assert_eq!(m.bus.read32(CPU_INTERRUPT_BASE).unwrap(), 0);
    for _ in 0..3 {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
    assert_eq!(m.cpu.pc(), 8);
    assert_eq!(m.cpu.current_window(), 0);
}
