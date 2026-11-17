use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_PS, PSR_S};
use helios::machine::{
    InstalledVideo, Machine, MachineProfile, SOLARIS_IDLE_NPC, SOLARIS_IDLE_PC, SOLARIS_IDLE_PSR,
};
use helios::mmu::{Width, PCR_EN};

const LOOP: [u8; 8] = [0x10, 0x80, 0, 0, 1, 0, 0, 0];

// This deliberately reproduces the submitted tests' admission fixture. It has
// no instruction or data mapping at the supposedly recognized Solaris PC.
fn address_only_machine() -> Machine {
    let mut m = Machine::with_profile_processors_video(
        MachineProfile::Ss20,
        16 * 1024 * 1024,
        &LOOP,
        MachineProfile::Ss20.default_nvram(),
        1,
        InstalledVideo::None,
    )
    .unwrap();
    let words = [0x8188_6000u32, 0x80a0_0000u32];
    let code: Vec<_> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    m.load_image(0x1000, &code).unwrap();
    assert!(m.set_processor_entry(0, 0x1000, 0x1004));
    m.cpu.write_reg(1, PSR_S | PSR_PS | PSR_ET);
    m.cpu.step(&mut m.bus).unwrap();
    m.cpu.step(&mut m.bus).unwrap();
    m.cpu.set_pc_npc(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC);
    assert_eq!(m.cpu.psr(), SOLARIS_IDLE_PSR);
    m
}

// Reconstruct the actual captured loop and its three load dependencies. An L1
// 16 MiB mapping translates f0xxxxxx to physical 00xxxxxx, with R/M pre-set.
// Eight complete warmup iterations make the MMU diagnostic view steady too.
pub fn idle_machine() -> Machine {
    let mut m = address_only_machine();
    for (pa, word) in [
        (0x1000, 0x201),
        (0x23c0, 0xee),
        (0x06fa24, 0xd00220ec),
        (0x06fa28, 0x80a22001),
        (0x06fa2c, 0x3280000f),
        (0x06fa30, 0x01000000), // annulled on the captured path
        (0x06fa34, 0xd0062058),
        (0x06fa38, 0x80a22000),
        (0x06fa3c, 0x02800006),
        (0x06fa40, 0x213c097e),
        (0x06fa54, 0xe00421e8),
        (0x06fa58, 0x9fc40000),
        (0x06fa5c, 0x01000000),
        (0x071708, 0x81c3e008),
        (0x07170c, 0x01000000),
        (0x06fa60, 0x10bffff1),
        (0x06fa64, 0x113c097e),
        (0x25f8ec, 1),
        (0x26dea0, 0),
        (0x25f9e8, 0xf0071708),
    ] {
        m.bus.write32(pa, word).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.write_reg(8, 0xf025f800);
    m.cpu.write_reg(24, 0xf026de48);
    m.cpu.write_reg(16, 0xf0071708);
    m.cpu.write_reg(15, 0xf006fa58);
    for _ in 0..8 {
        iteration(&mut m);
    }
    m
}

pub fn iteration(m: &mut Machine) {
    for expected in [
        0xf006fa24, 0xf006fa28, 0xf006fa2c, 0xf006fa34, 0xf006fa38, 0xf006fa3c, 0xf006fa40,
        0xf006fa54, 0xf006fa58, 0xf006fa5c, 0xf0071708, 0xf007170c, 0xf006fa60, 0xf006fa64,
    ] {
        assert_eq!(m.cpu.pc(), expected);
        assert!(matches!(
            m.cpu.step(&mut m.bus).unwrap(),
            Step::Executed { .. }
        ));
    }
    assert_eq!(m.cpu.pc(), SOLARIS_IDLE_PC);
    assert_eq!(m.cpu.npc(), SOLARIS_IDLE_NPC);
    assert_eq!(m.cpu.psr(), SOLARIS_IDLE_PSR);
}
