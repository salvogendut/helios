use helios::bus::Bus;
use helios::devices::{
    cgsix::Cgsix,
    keyboard::Keyboard,
    nvram::{Nvram, CLOCK_CONTROL},
    timer::Timers,
};
use helios::machine::{CpuClockMode, Machine};
use helios::system_bus::{SystemBus, CGSIX_BASE};

const LOOP: [u8; 8] = [0x10, 0x80, 0, 0, 1, 0, 0, 0];
const THC: u64 = CGSIX_BASE + 0x301818;

fn video_bus() -> SystemBus {
    let mut bus = SystemBus::default();
    bus.install_cgsix(Cgsix::new(&[0; 32768]).unwrap()).unwrap();
    bus.write32(THC, 0x420).unwrap();
    bus
}

fn devices() -> SystemBus {
    let mut bus = SystemBus::default();
    bus.install_sun4m(Nvram::default()).unwrap();
    bus
}

fn vblank_ns() -> u64 {
    // 900 visible lines, 1472 pixel clocks/line, 105.561 MHz.
    (900u64 * 1472 * 1_000_000_000).div_ceil(105_561_000)
}

#[test]
fn cgsix_next_deadline_is_actual_start_of_vblank() {
    let bus = video_bus();
    assert_eq!(bus.next_device_deadline_ns(), Some(vblank_ns()));
}

#[test]
fn cgsix_deadline_must_not_skip_a_future_irq() {
    let mut bus = video_bus();
    bus.advance_ns(1_000_000);
    let predicted = bus.next_device_deadline_ns().unwrap();
    bus.advance_ns(vblank_ns() - 1_000_000 - 1);
    assert_eq!(bus.read32(THC).unwrap() & 0x10, 0);
    bus.advance_ns(1);
    assert_ne!(bus.read32(THC).unwrap() & 0x10, 0);
    assert!(
        predicted <= bus.logical_ns(),
        "predicted {predicted} crosses actual IRQ at {}",
        bus.logical_ns()
    );
}

#[test]
fn cgsix_ack_inside_vblank_reports_frame_end_then_next_vblank() {
    let mut bus = video_bus();
    bus.advance_ns(vblank_ns());
    assert_eq!(bus.next_device_deadline_ns(), Some(bus.logical_ns()));
    bus.write32(THC, 0x430).unwrap();
    let frame = (943u64 * 1472 * 1_000_000_000).div_ceil(105_561_000);
    assert_eq!(bus.next_device_deadline_ns(), Some(frame));
    bus.advance_ns(frame - bus.logical_ns());
    let next = ((943u64 + 900) * 1472 * 1_000_000_000).div_ceil(105_561_000);
    assert_eq!(bus.next_device_deadline_ns(), Some(next));
}

#[test]
fn keyboard_backpressured_command_does_not_create_an_unserviceable_zero_deadline() {
    let mut bus = devices();
    {
        let d = bus.devices_mut().unwrap();
        d.keyboard = Some(Keyboard::default());
        for (reg, val) in [(4, 0x44), (14, 3), (3, 0xc1), (5, 0xea)] {
            d.keyboard_serial.write(4, reg).unwrap();
            d.keyboard_serial.write(4, val).unwrap();
        }
    }
    for expected in [0xff, 4, 0x7f] {
        bus.advance_ns(8_333_334);
        assert_eq!(
            bus.devices_mut().unwrap().keyboard_serial.read(6).unwrap(),
            expected
        );
    }
    {
        let d = bus.devices_mut().unwrap();
        for code in 1..=16 {
            assert!(d.keyboard.as_mut().unwrap().key(code, true));
        }
        d.keyboard_serial.write(6, 15).unwrap();
    }
    bus.advance_ns(0); // Try the command once; its response must await FIFO space.
    let before = bus.diagnostic_state();
    bus.advance_ns(0);
    assert_eq!(
        before,
        bus.diagnostic_state(),
        "zero time cannot unblock this command"
    );
    // Retained keyboard phase is 6/3 ns after three rounded byte times.
    assert_eq!(bus.next_device_deadline_ns(), Some(33_333_334));
}

#[test]
fn mouse_packet_backlog_still_needs_a_baud_deadline() {
    let mut bus = devices();
    {
        let d = bus.devices_mut().unwrap();
        for (reg, val) in [(4, 0x44), (14, 3), (3, 0xc1)] {
            d.keyboard_serial.write(0, reg).unwrap();
            d.keyboard_serial.write(0, val).unwrap();
        }
        d.mouse.motion(1000, 0); // Six full packets; motion remains, FIFO cannot accept another.
    }
    let before = bus.diagnostic_state();
    bus.advance_ns(0);
    assert_eq!(before, bus.diagnostic_state());
    assert_eq!(bus.next_device_deadline_ns(), Some(8_333_333));
}

#[test]
fn esp_reset_pulse_expiry_is_a_deadline_even_with_reset_irq_disabled() {
    let mut bus = devices();
    {
        let esp = &mut bus.devices_mut().unwrap().esp;
        esp.write(0x0c, 0).unwrap();
        esp.write(0x24, 0).unwrap(); // CCF=0 encodes factor 8: 130 * 8 * 25 ns.
        esp.write(0x20, 0x40).unwrap(); // Disable reset notification.
        esp.write(0x0c, 3).unwrap();
        assert!(!esp.irq());
    }
    let predicted = bus.next_device_deadline_ns();
    bus.advance_ns(25_999);
    assert!(bus.devices_mut().unwrap().esp.write(0x0c, 1).is_err());
    bus.advance_ns(1);
    assert!(bus.devices_mut().unwrap().esp.write(0x0c, 1).is_ok());
    assert_eq!(predicted, Some(26_000));
}

#[test]
fn rtc_fraction_and_stop_are_reflected_in_absolute_deadlines() {
    let mut bus = devices();
    assert_eq!(bus.next_device_deadline_ns(), Some(1_000_000_000));
    bus.advance_ns(999_999_999);
    assert_eq!(bus.next_device_deadline_ns(), Some(1_000_000_000));
    bus.advance_ns(1);
    assert_eq!(bus.next_device_deadline_ns(), Some(2_000_000_000));
    let n = &mut bus.devices_mut().unwrap().nvram;
    n.write(CLOCK_CONTROL, 0x80).unwrap();
    n.write(CLOCK_CONTROL + 1, 0x80).unwrap();
    n.write(CLOCK_CONTROL, 0).unwrap();
    assert_eq!(bus.next_device_deadline_ns(), None);
}

#[test]
fn processor_user_timer_deadline_keeps_fraction_and_overflow_state() {
    let mut t = Timers::default();
    t.write(true, 16, 1).unwrap();
    t.write(false, 12, 1).unwrap();
    t.write_user_doubleword([0x7fff_ffff, 0xffff_fc00]).unwrap();
    t.advance_ns(123);
    assert_eq!(t.next_deadline_ns(), Some(877));
    let before = t.clone();
    assert_eq!(t.next_deadline_ns(), Some(877));
    assert_eq!(t, before);
    t.advance_ns(876);
    assert_eq!(t.next_deadline_ns(), Some(1));
    t.advance_ns(1);
    assert_eq!(t.next_deadline_ns(), Some(0));
    assert_eq!(t.read_user_doubleword().unwrap(), [0x8000_0000, 0]);
}

#[test]
fn up_virtual_time_includes_prom_wait_and_fractional_cpu_cycles() {
    let mut m = Machine::ss5(8 * 1024 * 1024, &LOOP).unwrap();
    m.reset_with_clock_mode(CpuClockMode::Hardware);
    for cycles in 1..=85 {
        m.step().unwrap();
        let expected = cycles * 24 * 1_000_000_000 / 21_250_000 + cycles * 1000 / 85;
        assert_eq!(m.virtual_ns(), expected);
    }
    assert_eq!(m.virtual_ns(), 97_000);
    let time = m.virtual_ns();
    m.flush_device_time();
    assert_eq!(m.virtual_ns(), time);
    assert_eq!(m.timebase().deferred_ns, 0);
}

#[test]
fn explicit_time_on_an_unstepped_up_machine_does_not_claim_scheduler_ownership() {
    let mut m = Machine::bare_metal(0x4000, 0x1000).unwrap();
    m.advance_ns(5_000);
    assert_eq!(m.timebase().next_cpu_ready_ns, None);
}
