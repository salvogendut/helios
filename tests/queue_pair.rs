use helios::{
    bus::Bus,
    devices::sx::SX_BASE,
    machine::{Machine, MachineProfile},
    mmu::{Width, PCR_EN},
};

const PAIR: [u32; 2] = [0xbfe0_0509, 0xbfe0_0d09];

fn fixture() -> Machine {
    let profile = MachineProfile::Ss20;
    let mut m =
        Machine::with_profile(profile, 16 * 1024 * 1024, &[0; 4], profile.default_nvram()).unwrap();
    for r in 1..128 {
        m.bus
            .write32(SX_BASE + 0x100 + r * 4, 0xa5a5_0000 | r as u32)
            .unwrap();
    }
    m.bus.write32(SX_BASE + 0x124, 0x1234_5678).unwrap();
    m.bus.write32(SX_BASE, 0x1000).unwrap();
    m
}
fn regs(m: &Machine) -> Vec<u32> {
    let sx = m.bus.sx().unwrap();
    (0..128).map(|r| sx.register_word(0x100 + r * 4)).collect()
}
fn expected_pair(m: &Machine) -> Vec<u32> {
    let mut expected = regs(m);
    let source = expected[9];
    expected[10..42].fill(source);
    expected
}

#[test]
fn captured_std_enqueues_both_words_through_user_iq_port() {
    let mut m = fixture();
    let expected = expected_pair(&m);
    let result = m.bus.write_doubleword(SX_BASE + 0x1020, PAIR);
    eprintln!(
        "captured user STD: {result:?}; R10={:#x}, R26={:#x}",
        regs(&m)[10],
        regs(&m)[26]
    );
    assert!(
        result.is_ok(),
        "one 64-bit IQ transaction carries two instructions: {result:?}"
    );
    assert_eq!(regs(&m), expected);
}

#[test]
fn captured_std_enqueues_both_words_through_supervisor_iq_port() {
    let mut m = fixture();
    let expected = expected_pair(&m);
    m.bus.write_doubleword(SX_BASE + 0x20, PAIR).unwrap();
    assert_eq!(
        regs(&m),
        expected,
        "the second word is an instruction, not a reserved-register write"
    );
}

#[test]
fn paired_instructions_execute_in_order_within_the_transaction() {
    let mut m = fixture();
    let mut expected = regs(&m);
    expected[10] = expected[9];
    expected[20] = expected[9];
    // ORS R0,R9 -> R10, then ORS R0,R10 -> R20.
    m.bus
        .write_doubleword(SX_BASE + 0x1020, [0xb0e0_0509, 0xb0e0_0a0a])
        .unwrap();
    assert_eq!(regs(&m), expected);
}

#[test]
fn rejected_second_instruction_does_not_commit_the_first() {
    let mut m = fixture();
    let before = regs(&m);
    let result = m
        .bus
        .write_doubleword(SX_BASE + 0x1020, [0xb0e0_0509, 0xa000_0000]);
    assert!(result.is_err());
    assert_eq!(
        regs(&m),
        before,
        "a rejected pair must leave the SX registers retryable"
    );
}

#[test]
fn reserved_user_word_at_0x24_does_not_become_an_iq_alias() {
    let mut m = fixture();
    let before = regs(&m);
    assert!(m.bus.write32(SX_BASE + 0x1024, 0xb0e0_0509).is_err());
    assert_eq!(regs(&m), before);
}

#[test]
fn original_pte_and_user_va_accept_the_captured_std_without_an_mmu_fault() {
    let mut m = fixture();
    let va = 0xef7a_0020u32;
    // Original context 24 and PTE 0xf8000166. Physical SX user IQ is
    // 0xf80001020. Independent standard SRMMU L1/L2/L3 table layout.
    for (pa, value) in [
        (0x1000 + 24 * 4, 0x201),
        (0x2000 + ((va >> 24) as u64) * 4, 0x301),
        (0x3000 + (((va >> 18) & 63) as u64) * 4, 0x401),
        (0x4000 + (((va >> 12) & 63) as u64) * 4, 0xf800_0166),
    ] {
        m.bus.write32(pa, value).unwrap();
    }
    for (address, value) in [(0x100, 0x100), (0x200, 24), (0, PCR_EN)] {
        m.cpu
            .mmu
            .store(&mut m.bus, address, 4, Width::Word, value)
            .unwrap();
    }
    let expected = expected_pair(&m);
    let result = m.cpu.mmu.store_doubleword(&mut m.bus, va, 10, PAIR);
    eprintln!(
        "translated STD: {result:?}; SFSR={:#x}, SFAR={:#x}",
        m.cpu.mmu.fault_status(),
        m.cpu.mmu.fault_address()
    );
    assert!(
        result.is_ok(),
        "valid user mapping must accept the IQ pair: {result:?}"
    );
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert_eq!(regs(&m), expected);
}

#[test]
fn original_cpu_std_retires_instead_of_returning_a_successful_trap_step() {
    use helios::cpu::{Cpu, Step};
    let mut m = fixture();
    let pc = 0xee66_f104u32;
    let va = 0xef7a_0020u32;
    m.cpu = Cpu::new(pc - 4);
    m.cpu.use_mmu_config(MachineProfile::Ss20.mmu());
    m.cpu.use_cpu_config(MachineProfile::Ss20.cpu_config());
    for (pa, value) in [
        (0x1000 + 24 * 4, 0x201),
        (0x2000 + ((va >> 24) as u64) * 4, 0x301),
        (0x3000 + (((va >> 18) & 63) as u64) * 4, 0x401),
        (0x4000 + (((va >> 12) & 63) as u64) * 4, 0xf800_0166),
        (0x2000 + ((pc >> 24) as u64) * 4, 0x601),
        (0x6000 + (((pc >> 18) & 63) as u64) * 4, 0x701),
        (0x7000 + (((pc >> 12) & 63) as u64) * 4, 0x90e),
        (0x9100, 0x8188_2020), // WRPSR: enable traps and enter user mode.
        (0x9104, 0xd03e_2020), // Original STD %o0,[%i0+0x20].
    ] {
        m.bus.write32(pa, value).unwrap();
    }
    for (address, value) in [(0x100, 0x100), (0x200, 24), (0, PCR_EN)] {
        m.cpu
            .mmu
            .store(&mut m.bus, address, 4, Width::Word, value)
            .unwrap();
    }
    m.cpu.write_reg(8, PAIR[0]);
    m.cpu.write_reg(9, PAIR[1]);
    m.cpu.write_reg(24, va - 0x20);
    assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
    let expected = expected_pair(&m);
    let step = m.step();
    eprintln!(
        "original CPU STD step={step:?}; SFSR={:#x}, SFAR={:#x}",
        m.cpu.mmu.fault_status(),
        m.cpu.mmu.fault_address()
    );
    assert!(
        matches!(step, Ok(Step::Executed { pc: p, instruction: 0xd03e_2020, .. }) if p == pc),
        "a checked Result alone misses a synchronous guest trap: {step:?}"
    );
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (pc + 4, pc + 8));
    assert_eq!(regs(&m), expected);
}
