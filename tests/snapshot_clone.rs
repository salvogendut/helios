use helios::bus::MemoryBus;
use helios::machine::{Machine, MachineProfile};

fn cloned_snapshot_is_independent(processor: u8) {
    let machine = Machine::with_profile_processors(
        MachineProfile::Ss20,
        16 << 20,
        &[1, 0, 0, 0],
        MachineProfile::Ss20.default_nvram(),
        2,
    )
    .unwrap();
    let original = machine.diagnostic_snapshot();
    let cloned = original.clone();
    assert_eq!(original, cloned);
    let cloned_cpu = if processor == 0 {
        &cloned.cpu
    } else {
        cloned.secondary_cpu.as_ref().unwrap()
    };
    let original_cpu = if processor == 0 {
        &original.cpu
    } else {
        original.secondary_cpu.as_ref().unwrap()
    };
    let mut scratch_bus = MemoryBus::default();
    cloned_cpu
        .mmu
        .mxcc_handle()
        .borrow_mut()
        .write_u64(&mut scratch_bus, 0x01c0_0a00, 0x200)
        .unwrap();
    assert_eq!(
        machine
            .processor_cpu(processor)
            .unwrap()
            .mmu
            .mxcc_handle()
            .borrow()
            .read_word(0x01c0_0a04),
        0,
        "live machine must remain untouched"
    );
    let differences = original.differences(&cloned);
    assert_eq!(
        original_cpu
            .mmu
            .mxcc_handle()
            .borrow()
            .read_word(0x01c0_0a04),
        0,
        "cloned CPU{processor} snapshot mutated its source; differences={differences:?}"
    );
    assert!(differences.contains(&if processor == 0 { "cpu0" } else { "cpu2" }));
}

#[test]
fn cloned_cpu0_checkpoint_has_independent_mxcc() {
    cloned_snapshot_is_independent(0);
}

#[test]
fn cloned_cpu2_checkpoint_has_independent_mxcc() {
    cloned_snapshot_is_independent(2);
}
