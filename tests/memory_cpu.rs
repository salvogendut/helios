use helios::bus::{Bus, BusErrorKind};
use helios::cpu::{FaultKind, Step, PSR_ET, PSR_PS, PSR_S, PSR_VERSION};
use helios::machine::{Machine, MAX_PROM_BYTES};
use helios::mmu::{FaultType, Width, PCR_BM, PCR_EN, PCR_NF, PROM_BASE};

const NOP: u32 = 0x0100_0000;

fn immediate(op: u32, op3: u32, rd: u32, rs1: u32, value: u32) -> u32 {
    (op << 30) | (rd << 25) | (op3 << 19) | (rs1 << 14) | 0x2000 | (value & 0x1fff)
}

fn alternate(op3: u32, rd: u32, rs1: u32, rs2: u32, asi: u32) -> u32 {
    0xc000_0000 | (rd << 25) | (op3 << 19) | (rs1 << 14) | (asi << 5) | rs2
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn setup(program: &[u32]) -> Machine {
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(0, &bytes(program)).unwrap();
    machine
}

fn enable(m: &mut Machine, control: u32) {
    // Context 0 -> region 0 -> segment 0 -> page 0 (identity, user/supervisor RWX).
    for (address, word) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4000, 0x0e),
    ] {
        m.bus.write32(address, word).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, control)
        .unwrap();
}

#[test]
fn alternate_loads_and_stores_decode_asi_rs2_and_signed_lanes() {
    let mut m = setup(&[
        alternate(0x10, 2, 1, 3, 0x20),
        alternate(0x19, 4, 1, 0, 0x20),
        alternate(0x1a, 5, 1, 0, 0x20),
        alternate(0x15, 6, 1, 3, 0x20),
        alternate(0x16, 7, 1, 0, 0x20),
        alternate(0x14, 2, 1, 8, 0x20),
        alternate(0x11, 9, 1, 0, 0x20),
        alternate(0x12, 10, 1, 0, 0x20),
    ]);
    m.cpu.write_reg(1, 0x8000_8000);
    m.cpu.write_reg(3, 4);
    m.cpu.write_reg(6, 0x56);
    m.cpu.write_reg(7, 0xabcd);
    m.cpu.write_reg(8, 8);
    m.bus.write32(0x8000, 0x89ab_cdef).unwrap();
    m.bus.write32(0x8004, 0x1234_5678).unwrap();
    for _ in 0..8 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(2), 0x1234_5678);
    assert_eq!(m.cpu.read_reg(4), 0xffff_ff89);
    assert_eq!(m.cpu.read_reg(5), 0xffff_89ab);
    assert_eq!(m.cpu.read_reg(9), 0xab);
    assert_eq!(m.cpu.read_reg(10), 0xabcd);
    assert_eq!(m.bus.read32(0x8000).unwrap(), 0xabcd_cdef);
    assert_eq!(m.bus.read32(0x8004).unwrap(), 0x5634_5678);
    assert_eq!(m.bus.read32(0x8008).unwrap(), 0x1234_5678);
}

#[test]
fn alternate_privilege_and_encoding_checks_precede_control_space_side_effects() {
    for supervisor in [false, true] {
        for invalid_encoding in [false, true] {
            let mut instruction = alternate(0x10, 0, 1, 0, 4);
            if invalid_encoding {
                instruction |= 0x2000;
            }
            let psr = PSR_ET | if supervisor { PSR_S } else { 0 };
            let mut m = setup(&[immediate(2, 0x31, 0, 0, psr), instruction]);
            m.cpu.write_reg(1, 0x300); // destructive SFSR read, even if rd=g0
            m.cpu
                .mmu
                .store(&mut m.bus, 0x1300, 4, Width::Word, 0x326)
                .unwrap();
            m.step().unwrap();
            let result = m.step().unwrap();
            if supervisor && !invalid_encoding {
                assert!(matches!(result, Step::Executed { .. }));
                assert_eq!(m.cpu.mmu.fault_status(), 0);
            } else {
                assert!(
                    matches!(result, Step::TrapTaken { trap_type, .. } if trap_type == if supervisor { 2 } else { 3 })
                );
                assert_eq!(m.cpu.mmu.fault_status(), 0x326);
            }
        }
    }
}

#[test]
fn unsupported_control_spaces_stop_without_cpu_error_state() {
    let mut m = setup(&[alternate(0x10, 1, 0, 0, 6)]);
    let before = m.cpu.clone();
    assert!(matches!(
        m.step().unwrap_err().kind,
        FaultKind::UnsupportedMemory(_)
    ));
    assert_eq!(m.cpu, before);
}

#[test]
fn data_translation_fault_in_branch_slot_can_be_mapped_and_retried() {
    let mut m = setup(&[
        immediate(2, 0x31, 0, 0, PSR_S | PSR_ET),
        0x1080_0007,              // ba 0x20, delayed
        immediate(3, 0, 2, 1, 0), // ld [%g1], %g2
    ]);
    m.cpu.write_reg(1, 0x2000);
    m.cpu.write_reg(3, 0x4008); // missing PTE
    m.cpu.write_reg(4, 0x90e); // map VA 0x2000 to PA 0x9000, RWX
    m.bus.write32(0x9000, 0xface_feed).unwrap();
    m.load_image(
        0x90,
        &bytes(&[
            alternate(0x14, 4, 3, 0, 0x20), // handler installs the PTE by bypass store
            immediate(2, 0x38, 0, 17, 0),   // jmpl %l1, %g0
            immediate(2, 0x39, 0, 18, 0),   // rett %l2
        ]),
    )
    .unwrap();
    enable(&mut m, PCR_EN);
    m.step().unwrap();
    m.step().unwrap();
    let Step::TrapTaken {
        fault,
        trap_type,
        vector,
    } = m.step().unwrap()
    else {
        panic!("expected trap");
    };
    assert_eq!((trap_type, vector, fault.pc, fault.npc), (9, 0x90, 8, 0x20));
    assert!(matches!(fault.kind, FaultKind::DataMmu(e) if e.kind == FaultType::Invalid));
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (8, 0x20));
    assert_eq!(m.cpu.read_reg(2), 0);
    assert_eq!(
        (m.cpu.mmu.fault_status(), m.cpu.mmu.fault_address()),
        (0x326, 0x2000)
    );
    for _ in 0..3 {
        m.step().unwrap();
    }
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (8, 0x20));
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(2), 0xface_feed);
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0x20, 0x24));
    assert_eq!(m.cpu.current_window(), 0);
    assert_eq!(m.bus.read32(0x4008).unwrap(), 0x92e);
}

#[test]
fn fetch_exception_has_distinct_vector_and_no_instruction_word() {
    let mut m = setup(&[immediate(2, 0x31, 0, 0, PSR_ET | PSR_S)]);
    m.step().unwrap();
    enable(&mut m, PCR_EN | PCR_NF);
    m.bus.write32(0x4000, 0).unwrap();
    let Step::TrapTaken {
        fault,
        trap_type,
        vector,
    } = m.step().unwrap()
    else {
        panic!("expected fetch trap");
    };
    assert_eq!((trap_type, vector), (1, 0x10));
    assert_eq!(fault.instruction, None);
    assert!(matches!(fault.kind, FaultKind::InstructionMmu(e) if e.kind == FaultType::Invalid));
    assert_eq!((m.cpu.read_reg(17), m.cpu.read_reg(18)), (4, 8));
}

#[test]
fn translation_fault_with_et_clear_latches_error_instead_of_implying_no_fault() {
    let mut m = setup(&[immediate(3, 0, 2, 1, 0)]);
    m.cpu.write_reg(1, 0x1000);
    m.cpu.write_reg(2, 99);
    enable(&mut m, PCR_EN);
    let error = m.step().unwrap_err();
    assert!(matches!(error.kind, FaultKind::DataMmu(_)));
    assert_eq!(m.cpu.read_reg(2), 99);
    assert_eq!(m.cpu.error_state(), Some(error));
    let before = m.cpu.clone();
    assert_eq!(m.step().unwrap_err(), error);
    assert_eq!(m.cpu, before);
}

#[test]
fn translated_physical_bus_error_remains_an_access_error() {
    let mut m = setup(&[
        immediate(2, 0x31, 0, 0, PSR_ET | PSR_S),
        immediate(3, 0, 2, 1, 0),
    ]);
    m.cpu.write_reg(1, 0x1000);
    enable(&mut m, PCR_EN);
    m.bus.write32(0x4004, 0x200e).unwrap(); // PA 0x20000 is unmapped
    m.step().unwrap();
    let Step::TrapTaken {
        fault, trap_type, ..
    } = m.step().unwrap()
    else {
        panic!("expected bus trap");
    };
    assert_eq!(trap_type, 0x29);
    assert!(matches!(fault.kind, FaultKind::DataAccess(e) if e.kind == BusErrorKind::Unmapped));
    assert_eq!(m.cpu.mmu.fault_status(), 0x36);
    assert_eq!(m.cpu.mmu.fault_address(), 0x1000);
}

#[test]
fn reset_fetches_prom_reads_data_ram_and_guest_can_leave_boot_mode() {
    let prom = bytes(&[
        immediate(3, 0, 1, 0, 0),    // ld [0], %g1: data comes from RAM
        alternate(0x14, 0, 0, 0, 4), // sta %g0, [0] ASI 4: clear boot mode
    ]);
    let mut m = Machine::sun4m(0x10000, &prom).unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.npc()), (0, 4));
    assert_eq!(m.cpu.psr(), PSR_VERSION | PSR_S | PSR_PS);
    assert_eq!(m.cpu.window_invalid_mask(), 1);
    assert_eq!(m.cpu.mmu.control(), 0x0400_4000);
    m.bus.write32(0, 0x1234_5678).unwrap();
    m.bus.write32(8, immediate(2, 2, 2, 0, 42)).unwrap();
    for _ in 0..3 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(1), 0x1234_5678);
    assert_eq!(m.cpu.read_reg(2), 42);
    assert_eq!(m.cpu.mmu.control(), 0x0400_0000);
    assert_eq!(
        m.bus.read32(PROM_BASE).unwrap(),
        u32::from_be_bytes(prom[..4].try_into().unwrap())
    );
    assert_eq!(
        m.bus.read32(PROM_BASE + MAX_PROM_BYTES as u64 - 4).unwrap(),
        u32::MAX
    );
    assert_eq!(
        m.bus.write32(PROM_BASE, 0).unwrap_err().kind,
        BusErrorKind::ReadOnly
    );
    // An ordinary fault latches error mode; reset clears it without losing RAM.
    m.step().unwrap_err();
    m.reset();
    assert_eq!(m.cpu.error_state(), None);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert_eq!(m.cpu.mmu.control() & PCR_BM, PCR_BM);
    assert_eq!(m.cpu.read_reg(1), 0);
    assert_eq!(m.bus.read32(0).unwrap(), 0x1234_5678);
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(1), 0x1234_5678);
}

#[test]
fn guest_can_enable_translation_and_clear_boot_mode_in_one_store() {
    let mut m = Machine::sun4m(
        0x10000,
        &bytes(&[
            immediate(2, 2, 1, 0, 1),    // or %g0, 1, %g1
            alternate(0x14, 1, 0, 0, 4), // enable translation, clear BM
        ]),
    )
    .unwrap();
    // Install a nonidentity mapping for VA 0. The STA commits before the next fetch.
    enable(&mut m, PCR_BM);
    m.bus.write32(0x4000, 0x800e).unwrap(); // PA 0x80000, outside normal RAM
    m.bus.map_ram(0x80000, 4096).unwrap();
    m.bus.write32(0x80008, immediate(2, 2, 2, 0, 77)).unwrap();
    for _ in 0..3 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(2), 77);
    assert_eq!(m.cpu.pc(), 12);
}

#[test]
fn prom_size_limits_and_partial_last_word_are_explicit() {
    assert!(Machine::sun4m(4096, &[]).is_err());
    assert!(Machine::sun4m(4096, &vec![0; MAX_PROM_BYTES + 1]).is_err());
    let mut m = Machine::sun4m(4096, &[1, 2, 3]).unwrap();
    assert_eq!(m.bus.read32(PROM_BASE).unwrap(), 0x0102_03ff);
    let mut m = Machine::sun4m(4096, &bytes(&[NOP])).unwrap();
    assert!(matches!(
        m.step().unwrap(),
        Step::Executed {
            instruction: NOP,
            ..
        }
    ));
}
