use helios::bus::{Bus, Initiator};
use helios::cpu::Step;
use helios::devices::scsi::MediaKind;
use helios::machine::{CpuClockMode, Machine, MachineProfile};
use helios::mmu::{Width, PCR_EN};
use helios::system_bus::{ESP_BASE as ESP, IO_CONTROL_BASE as IO, SYSTEM_INTERRUPT_BASE};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bare_with(words: &[u32]) -> Machine {
    let mut machine = Machine::bare_metal(0x4000, 0).unwrap();
    machine
        .load_image(
            0,
            &words
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    machine
}

fn bare() -> Machine {
    bare_with(&[
        0x9d_e3_bf_a0, // save %sp, -96, %sp
        0xa0_04_20_01, // add %l0, 1, %l0
        0x81_e8_00_00, // restore
    ])
}

fn peripheral() -> Machine {
    Machine::with_profile(
        MachineProfile::Ss5,
        16 * 1024 * 1024,
        &[1, 0, 0, 0],
        MachineProfile::Ss5.default_nvram(),
    )
    .unwrap()
}

fn difference(before: &Machine, after: &Machine) -> Vec<&'static str> {
    before
        .diagnostic_snapshot()
        .differences(&after.diagnostic_snapshot())
}

#[test]
fn identical_machines_have_no_differences() {
    let first = peripheral();
    let second = peripheral();
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
}

#[test]
fn inactive_window_and_cpu_clock_changes_are_detected() {
    let mut machine = bare();
    let before = machine.diagnostic_snapshot();
    let visible: Vec<_> = (0..32).map(|reg| machine.cpu.read_reg(reg)).collect();
    for _ in 0..3 {
        assert!(matches!(machine.step().unwrap(), Step::Executed { .. }));
    }
    machine.cpu.set_pc_npc(before.cpu.pc(), before.cpu.npc());
    let after = machine.diagnostic_snapshot();
    assert_eq!(
        (0..32)
            .map(|reg| machine.cpu.read_reg(reg))
            .collect::<Vec<_>>(),
        visible,
        "the local-register write leaked into the active register view"
    );
    let differences = before.differences(&after);
    assert!(differences.contains(&"cpu0"), "{differences:?}");
    assert_eq!(machine.cpu.current_window(), 0);
}

#[test]
fn fpu_and_mmu_state_changes_are_detected() {
    let mut machine = peripheral();
    let before = machine.diagnostic_snapshot();

    machine.cpu.fpu.write_reg(1, 0x3f80_0000);
    let fpu = machine.diagnostic_snapshot();
    assert!(before.differences(&fpu).contains(&"cpu0"));
    machine.cpu.fpu.write_reg(1, 0);

    machine.cpu.mmu.set_initiator(Initiator::Host);
    let mmu = machine.diagnostic_snapshot();
    assert!(before.differences(&mmu).contains(&"cpu0"));
}

#[test]
fn ram_device_and_timing_changes_are_detected() {
    let mut machine = peripheral();
    let before = machine.diagnostic_snapshot();

    machine
        .load_image(0x1000, &[0xaa, 0xbb, 0xcc, 0xdd])
        .unwrap();
    let ram = machine.diagnostic_snapshot();
    let ram_differences = before.differences(&ram);
    assert!(
        ram_differences.contains(&"bus.dram") || ram_differences.contains(&"bus.memory"),
        "{ram_differences:?}"
    );

    machine
        .bus
        .devices_mut()
        .unwrap()
        .nvram
        .write(0x100, 0x5a)
        .unwrap();
    let device = machine.diagnostic_snapshot();
    assert!(ram.differences(&device).contains(&"bus.devices"));

    machine.advance_ns(1);
    let timing = machine.diagnostic_snapshot();
    let timing_differences = device.differences(&timing);
    assert!(
        timing_differences.contains(&"bus.logical")
            || timing_differences.contains(&"bus.device_ns")
            || timing_differences.contains(&"scheduler"),
        "{timing_differences:?}"
    );
}

#[test]
fn one_executed_step_changes_cpu_and_bus_timing_state() {
    let mut machine = peripheral();
    let before = machine.diagnostic_snapshot();
    let _ = machine.step();
    let after = machine.diagnostic_snapshot();
    let differences = before.differences(&after);
    assert!(differences.contains(&"cpu0"), "{differences:?}");
    assert!(
        differences.contains(&"bus.prom") || differences.contains(&"bus.device_ns"),
        "{differences:?}"
    );
}

#[test]
fn hardware_clock_phase_changes_are_detected() {
    let mut machine = peripheral();
    machine.reset_with_clock_mode(CpuClockMode::Hardware);
    machine.load_image(0, &[0x01, 0x00, 0x00, 0x00]).unwrap();
    let before = machine.diagnostic_snapshot();
    assert!(matches!(machine.step().unwrap(), Step::Executed { .. }));
    let after = machine.diagnostic_snapshot();
    let differences = before.differences(&after);
    assert!(differences.contains(&"cpu0"), "{differences:?}");
    assert!(differences.contains(&"clock0"), "{differences:?}");
}

#[test]
fn bus_probe_does_not_mutate_diagnostic_state() {
    let mut machine = peripheral();
    let before = machine.diagnostic_snapshot();
    let _ = machine.bus.read32(0x1000).unwrap();
    assert_eq!(
        before.differences(&machine.diagnostic_snapshot()),
        Vec::<&'static str>::new()
    );
}

#[test]
fn stepped_identical_machines_remain_equal() {
    for _ in 0..32 {
        let mut first = bare();
        let mut second = bare();
        for _ in 0..3 {
            let first_step = first.step();
            let second_step = second.step();
            assert_eq!(first_step, second_step);
            if first_step.is_err() {
                break;
            }
        }
        assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
    }
}

#[test]
fn trap_handler_state_is_reproducible_and_comparator_stable() {
    // wr %g1, 0x23, %psr enables ET and selects CWP 3; ta 7 enters TT=0x70.
    let words = [0x81_88_60_23, 0x91_d0_20_07, 0x01_00_00_00];
    let mut first = bare_with(&words);
    let mut second = bare_with(&words);
    assert!(matches!(first.step().unwrap(), Step::Executed { .. }));
    assert!(matches!(second.step().unwrap(), Step::Executed { .. }));
    let first_trap = first.step().unwrap();
    let second_trap = second.step().unwrap();
    assert_eq!(first_trap, second_trap);
    assert!(matches!(
        first_trap,
        Step::TrapTaken {
            trap_type: 0x87,
            ..
        }
    ));
    assert_eq!(first.cpu.pc(), 0x870);
    assert_eq!(first.cpu.npc(), 0x874);
    assert_eq!(first.cpu.current_window(), 2);
    assert_eq!(first.cpu.tbr(), 0x870);
    assert_eq!(first.cpu.psr() & 0x20, 0);
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
}

#[test]
fn annulled_branch_skips_a_side_effecting_delay_slot() {
    let words = [
        0x30_80_00_02, // ba,a target PC + 8
        0xa0_04_20_01, // add %l0, 1, %l0 (must be annulled)
        0x01_00_00_00, // target
    ];
    let mut first = bare_with(&words);
    let mut second = bare_with(&words);
    assert!(matches!(first.step().unwrap(), Step::Executed { .. }));
    assert!(matches!(second.step().unwrap(), Step::Executed { .. }));
    assert_eq!((first.cpu.pc(), first.cpu.npc()), (8, 12));
    assert_eq!(first.cpu.read_reg(16), 0);
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
}

#[test]
fn secondary_processor_state_is_compared_separately() {
    let dual = || {
        Machine::with_profile_processors(
            MachineProfile::Ss20,
            16 * 1024 * 1024,
            &[1, 0, 0, 0],
            MachineProfile::Ss20.default_nvram(),
            2,
        )
        .unwrap()
    };
    let mut first = dual();
    let second = dual();
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
    first
        .secondary_cpu_mut()
        .unwrap()
        .cpu
        .fpu
        .write_reg(1, 0x3f80_0000);
    let differences = difference(&first, &second);
    assert!(differences.contains(&"cpu2"), "{differences:?}");
    assert!(!differences.contains(&"cpu0"), "{differences:?}");
}

fn mmu_write(machine: &mut Machine, register: u32, value: u32) {
    machine
        .cpu
        .mmu
        .store(&mut machine.bus, register << 8, 4, Width::Word, value)
        .unwrap();
}

fn mmu_flush(machine: &mut Machine, va: u32) {
    for kind in 0..=4 {
        machine
            .cpu
            .mmu
            .store(&mut machine.bus, va | (kind << 8), 3, Width::Word, 0)
            .unwrap();
    }
}

fn paged_machine() -> Machine {
    let mut machine = Machine::bare_metal(0x20000, 0).unwrap();
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x0000_088e),
    ] {
        machine.bus.write32(address, word).unwrap();
    }
    machine.bus.write32(0x8000, 0x1122_3344).unwrap();
    machine.bus.write32(0x9000, 0xaabb_ccdd).unwrap();
    mmu_write(&mut machine, 1, 0x100);
    mmu_write(&mut machine, 0, PCR_EN);
    machine
}

#[test]
fn page_remap_translation_and_replay_are_compared() {
    let va = 0x1234_5000;
    let mut first = paged_machine();
    let mut second = paged_machine();
    assert_eq!(
        first.cpu.mmu.load(&mut first.bus, va, 11, Width::Word),
        Ok(0x1122_3344)
    );
    assert_eq!(
        second.cpu.mmu.load(&mut second.bus, va, 11, Width::Word),
        Ok(0x1122_3344)
    );
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());

    let before = first.diagnostic_snapshot();
    first.bus.write32(0x4014, 0x0000_098e).unwrap();
    mmu_flush(&mut first, va);
    second.bus.write32(0x4014, 0x0000_098e).unwrap();
    mmu_flush(&mut second, va);
    assert_eq!(
        first.cpu.mmu.load(&mut first.bus, va, 11, Width::Word),
        Ok(0xaabb_ccdd)
    );
    assert_eq!(
        second.cpu.mmu.load(&mut second.bus, va, 11, Width::Word),
        Ok(0xaabb_ccdd)
    );
    let after = first.diagnostic_snapshot();
    assert!(!before.differences(&after).is_empty());
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
}

fn dma_machine() -> Machine {
    let mut machine = Machine::sun4m(0x20000, &0x0100_0000u32.to_be_bytes()).unwrap();
    machine.bus.write32(IO, 1).unwrap();
    machine.bus.write32(IO + 4, 0x1000).unwrap();
    machine.bus.write32(IO + 0x2000, 0x003f_0000).unwrap();
    machine.bus.write32(0x10000, 0x806).unwrap();
    machine.bus.write32(0x10004, 0x902).unwrap();
    machine.bus.write32(0x8000, 0x1234_5678).unwrap();
    machine.bus.write32(0x9000, 0xabcd_ef01).unwrap();
    machine
}

#[test]
fn dma_replay_is_deterministic_and_state_changes_are_detected() {
    let mut first = dma_machine();
    let mut second = dma_machine();
    assert_eq!(
        first.dma_read(5, 0xff00_0000, Width::Word),
        second.dma_read(5, 0xff00_0000, Width::Word)
    );
    first.dma_write(5, 0xff00_0001, Width::Byte, 0xaa).unwrap();
    second.dma_write(5, 0xff00_0001, Width::Byte, 0xaa).unwrap();
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
    let before = first.diagnostic_snapshot();
    first
        .dma_write(5, 0xff00_0002, Width::Half, 0xbbcc)
        .unwrap();
    let after = first.diagnostic_snapshot();
    assert!(!before.differences(&after).is_empty());
}

#[test]
fn interrupt_controller_replay_is_deterministic_and_detects_changes() {
    let mut first = Machine::sun4m(0x20000, &[1, 0, 0, 0]).unwrap();
    let mut second = Machine::sun4m(0x20000, &[1, 0, 0, 0]).unwrap();
    for machine in [&mut first, &mut second] {
        machine
            .bus
            .write32(SYSTEM_INTERRUPT_BASE + 8, (1 << 31) | (1 << 18))
            .unwrap();
    }
    assert_eq!(first.bus.interrupt_levels(), second.bus.interrupt_levels());
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
    let before = first.diagnostic_snapshot();
    first
        .bus
        .write32(SYSTEM_INTERRUPT_BASE + 12, 1 << 18)
        .unwrap();
    let after = first.diagnostic_snapshot();
    assert!(!before.differences(&after).is_empty());
}

#[test]
fn snapshots_own_independent_mxcc_state() {
    let mut first = Machine::with_profile_processors(
        MachineProfile::Ss20,
        16 * 1024 * 1024,
        &[1, 0, 0, 0],
        MachineProfile::Ss20.default_nvram(),
        2,
    )
    .unwrap();
    let snapshot = first.diagnostic_snapshot();
    let cpu0 = snapshot.cpu.mmu.mxcc_handle();
    let cpu2 = snapshot.secondary_cpu.as_ref().unwrap().mmu.mxcc_handle();
    assert_eq!(cpu0.borrow().read_word(0x01c0_0a04), 0);
    assert_eq!(cpu2.borrow().read_word(0x01c0_0a04), 0);
    first.bus.write32(0x000f_f8c0_0a04, 0x200).unwrap();
    first.bus.write32(0x000f_fac0_0a04, 0x200).unwrap();
    assert_eq!(cpu0.borrow().read_word(0x01c0_0a04), 0);
    assert_eq!(cpu2.borrow().read_word(0x01c0_0a04), 0);
}

#[test]
fn cpu_interrupt_entry_replays_and_changes_snapshot_state() {
    let words = [0x81_88_60_23, 0x01_00_00_00]; // ET/CWP=3, then nop
    let mut first = bare_with(&words);
    let mut second = bare_with(&words);
    assert!(matches!(first.step().unwrap(), Step::Executed { .. }));
    assert!(matches!(second.step().unwrap(), Step::Executed { .. }));
    let before = first.diagnostic_snapshot();
    let first_interrupt = first
        .cpu
        .step_with_interrupts(&mut first.bus, 1 << 10)
        .unwrap();
    let second_interrupt = second
        .cpu
        .step_with_interrupts(&mut second.bus, 1 << 10)
        .unwrap();
    assert_eq!(first_interrupt, second_interrupt);
    assert!(matches!(
        first_interrupt,
        Step::InterruptTaken { level: 10, .. }
    ));
    assert_eq!(first.cpu.pc(), 0x1a0);
    assert_eq!(first.cpu.npc(), 0x1a4);
    let after = first.diagnostic_snapshot();
    assert!(!before.differences(&after).is_empty());
    assert_eq!(difference(&first, &second), Vec::<&'static str>::new());
}

struct ReviewImage(PathBuf);

impl ReviewImage {
    fn new(value: u8) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-issue5-sliceb-{}-{}-scsi3.raw",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&vec![value; 4096]).unwrap();
        Self(path)
    }
}

impl Drop for ReviewImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn wait_esp_irq(machine: &mut Machine) -> u8 {
    for _ in 0..1000 {
        machine.service_scsi().unwrap();
        machine.advance_ns(100);
        if machine.bus.devices().unwrap().esp.irq() {
            let status = machine.bus.read8(ESP + 16).unwrap();
            machine.bus.read8(ESP + 24).unwrap();
            machine.bus.read8(ESP + 20).unwrap();
            return status;
        }
    }
    panic!("ESP never interrupted");
}

fn pending_esp_read(image: &ReviewImage) -> Machine {
    let mut machine = Machine::sun4m(0x20000, &[1, 0, 0, 0]).unwrap();
    machine
        .bus
        .devices_mut()
        .unwrap()
        .scsi
        .attach(3, MediaKind::Disk, &image.0)
        .unwrap();
    machine.bus.write8(ESP + 12, 0).unwrap();
    machine.bus.write8(ESP + 32, 7).unwrap();
    machine.bus.write8(ESP + 20, 1).unwrap();
    machine.bus.write8(ESP + 16, 3).unwrap();
    machine.bus.write8(ESP + 8, 0x80).unwrap();
    for byte in [0x08, 0, 0, 0, 1, 0] {
        machine.bus.write8(ESP + 8, byte).unwrap();
    }
    machine.bus.write8(ESP + 12, 0x42).unwrap();
    assert_eq!(wait_esp_irq(&mut machine) & 7, 1);
    machine
}

fn next_esp_pio_byte(machine: &mut Machine) -> u8 {
    machine.bus.write8(ESP + 12, 0x10).unwrap();
    wait_esp_irq(machine);
    assert_eq!(machine.bus.read8(ESP + 28).unwrap(), 1);
    machine.bus.read8(ESP + 8).unwrap()
}

#[test]
fn esp_buffered_payload_is_part_of_snapshot_state() {
    let first_image = ReviewImage::new(0x11);
    let second_image = ReviewImage::new(0x22);
    let mut first = pending_esp_read(&first_image);
    let mut second = pending_esp_read(&second_image);
    assert_ne!(
        first.bus.devices().unwrap().esp,
        second.bus.devices().unwrap().esp
    );
    let first_snapshot = first.diagnostic_snapshot();
    let second_snapshot = second.diagnostic_snapshot();
    assert_eq!(next_esp_pio_byte(&mut first), 0x11);
    assert_eq!(next_esp_pio_byte(&mut second), 0x22);
    assert!(
        first_snapshot
            .differences(&second_snapshot)
            .contains(&"bus.devices"),
        "staged ESP payloads compared equal"
    );
}
