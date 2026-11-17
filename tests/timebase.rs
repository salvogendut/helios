use helios::bus::Bus;
use helios::devices::{keyboard::Keyboard, nvram::Nvram};
use helios::machine::{CpuClockMode, InstalledVideo, Machine, MachineProfile, Timebase};
use helios::system_bus::{SystemBus, SYSTEM_TIMER_BASE};

const LOOP: [u8; 8] = [0x10, 0x80, 0, 0, 1, 0, 0, 0]; // ba .; nop

fn timer_machine() -> Machine {
    let mut m = Machine::sun4m(0x20000, &[0x01, 0x00, 0x00, 0x00]).unwrap();
    m.load_image(0x1000, &LOOP).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    m.bus.write32(SYSTEM_TIMER_BASE, 4 << 9).unwrap();
    m
}

fn assert_query_is_read_only(m: &Machine, label: &str) -> Timebase {
    let before = m.diagnostic_snapshot();
    let timebase = m.timebase();
    assert_eq!(
        before.differences(&m.diagnostic_snapshot()),
        Vec::<&'static str>::new(),
        "{label}"
    );
    assert_eq!(timebase.now_ns, m.virtual_ns());
    timebase
}

#[test]
fn timer_deadline_includes_deferred_cpu_time_and_stops_at_the_event() {
    let mut m = timer_machine();
    // A 100 ns step leaves the CPU fast-path time deferred. The timer count
    // must still be reported from its unapplied state plus the pending delta.
    let _ = m.step().unwrap();
    let timebase = assert_query_is_read_only(&m, "deferred query");
    assert_eq!(timebase.now_ns, 100);
    assert_eq!(timebase.deferred_ns, 100);
    assert_eq!(timebase.next_device_ns, Some(1_500));
    assert_eq!(timebase.next_ns, Some(1_500));

    // Explicit advancement applies deferred time first, then the new interval.
    m.advance_ns(1_400);
    assert_eq!(m.virtual_ns(), 1_500);
    assert_eq!(m.timebase().deferred_ns, 0);
    assert!(m.bus.devices().unwrap().timers.system_irq());
    assert_eq!(m.timebase().next_device_ns, Some(1_500));

    // The event remains pending until the guest acknowledges it.
    assert!(m.bus.devices().unwrap().timers.system_irq());
}

#[test]
fn explicit_advance_flushes_pending_fast_path_time_before_new_time() {
    let mut m = timer_machine();
    for _ in 0..3 {
        let _ = m.step().unwrap();
    }
    assert_eq!(m.virtual_ns(), 300);
    assert_eq!(m.timebase().deferred_ns, 300);
    m.advance_ns(1);
    assert_eq!(m.virtual_ns(), 301);
    assert_eq!(m.timebase().deferred_ns, 0);
    assert!(!m.bus.devices().unwrap().timers.processor_irq());
    assert_eq!(m.bus.read32(m.bus.system_timer_base() + 4).unwrap(), 512);
}

#[test]
fn scheduled_path_exposes_one_absolute_timebase_with_fractional_phases() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut m = Machine::with_profile_processors_video(
            profile,
            16 * 1024 * 1024,
            &LOOP,
            profile.default_nvram(),
            2,
            InstalledVideo::None,
        )
        .unwrap();
        m.reset_with_clock_mode(CpuClockMode::Hardware);
        m.load_image(0x1000, &LOOP).unwrap();
        m.load_image(0x2000, &LOOP).unwrap();
        assert!(m.set_processor_entry(0, 0x1000, 0x1004));
        assert!(m.set_processor_entry(2, 0x2000, 0x2004));
        assert!(m.set_processor_arbitration(2, true));
        for _ in 0..1_500u64 {
            let first = m.step_scheduled().unwrap();
            let timebase = m.timebase();
            assert_eq!(timebase.now_ns, m.scheduler_stats().virtual_ns);
            assert_eq!(first.virtual_ns, timebase.now_ns);
            let second = m.step_scheduled().unwrap();
            let timebase = m.timebase();
            assert_eq!(timebase.now_ns, m.scheduler_stats().virtual_ns);
            assert_eq!(second.virtual_ns, timebase.now_ns);
        }
        assert_eq!(m.virtual_ns(), 20_000);
        assert!(m.timebase().next_cpu_ready_ns.is_some());
        assert_query_is_read_only(&m, "scheduled final query");
    }
}

#[test]
fn direct_path_reports_logical_time_without_a_scheduler_deadline() {
    let mut m = timer_machine();
    let _ = m.step().unwrap();
    let timebase = assert_query_is_read_only(&m, "direct query");
    assert_eq!(timebase.now_ns, 100);
    assert_eq!(timebase.next_cpu_ready_ns, None);
}

#[test]
fn clock_queries_are_stable_across_reset_and_pending_time() {
    let mut m = timer_machine();
    let _ = m.step().unwrap();
    assert_eq!(m.virtual_ns(), 100);
    assert_eq!(m.timebase().deferred_ns, 100);

    // A reset flushes pending device time before clearing the logical clock.
    m.reset();
    assert_eq!(m.virtual_ns(), 0);
    assert_eq!(m.timebase().deferred_ns, 0);
    assert_eq!(m.timebase().now_ns, 0);
}

#[test]
fn scheduler_deadline_ownership_is_explicit_and_resets() {
    let mut m = Machine::bare_metal(0x4000, 0x1000).unwrap();
    m.advance_ns(5_000);
    assert_eq!(m.timebase().next_cpu_ready_ns, None);

    m.load_image(0x1000, &LOOP).unwrap();
    m.cpu.set_pc_npc(0x1000, 0x1004);
    m.step_scheduled().unwrap();
    assert!(m.timebase().next_cpu_ready_ns.is_some());

    m.reset();
    assert_eq!(m.timebase().next_cpu_ready_ns, None);
}

#[test]
fn blocked_keyboard_queue_reports_a_baud_deadline_not_zero() {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    bus.devices_mut().unwrap().keyboard = Some(Keyboard::default());
    let before = bus.diagnostic_state();
    let deadline = bus.next_device_deadline_ns();
    assert!(deadline.is_some());
    assert!(deadline.unwrap() > bus.logical_ns());
    bus.advance_ns(0);
    assert_eq!(before, bus.diagnostic_state());
}
