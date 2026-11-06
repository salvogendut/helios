use helios::bus::{Bus, BusErrorKind};
use helios::cpu::Step;
use helios::devices::timer::Timers;
use helios::machine::Machine;
use helios::mmu::{FaultType, MemoryError, Srmmu, Width, PCR_EN, PCR_NF};
use helios::system_bus::{
    CPU_INTERRUPT_BASE, CPU_TIMER_BASE, RESET_BASE, SYSTEM_INTERRUPT_BASE, SYSTEM_TIMER_BASE,
};

fn user_timer() -> Timers {
    let mut timer = Timers::default();
    timer.write(true, 16, 1).unwrap();
    timer
}

fn machine() -> Machine {
    Machine::sun4m(0x5000, &0x3080_0000u32.to_be_bytes()).unwrap() // ba,a .
}

#[test]
fn user_count_has_54_bits_and_overflow_is_sticky_until_either_half_is_written() {
    let mut t = user_timer();
    t.write(false, 12, 1).unwrap();
    t.write_user_doubleword([0x1234_5678, 0x7fff_ffff]).unwrap();
    assert_eq!(
        t.read_user_doubleword().unwrap(),
        [0x1234_5678, 0x7fff_fe00]
    );
    t.advance_ns(500);
    assert_eq!(
        t.read_user_doubleword().unwrap(),
        [0x1234_5678, 0x8000_0000]
    );
    // LSW bit 31 is count data, unlike the counter-mode pending flag.
    assert!(!t.processor_irq());
    t.write(false, 4, 0xffff_ffff).unwrap();
    t.advance_ns(500);
    assert_eq!(t.read_user_doubleword().unwrap(), [0x1234_5679, 0]);

    t.write_user_doubleword([u32::MAX, u32::MAX]).unwrap();
    assert_eq!(
        t.read_user_doubleword().unwrap(),
        [0x7fff_ffff, 0xffff_fe00]
    );
    t.advance_ns(499);
    assert_eq!(t.read(false, 0).unwrap(), 0x7fff_ffff);
    t.advance_ns(1);
    assert_eq!(t.read_user_doubleword().unwrap(), [0x8000_0000, 0]);
    t.advance_ns(1500);
    assert_eq!(t.read_user_doubleword().unwrap(), [0x8000_0000, 0x600]);
    assert_eq!(t.read(false, 0).unwrap(), 0x8000_0000); // reads do not acknowledge
    assert_eq!(t.read(false, 4).unwrap(), 0x600);
    assert!(!t.processor_irq());
    t.write(false, 0, 0x8000_0005).unwrap();
    assert_eq!(t.read_user_doubleword().unwrap(), [5, 0x600]);
    t.write_user_doubleword([0x7fff_ffff, 0xffff_fe00]).unwrap();
    t.advance_ns(500);
    t.write(false, 4, 0x8ff).unwrap();
    assert_eq!(t.read_user_doubleword().unwrap(), [0, 0x800]);
}

#[test]
fn run_gates_user_ticks_only_and_preserves_the_shared_500ns_phase() {
    let mut t = user_timer();
    assert_eq!(t.read(false, 12).unwrap(), 0);
    t.advance_ns(499);
    assert_eq!(t.read_user_doubleword().unwrap(), [0, 0]);
    t.write(false, 12, u32::MAX).unwrap();
    assert_eq!(t.read(false, 12).unwrap(), 1);
    t.advance_ns(1);
    assert_eq!(t.read_user_doubleword().unwrap(), [0, 0x200]);
    t.write(false, 12, 0).unwrap();
    t.advance_ns(1001);
    assert_eq!(t.read_user_doubleword().unwrap(), [0, 0x200]);
    assert_eq!(t.read(true, 4).unwrap(), 0x600);
    t.write(false, 12, 1).unwrap();
    t.advance_ns(499);
    assert_eq!(t.read_user_doubleword().unwrap(), [0, 0x400]);
    t.write(false, 12, 0).unwrap();
    t.write_user_doubleword([1, 0x1000]).unwrap();
    t.advance_seconds(5);
    assert_eq!(t.read_user_doubleword().unwrap(), [1, 0x1000]);
    assert!(t.system_irq()); // independent system counter continued
}

#[test]
fn large_user_time_advances_cross_multiple_wraps_without_iteration_or_lost_fraction() {
    let mut t = user_timer();
    t.write(false, 12, 1).unwrap();
    t.advance_ns(123);
    let mut split = t.clone();
    t.advance_ns(u64::MAX);
    split.advance_ns(u64::MAX / 2);
    split.advance_ns(u64::MAX - u64::MAX / 2);
    assert_eq!(t, split);
    // Integer reference in 500 ns tick units, including the initial fraction.
    let ns_ticks = (u128::from(u64::MAX) + 123) / 500;
    let count_mask = (1u128 << 54) - 1;
    let expected = ((ns_ticks & count_mask) << 9) | (1u128 << 63);
    assert_eq!(
        t.read_user_doubleword().unwrap(),
        [(expected >> 32) as u32, expected as u32]
    );
    t.advance_seconds(u64::MAX);
    split.advance_seconds(u64::MAX / 2);
    split.advance_seconds(u64::MAX - u64::MAX / 2);
    assert_eq!(t, split);
    let ticks = ns_ticks + u128::from(u64::MAX) * 2_000_000;
    let expected = ((ticks & count_mask) << 9) | (1u128 << 63);
    assert_eq!(
        t.read_user_doubleword().unwrap(),
        [(expected >> 32) as u32, expected as u32]
    );
    assert!(!t.processor_irq());
}

#[test]
fn mode_switch_clears_old_processor_irq_and_leaves_system_timer_and_run_alone() {
    let mut m = machine();
    m.bus.write32(SYSTEM_INTERRUPT_BASE + 8, u32::MAX).unwrap();
    m.bus.write32(SYSTEM_TIMER_BASE, 1024).unwrap();
    m.bus.write32(CPU_TIMER_BASE, 1024).unwrap();
    m.bus.write32(CPU_TIMER_BASE + 12, 1).unwrap();
    m.advance_ns(500);
    assert_eq!(m.bus.interrupt_levels(), (1 << 10) | (1 << 14));
    m.bus.write32(SYSTEM_TIMER_BASE + 16, u32::MAX).unwrap();
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 16).unwrap(), 1);
    assert_eq!(m.bus.interrupt_levels(), 1 << 10);
    assert_eq!(m.bus.read32(CPU_INTERRUPT_BASE).unwrap() & (1 << 14), 0);
    assert_eq!(m.bus.read32(CPU_TIMER_BASE + 12).unwrap(), 1);
    assert_eq!(m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(), [0, 0]);
    m.bus
        .write_doubleword(CPU_TIMER_BASE, [0x7fff_ffff, 0xffff_fe00])
        .unwrap();
    m.advance_ns(500);
    assert_eq!(
        m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(),
        [0x8000_0000, 0]
    );
    assert_eq!(m.bus.interrupt_levels(), 1 << 10);
    // An identical configuration write must not clear overflow or restart count.
    m.bus.write32(SYSTEM_TIMER_BASE + 16, 1).unwrap();
    assert_eq!(
        m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(),
        [0x8000_0000, 0]
    );
    m.bus.write32(CPU_TIMER_BASE + 8, 1536).unwrap(); // counter-mode limit latch
    assert_eq!(m.bus.read32(CPU_TIMER_BASE + 8).unwrap(), 0);
    assert_eq!(
        m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(),
        [0x8000_0000, 0]
    );
    m.bus.write32(CPU_TIMER_BASE + 12, 0).unwrap();
    m.bus.write32(SYSTEM_TIMER_BASE + 16, !1).unwrap();
    assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 16).unwrap(), 0);
    assert_eq!(m.bus.read32(CPU_TIMER_BASE + 4).unwrap(), 0);
    assert_eq!(m.bus.read32(CPU_TIMER_BASE).unwrap(), 1536);
    m.advance_ns(1500);
    assert_eq!(m.bus.interrupt_levels(), (1 << 10) | (1 << 14)); // Run=0 doesn't gate counter
}

#[test]
fn doubleword_mmio_is_exclusive_to_user_count_and_invalid_accesses_are_atomic() {
    let mut m = machine();
    m.bus.write32(CPU_TIMER_BASE, 1024).unwrap();
    m.advance_ns(500);
    assert!(m
        .bus
        .read_doubleword(CPU_TIMER_BASE)
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m
        .bus
        .write_doubleword(CPU_TIMER_BASE, [0, 0])
        .unwrap_err()
        .kind
        .is_host_stop());
    assert!(m.bus.devices().unwrap().timers.processor_irq());
    m.bus.write32(SYSTEM_TIMER_BASE + 16, 1).unwrap();
    m.bus
        .write_doubleword(CPU_TIMER_BASE, [0x1234, 0xabcd_ef00])
        .unwrap();
    let before = m.bus.devices().unwrap().timers.clone();
    for error in [
        m.bus.read8(CPU_TIMER_BASE).unwrap_err(),
        m.bus.read16(CPU_TIMER_BASE).unwrap_err(),
        m.bus.write8(CPU_TIMER_BASE, 0).unwrap_err(),
        m.bus.write16(CPU_TIMER_BASE, 0).unwrap_err(),
        m.bus.write32_masked(CPU_TIMER_BASE, 0, 0xffff).unwrap_err(),
        m.bus.swap32(CPU_TIMER_BASE, 0).unwrap_err(),
        m.bus.ldstub8(CPU_TIMER_BASE).unwrap_err(),
        m.bus.read_doubleword(SYSTEM_TIMER_BASE).unwrap_err(),
        m.bus
            .write_doubleword(SYSTEM_TIMER_BASE, [0, 0])
            .unwrap_err(),
        m.bus.read_doubleword(CPU_TIMER_BASE + 8).unwrap_err(),
        m.bus
            .write_doubleword(CPU_TIMER_BASE + 8, [0, 1])
            .unwrap_err(),
    ] {
        assert!(error.kind.is_host_stop());
    }
    assert_eq!(
        m.bus.read_doubleword(CPU_TIMER_BASE + 4).unwrap_err().kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(
        m.bus
            .write_doubleword(CPU_TIMER_BASE + 4, [0, 0])
            .unwrap_err()
            .kind,
        BusErrorKind::Misaligned
    );
    assert_eq!(m.bus.devices().unwrap().timers, before);
    assert_eq!(
        m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(),
        [0x1234, 0xabcd_ee00]
    );
}

#[test]
fn guest_prom_configures_starts_reads_and_stops_user_time_across_low_word_carry() {
    let words = [
        0x031c_7400u32, // sethi CPU_TIMER_BASE, g1
        0x071c_7440,    // sethi SYSTEM_TIMER_BASE, g3
        0x8610_e010,    // or g3, 0x10, g3
        0x8410_2001,    // mov 1, g2
        0xc4a0_c400,    // sta g2, [g3] ASI20: user mode
        0xc8b8_4400,    // stda g4:g5, [g1] ASI20
        0xc420_600c,    // st g2, [g1+12]: Run=1
        0x0100_0000,
        0x0100_0000,
        0x0100_0000,
        0xcc98_4400, // ldda [g1] ASI20, g6:g7
        0xc020_600c, // st g0, [g1+12]: Run=0
        0xd098_4400, // ldda [g1] ASI20, o0:o1
        0x3080_0000, // ba,a .
    ];
    let prom: Vec<u8> = words.into_iter().flat_map(u32::to_be_bytes).collect();
    let mut m = Machine::sun4m(8192, &prom).unwrap();
    m.cpu.write_reg(4, 0x1234_5678);
    m.cpu.write_reg(5, 0xffff_fe00);
    for _ in 0..13 {
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    }
    assert_eq!((m.cpu.read_reg(6), m.cpu.read_reg(7)), (0x1234_5679, 0));
    assert_eq!((m.cpu.read_reg(8), m.cpu.read_reg(9)), (0x1234_5679, 0));
    m.advance_seconds(10);
    assert_eq!(
        m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(),
        [0x1234_5679, 0]
    );
    assert!(!m.bus.devices().unwrap().timers.processor_irq());
}

#[test]
fn user_mapping_allows_consistent_reads_but_protects_timer_from_writes() {
    let mut m = machine();
    for (address, word) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4008, 0x071d_0002), // VA 0x2000 -> CPU timer page, user/supervisor read-only
    ] {
        m.bus.write32(address, word).unwrap();
    }
    m.bus.write32(SYSTEM_TIMER_BASE + 16, 1).unwrap();
    m.bus
        .write_doubleword(CPU_TIMER_BASE, [0x7fff_ffff, 0xffff_fe00])
        .unwrap();
    m.bus.write32(CPU_TIMER_BASE + 12, 1).unwrap();
    m.advance_ns(500);
    let mut mmu = Srmmu::default();
    mmu.store(&mut m.bus, 0x100, 4, Width::Word, 0x100).unwrap();
    mmu.store(&mut m.bus, 0, 4, Width::Word, PCR_EN).unwrap();
    assert_eq!(
        mmu.load_doubleword(&mut m.bus, 0x2000, 10).unwrap(),
        [0x8000_0000, 0]
    );
    let before = m.bus.devices().unwrap().timers.clone();
    assert!(
        matches!(mmu.store_doubleword(&mut m.bus, 0x2000, 10, [0, 0]),
        Err(MemoryError::Translation(fault)) if fault.kind == FaultType::Protection)
    );
    mmu.store(&mut m.bus, 0, 4, Width::Word, PCR_EN | PCR_NF)
        .unwrap();
    mmu.store_doubleword(&mut m.bus, 0x2000, 10, [0, 0])
        .unwrap();
    assert_eq!(m.bus.devices().unwrap().timers, before);
}

#[test]
fn power_on_and_software_reset_clear_user_mode_run_count_and_overflow() {
    for software in [false, true] {
        let mut m = machine();
        m.bus.write32(SYSTEM_TIMER_BASE + 16, 1).unwrap();
        m.bus
            .write_doubleword(CPU_TIMER_BASE, [0x7fff_ffff, 0xffff_fe00])
            .unwrap();
        m.bus.write32(CPU_TIMER_BASE + 12, 1).unwrap();
        m.advance_ns(749);
        if software {
            m.bus.write32(RESET_BASE, 1).unwrap();
            assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
        } else {
            m.reset();
        }
        assert_eq!(m.bus.read32(SYSTEM_TIMER_BASE + 16).unwrap(), 0);
        assert_eq!(m.bus.read32(CPU_TIMER_BASE + 12).unwrap(), 0);
        assert_eq!(m.bus.read32(CPU_TIMER_BASE + 4).unwrap(), 0);
        m.bus.write32(SYSTEM_TIMER_BASE + 16, 1).unwrap();
        assert_eq!(m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(), [0, 0]);
        m.bus.write32(CPU_TIMER_BASE + 12, 1).unwrap();
        m.advance_ns(499);
        assert_eq!(m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(), [0, 0]);
        m.advance_ns(1);
        assert_eq!(m.bus.read_doubleword(CPU_TIMER_BASE).unwrap(), [0, 512]);
    }
}
