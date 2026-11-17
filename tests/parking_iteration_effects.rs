mod support;

use helios::bus::Bus;
use helios::cpu::Step;
use helios::mmu::Width;
use support::{idle_machine, iteration};

#[test]
fn instruction_ptp2_created_by_an_earlier_fetch_changes_the_helper() {
    let mut m = idle_machine();
    // Make the later helper fetch miss only its fast-cache key. No PTP2 is
    // installed yet; the observer currently sees the expected base mapping.
    m.cpu
        .mmu
        .load(&mut m.bus, 0xf0171708, 9, Width::Word)
        .unwrap();
    m.bus.write32(0x2004, 0x401).unwrap(); // fill_ptp2 source for f0040000
    m.bus.write32(0x40c4, 0x000400ee).unwrap();
    m.bus.write32(0x431708, 0x82006001).unwrap();
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x500, 5, Width::Word).unwrap(),
        0
    );
    let admitted = m.parking_token().is_ok();
    // The first fetch creates PTP2; ten instructions later the helper uses it.
    let mut actual = None;
    for _ in 0..11 {
        actual = Some(m.cpu.step(&mut m.bus).unwrap());
    }
    assert!(
        matches!(
            actual,
            Some(Step::Executed {
                pc: 0xf0071708,
                instruction: 0x82006001,
                ..
            })
        ),
        "{actual:?}"
    );
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x500, 5, Width::Word).unwrap(),
        0x401
    );
    assert!(
        !admitted,
        "the admitted loop actually executes active code after a PTP2 fill"
    );
}

#[test]
fn data_ptp2_created_by_the_first_load_changes_the_second_load() {
    let mut m = idle_machine();
    m.cpu
        .mmu
        .load(&mut m.bus, 0xf036dea0, 11, Width::Word)
        .unwrap();
    m.bus.write32(0x2024, 0x401).unwrap(); // fill_ptp2 source for f0240000
    m.bus.write32(0x40b4, 0x000400ee).unwrap();
    m.bus.write32(0x42dea0, 1).unwrap();
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x500, 6, Width::Word).unwrap(),
        0
    );
    let admitted = m.parking_token().is_ok();
    let mut actual = None;
    for _ in 0..4 {
        actual = Some(m.cpu.step(&mut m.bus).unwrap());
    }
    assert!(matches!(
        actual,
        Some(Step::Executed {
            pc: 0xf006fa34,
            instruction: 0xd0062058,
            ..
        })
    ));
    assert_eq!(
        m.cpu.read_reg(8),
        1,
        "normal execution now leaves the idle path"
    );
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x500, 6, Width::Word).unwrap(),
        0x401
    );
    assert!(
        !admitted,
        "the second dependency changes after the first load fills PTP2"
    );
}

#[test]
fn global_mapping_diagnostic_fill_uses_entry_context_not_current_context() {
    let mut m = idle_machine();
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x9100, 6, Width::Word).unwrap(),
        0
    );
    m.bus.write32(0x101c, 0x201).unwrap(); // context 7 has the same L1 table
    m.bus.write32(0x23c0, 0xba).unwrap(); // supervisor-global executable/readable leaf
    m.cpu
        .mmu
        .store(&mut m.bus, 0x1000, 4, Width::Word, 0)
        .unwrap(); // replace slot zero
    m.cpu
        .mmu
        .store(&mut m.bus, 0x200, 4, Width::Word, 7)
        .unwrap();
    assert_eq!(
        m.cpu
            .mmu
            .load(&mut m.bus, 0xf006fa24, 9, Width::Word)
            .unwrap(),
        0xd00220ec
    );
    m.cpu
        .mmu
        .store(&mut m.bus, 0x200, 4, Width::Word, 0)
        .unwrap();
    let admitted = m.parking_token().is_ok();
    iteration(&mut m);
    assert_eq!(
        m.cpu.mmu.load(&mut m.bus, 0x9100, 6, Width::Word).unwrap(),
        7
    );
    assert!(
        !admitted,
        "normal data diagnostics fill context 7, not the current context 0"
    );
}
