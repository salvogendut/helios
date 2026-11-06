use helios::bus::{Bus, BusError, BusErrorKind, MemoryBus};
use helios::cpu::{Cpu, FaultKind, Step, PSR_ET, PSR_S};
use helios::machine::Machine;
use helios::mmu::{FaultType, MemoryError, Srmmu, Width, PCR_EN, PCR_NF};

fn mem(op3: u32, rd: u32, rs1: u32, operand: u32, asi: Option<u8>) -> u32 {
    0xc000_0000
        | (rd << 25)
        | (op3 << 19)
        | (rs1 << 14)
        | match asi {
            Some(asi) => (u32::from(asi) << 5) | (operand & 31),
            None => 0x2000 | (operand & 0x1fff),
        }
}

fn setup(program: &[u32]) -> Machine {
    let mut m = Machine::bare_metal(0x10000, 0).unwrap();
    let bytes: Vec<_> = program.iter().flat_map(|word| word.to_be_bytes()).collect();
    m.load_image(0, &bytes).unwrap();
    m
}

#[test]
fn doubleword_register_pairs_include_g0_window_edges_and_address_aliases() {
    for asi in [None, Some(0x20)] {
        let alt = if asi.is_some() { 0x10 } else { 0 };
        for rd in [0, 2, 14, 16, 30] {
            let mut m = setup(&[mem(3 | alt, rd, 2, 0, asi), mem(7 | alt, rd, 1, 0, asi)]);
            m.cpu.write_reg(2, 0x8000);
            m.cpu.write_reg(1, 0x9000);
            m.bus
                .write_doubleword(0x8000, [0x89ab_cdef, 0x1234_5678])
                .unwrap();
            m.step().unwrap();
            assert_eq!(
                m.cpu.read_reg(rd as u8),
                if rd == 0 { 0 } else { 0x89ab_cdef }
            );
            assert_eq!(m.cpu.read_reg(rd as u8 + 1), 0x1234_5678);
            // g1 was overwritten when rd=0; restore the store's address only.
            if rd == 0 {
                m.cpu.write_reg(1, 0x9000);
            }
            m.step().unwrap();
            assert_eq!(
                m.bus.read_doubleword(0x9000).unwrap(),
                if rd == 0 {
                    [0, 0x9000]
                } else {
                    [0x89ab_cdef, 0x1234_5678]
                }
            );
        }
    }
    let mut m = setup(&[mem(7, 2, 2, 0, None)]);
    m.cpu.write_reg(2, 0x8000);
    m.cpu.write_reg(3, 0xaabb_ccdd);
    m.step().unwrap();
    assert_eq!(
        m.bus.read_doubleword(0x8000).unwrap(),
        [0x8000, 0xaabb_ccdd]
    );
}

#[test]
fn doubleword_checks_privilege_then_encoding_and_register_pair_then_alignment() {
    for op3 in [3, 7] {
        for rd in [3, 31] {
            let mut m = setup(&[mem(op3, rd, 1, 0, None)]);
            m.cpu.write_reg(1, 0x8004); // both odd register pair and bad alignment
            assert_eq!(m.step().unwrap_err().kind, FaultKind::IllegalInstruction);
            assert_eq!(m.cpu.pc(), 0);
        }
        let mut m = setup(&[mem(op3, 2, 1, 0, None)]);
        m.cpu.write_reg(1, 0x8004);
        assert!(
            matches!(m.step().unwrap_err().kind, FaultKind::DataAccess(e) if e.kind == BusErrorKind::Misaligned)
        );
        assert_eq!(m.cpu.mmu.fault_status(), 0);
    }
    for op3 in [0x13, 0x17, 0x1d, 0x1f] {
        for immediate in [false, true] {
            let word = mem(op3, 3, 1, 0, Some(4)) | if immediate { 0x2000 } else { 0 };
            let mut m = setup(&[0x8188_2020, word]); // wr g0, ET, psr: user mode
            m.cpu.write_reg(1, 0x300);
            m.cpu
                .mmu
                .store(&mut m.bus, 0x1300, 4, Width::Word, 0x326)
                .unwrap();
            m.step().unwrap();
            assert!(matches!(
                m.step().unwrap(),
                Step::TrapTaken { trap_type: 3, .. }
            ));
            assert_eq!(m.cpu.mmu.fault_status(), 0x326);
        }
        let mut m = setup(&[mem(op3, 2, 1, 0, Some(4)) | 0x2000]);
        assert_eq!(m.step().unwrap_err().kind, FaultKind::IllegalInstruction);
    }
}

#[test]
fn failed_doubleword_access_does_not_commit_a_partial_register_or_memory_pair() {
    for op3 in [3, 7] {
        for rom in [false, true] {
            let mut m = setup(&[mem(op3, 2, 1, 0, None)]);
            m.cpu.write_reg(1, 0x20000);
            m.cpu.write_reg(2, 1);
            m.cpu.write_reg(3, 2);
            m.bus.map_ram(0x20000, 4).unwrap();
            m.bus.write32(0x20000, 0xaabb_ccdd).unwrap();
            if rom {
                m.bus
                    .map_rom(0x20004, &0x1234_5678u32.to_be_bytes())
                    .unwrap();
            }
            let result = m.step();
            if op3 == 3 && rom {
                result.unwrap();
                assert_eq!(
                    (m.cpu.read_reg(2), m.cpu.read_reg(3)),
                    (0xaabb_ccdd, 0x1234_5678)
                );
            } else {
                assert!(
                    matches!(result.unwrap_err().kind, FaultKind::DataAccess(e) if e.address == 0x20004)
                );
                assert_eq!((m.cpu.read_reg(2), m.cpu.read_reg(3)), (1, 2));
                assert_eq!(m.cpu.pc(), 0);
                assert_eq!(m.bus.read32(0x20000).unwrap(), 0xaabb_ccdd);
            }
        }
    }
}

#[test]
fn doubleword_at_top_of_virtual_space_uses_31_bit_physical_address() {
    let mut m = setup(&[mem(3, 2, 1, 0, None)]);
    m.bus.map_ram(0x7fff_fff8, 8).unwrap();
    m.bus.write_doubleword(0x7fff_fff8, [11, 22]).unwrap();
    m.cpu.write_reg(1, 0xffff_fff8);
    m.step().unwrap();
    assert_eq!((m.cpu.read_reg(2), m.cpu.read_reg(3)), (11, 22));
}

#[test]
fn atomics_handle_register_aliases_g0_and_every_unsigned_byte_lane() {
    for asi in [None, Some(0x20)] {
        let alt = if asi.is_some() { 0x10 } else { 0 };
        let mut m = setup(&[mem(0x0f | alt, 1, 1, 0, asi), mem(0x0f | alt, 0, 2, 0, asi)]);
        m.cpu.write_reg(1, 0x8000);
        m.cpu.write_reg(2, 0x8000);
        m.bus.write32(0x8000, 0xaabb_ccdd).unwrap();
        m.step().unwrap();
        assert_eq!(m.cpu.read_reg(1), 0xaabb_ccdd);
        assert_eq!(m.bus.read32(0x8000).unwrap(), 0x8000);
        m.step().unwrap();
        assert_eq!(m.bus.read32(0x8000).unwrap(), 0);
        assert_eq!(m.cpu.read_reg(0), 0);
        for (offset, old) in [(0, 1), (1, 0x80), (2, 0xfe), (3, 0xff)] {
            let instruction = mem(0x0d | alt, 3, 1, 0, asi);
            let mut m = setup(&[instruction, instruction]);
            m.cpu.write_reg(1, 0x8000 + offset);
            m.bus.write32(0x8000, 0x0180_feff).unwrap();
            m.step().unwrap();
            assert_eq!(m.cpu.read_reg(3), old);
            assert_eq!(
                m.bus.read32(0x8000).unwrap(),
                0x0180_feff | (0xff << ((3 - offset) * 8))
            );
            m.step().unwrap();
            assert_eq!(m.cpu.read_reg(3), 0xff);
        }
    }
}

#[test]
fn atomics_on_rom_and_misaligned_swap_preserve_destination_and_memory() {
    for op3 in [0x0d, 0x0f] {
        let mut m = setup(&[mem(op3, 2, 1, 0, None)]);
        m.bus
            .map_rom(0x20000, &0x89ab_cdefu32.to_be_bytes())
            .unwrap();
        m.cpu.write_reg(1, 0x20000);
        m.cpu.write_reg(2, 0x1234_5678);
        assert!(
            matches!(m.step().unwrap_err().kind, FaultKind::DataAccess(e) if e.kind == BusErrorKind::ReadOnly)
        );
        assert_eq!(m.cpu.read_reg(2), 0x1234_5678);
        assert_eq!(m.bus.read32(0x20000).unwrap(), 0x89ab_cdef);
    }
    let mut m = setup(&[mem(0x0f, 2, 1, 0, None)]);
    m.cpu.write_reg(1, 0x8001);
    assert!(
        matches!(m.step().unwrap_err().kind, FaultKind::DataAccess(e) if e.kind == BusErrorKind::Misaligned)
    );
    assert_eq!(m.cpu.mmu.fault_status(), 0);
}

fn translated(pte: u32, control: u32) -> (Srmmu, MemoryBus) {
    let mut bus = MemoryBus::default();
    bus.map_ram(0, 0x10000).unwrap();
    for (address, value) in [
        (0x1000, 0x201),
        (0x2000, 0x301),
        (0x3000, 0x401),
        (0x4008, pte),
    ] {
        bus.write32(address, value).unwrap();
    }
    bus.write_doubleword(0x8000, [0x1122_3344, 0x5566_7788])
        .unwrap();
    let mut mmu = Srmmu::default();
    mmu.store(&mut bus, 0x100, 4, Width::Word, 0x100).unwrap();
    mmu.store(&mut bus, 0, 4, Width::Word, control).unwrap();
    (mmu, bus)
}

#[test]
fn translated_doublewords_and_atomics_update_rm_and_preserve_neighboring_lanes() {
    let (mut mmu, mut bus) = translated(0x80e, PCR_EN);
    assert_eq!(
        mmu.load_doubleword(&mut bus, 0x2000, 10).unwrap(),
        [0x1122_3344, 0x5566_7788]
    );
    assert_eq!(bus.read32(0x4008).unwrap(), 0x82e);
    assert_eq!(
        mmu.swap(&mut bus, 0x2000, 10, 0xaabb_ccdd).unwrap(),
        0x1122_3344
    );
    assert_eq!(bus.read32(0x4008).unwrap(), 0x86e);
    assert_eq!(mmu.load_store_byte(&mut bus, 0x2005, 10).unwrap(), 0x66);
    assert_eq!(
        bus.read_doubleword(0x8000).unwrap(),
        [0xaabb_ccdd, 0x55ff_7788]
    );
    mmu.store_doubleword(&mut bus, 0x2000, 10, [11, 22])
        .unwrap();
    assert_eq!(bus.read_doubleword(0x8000).unwrap(), [11, 22]);
}

#[test]
fn read_only_translation_rejects_atomic_before_target_access_with_or_without_nf() {
    for nf in [0, PCR_NF] {
        for atomic_byte in [false, true] {
            let (mut mmu, mut bus) = translated(0x802, PCR_EN | nf); // ACC=0 read-only
            let result = if atomic_byte {
                mmu.load_store_byte(&mut bus, 0x2001, 11)
            } else {
                mmu.swap(&mut bus, 0x2000, 11, 0xdead_beef)
            };
            if nf != 0 {
                assert_eq!(result.unwrap(), 0);
            } else {
                assert!(
                    matches!(result.unwrap_err(), MemoryError::Translation(e) if e.kind == FaultType::Protection)
                );
            }
            assert_eq!(mmu.fault_status(), 0x3aa); // L3, supervisor store, protection, FAV
            assert_eq!(bus.read32(0x4008).unwrap(), 0x802);
            assert_eq!(
                bus.read_doubleword(0x8000).unwrap(),
                [0x1122_3344, 0x5566_7788]
            );
        }
    }
    // Permission checking must also occur on a TLB hit filled by a read.
    let (mut mmu, mut bus) = translated(0x802, PCR_EN);
    mmu.load(&mut bus, 0x2000, 11, Width::Word).unwrap();
    assert!(mmu.swap(&mut bus, 0x2000, 11, 0).is_err());
    assert_eq!(bus.read32(0x4008).unwrap(), 0x822);
}

#[test]
fn doubleword_nf_failure_is_one_operation_and_does_not_bypass_protection() {
    let (mut mmu, mut bus) = translated(0x802, PCR_EN | PCR_NF);
    mmu.store_doubleword(&mut bus, 0x2000, 11, [1, 2]).unwrap();
    assert_eq!(
        bus.read_doubleword(0x8000).unwrap(),
        [0x1122_3344, 0x5566_7788]
    );
    assert_eq!(bus.read32(0x4008).unwrap(), 0x802);
    assert_eq!(mmu.load_doubleword(&mut bus, 0x3000, 11).unwrap(), [0, 0]);
    assert!(mmu.load_doubleword(&mut bus, 0x2004, 11).is_err());
}

#[test]
fn control_space_doublewords_and_atomics_stop_before_register_side_effects() {
    for op3 in [0x13, 0x17, 0x1d, 0x1f] {
        let mut m = setup(&[mem(op3, 2, 1, 0, Some(4))]);
        m.cpu.write_reg(1, 0x300);
        m.cpu
            .mmu
            .store(&mut m.bus, 0x1300, 4, Width::Word, 0x326)
            .unwrap();
        let before = m.cpu.clone();
        assert!(matches!(
            m.step().unwrap_err().kind,
            FaultKind::UnsupportedMemory(MemoryError::UnsupportedOperation { .. })
        ));
        assert_eq!(m.cpu, before);
    }
}

/// A device-aware bus makes composite operations explicit. The CPU must use
/// these methods, rather than decomposing an exchange into destructive reads.
struct RecordingBus<B> {
    memory: B,
    events: Vec<(&'static str, u64)>,
}

impl<B: Bus> Bus for RecordingBus<B> {
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        self.events.push(("read32", address));
        self.memory.read32(address)
    }
    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        self.events.push(("write32", address));
        self.memory.write32_masked(address, value, mask)
    }
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        self.events.push(("read_doubleword", address));
        self.memory.read_doubleword(address)
    }
    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        self.events.push(("write_doubleword", address));
        self.memory.write_doubleword(address, words)
    }
    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        self.events.push(("swap32", address));
        self.memory.swap32(address, value)
    }
    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        self.events.push(("ldstub8", address));
        self.memory.ldstub8(address)
    }
}

#[test]
fn cpu_uses_indivisible_bus_transactions_for_doublewords_and_exchanges() {
    for (op3, method) in [
        (3, "read_doubleword"),
        (7, "write_doubleword"),
        (0x0d, "ldstub8"),
        (0x0f, "swap32"),
    ] {
        let m = setup(&[mem(op3, 2, 1, 0, None)]);
        let mut cpu = m.cpu;
        cpu.write_reg(1, 0x8000);
        let mut bus = RecordingBus {
            memory: m.bus,
            events: Vec::new(),
        };
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.events, [("read32", 0), (method, 0x8000)]);
    }
}

#[test]
fn failed_atomic_permission_does_not_invoke_target_bus_read_or_exchange() {
    let (mut mmu, memory) = translated(0x802, PCR_EN | PCR_NF);
    let mut bus = RecordingBus {
        memory,
        events: Vec::new(),
    };
    assert_eq!(mmu.swap(&mut bus, 0x2000, 11, 0).unwrap(), 0);
    assert!(bus
        .events
        .iter()
        .all(|(method, address)| *method == "read32" && *address < 0x8000));
}

#[test]
fn flush_orders_self_modifying_code_and_does_not_invalidate_tlb() {
    // Store a new instruction, flush it, then execute it in user mode.
    let mut m = setup(&[
        0x8188_2020, // wr g0, ET, psr
        mem(4, 3, 1, 0, None),
        0x81d8_6000, // flush [g1]
        0x81c0_6000, // jmpl g1, g0
        0x0100_0000,
    ]);
    m.cpu.write_reg(1, 0x100);
    m.cpu.write_reg(3, 0x8410_202a); // or g0, 42, g2
    for _ in 0..6 {
        m.step().unwrap();
    }
    assert_eq!(m.cpu.read_reg(2), 42);
    assert_eq!(m.cpu.psr() & (PSR_ET | PSR_S), PSR_ET);

    // Prime a data translation, change its PTE, then FLUSH: still use the TLB.
    let (mmu, mut bus) = translated(0x80e, PCR_EN);
    bus.write32(0x4000, 0x0e).unwrap(); // identity-mapped instruction page
    bus.write32(0, mem(0, 2, 1, 0, None)).unwrap();
    bus.write32(4, 0x81d8_6000).unwrap(); // flush [g1]
    bus.write32(8, mem(0, 3, 1, 0, None)).unwrap();
    bus.write32(0x9000, 99).unwrap();
    let mut cpu = Cpu::new(0);
    cpu.mmu = mmu;
    cpu.write_reg(1, 0x2000);
    cpu.step(&mut bus).unwrap();
    bus.write32(0x4008, 0x90e).unwrap();
    cpu.step(&mut bus).unwrap();
    cpu.step(&mut bus).unwrap();
    assert_eq!(cpu.read_reg(3), cpu.read_reg(2));
    assert_eq!(cpu.read_reg(3), 0x1122_3344);
}
