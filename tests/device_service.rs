use helios::machine::{Machine, MachineProfile};
use helios::mmu::Width;

fn machines() -> (Machine, Machine) {
    let mut guarded = Machine::with_profile(
        MachineProfile::Ss5,
        0x100_0000,
        &[1, 0, 0, 0],
        MachineProfile::Ss5.default_nvram(),
    )
    .unwrap();
    let mut reference = Machine::with_profile(
        MachineProfile::Ss5,
        0x100_0000,
        &[1, 0, 0, 0],
        MachineProfile::Ss5.default_nvram(),
    )
    .unwrap();
    for machine in [&mut guarded, &mut reference] {
        machine
            .cpu
            .mmu
            .store(&mut machine.bus, 0, 4, Width::Word, 0)
            .unwrap();
        let nops = [0x01, 0x00, 0x00, 0x00].repeat(10_000);
        machine.load_image(0x1000, &nops).unwrap();
        machine.cpu.set_pc_npc(0x1000, 0x1004);
    }
    reference.set_device_service_guards(false);
    (guarded, reference)
}

#[test]
fn idle_service_guards_match_unconditional_device_service() {
    let (mut guarded, mut reference) = machines();
    for step in 0..1_000 {
        assert_eq!(guarded.step().unwrap(), reference.step().unwrap());
        if step % 100 == 0 {
            assert_eq!(
                guarded
                    .diagnostic_snapshot()
                    .differences(&reference.diagnostic_snapshot()),
                Vec::<&'static str>::new()
            );
        }
    }
}
