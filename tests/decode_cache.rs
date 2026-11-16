use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{demo_image, Machine};

fn machines() -> (Machine, Machine) {
    let mut cached = Machine::bare_metal(0x4000, 0).unwrap();
    cached.cpu.set_decode_cache_enabled(true);
    cached.load_image(0, &demo_image()).unwrap();
    let mut interpreter = Machine::bare_metal(0x4000, 0).unwrap();
    interpreter.load_image(0, &demo_image()).unwrap();
    interpreter.cpu.set_decode_cache_enabled(false);
    (cached, interpreter)
}

#[test]
fn decoded_cache_matches_interpreter_after_each_demo_step() {
    let (mut cached, mut interpreter) = machines();
    for _ in 0..43 {
        assert_eq!(cached.step().unwrap(), interpreter.step().unwrap());
        assert_eq!(
            cached
                .diagnostic_snapshot()
                .differences(&interpreter.diagnostic_snapshot()),
            Vec::<&'static str>::new()
        );
    }
    assert_eq!(cached.cpu.read_reg(2), 55);
    assert_eq!(cached.bus.read32(0x100).unwrap(), 55);
}

#[test]
fn raw_word_validation_invalidates_self_modified_instruction() {
    // add %g1, 1, %g1
    let mut cached = Machine::bare_metal(0x1000, 0).unwrap();
    cached.cpu.set_decode_cache_enabled(true);
    cached.load_image(0, &0x8200_6001u32.to_be_bytes()).unwrap();
    assert!(matches!(cached.step().unwrap(), Step::Executed { .. }));
    assert_eq!(cached.cpu.read_reg(1), 1);

    // Replace the instruction at the same PC with add %g1, 2, %g1.
    cached.bus.write32(0, 0x8200_6002).unwrap();
    cached.cpu.set_pc_npc(0, 4);
    assert!(matches!(cached.step().unwrap(), Step::Executed { .. }));
    assert_eq!(cached.cpu.read_reg(1), 3);
}
