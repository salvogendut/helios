use helios::bus::{Bus, BusErrorKind, MapError};
use helios::dram::{Ss5Dram, APERTURE_BYTES, BANK_BYTES};
use helios::machine::{Machine, DEFAULT_RAM_BYTES};
use helios::system_bus::SystemBus;

const MIB: usize = 1024 * 1024;

#[test]
fn dsimm_layouts_reject_impossible_totals_and_preserve_slot_order() {
    for total in [0, 1, 12 * MIB, 240 * MIB, 248 * MIB, 257 * MIB, usize::MAX] {
        assert!(matches!(
            Ss5Dram::from_total(total),
            Err(MapError::InvalidRange)
        ));
    }
    assert!(Ss5Dram::new([16 * MIB; 8]).is_err());
    let memory = Ss5Dram::from_total(DEFAULT_RAM_BYTES).unwrap();
    assert_eq!(
        memory.bank_sizes(),
        [BANK_BYTES, BANK_BYTES, BANK_BYTES, BANK_BYTES, 0, 0, 0, 0]
    );
    let memory = Ss5Dram::from_total(48 * MIB).unwrap();
    assert_eq!(
        memory.bank_sizes(),
        [BANK_BYTES, 8 * MIB, 8 * MIB, 0, 0, 0, 0, 0]
    );
}

#[test]
fn small_banks_mirror_all_lanes_and_exchanges_without_aliasing_other_banks() {
    let mut memory = Ss5Dram::new([8 * MIB, 0, BANK_BYTES, 0, 0, 0, 0, 0]).unwrap();
    memory.write32(0, 0x1234_5678).unwrap();
    for mirror in 0..4 {
        let base = mirror * 8 * MIB as u64;
        assert_eq!(memory.read32(base).unwrap(), 0x1234_5678);
        assert_eq!(memory.read8(base + 1).unwrap(), 0x34);
        assert_eq!(memory.read16(base + 2).unwrap(), 0x5678);
    }
    memory.write8(8 * MIB as u64 + 1, 0xab).unwrap();
    memory.write16(16 * MIB as u64 + 2, 0xcdef).unwrap();
    assert_eq!(
        memory.swap32(24 * MIB as u64, 0x0102_0304).unwrap(),
        0x12ab_cdef
    );
    assert_eq!(memory.ldstub8(8 * MIB as u64 + 2).unwrap(), 3);
    assert_eq!(memory.read32(0).unwrap(), 0x0102_ff04);
    memory
        .write_doubleword(32 * MIB as u64 - 8, [0x89ab_cdef, 0x0123_4567])
        .unwrap();
    assert_eq!(
        memory.read_doubleword(8 * MIB as u64 - 8).unwrap(),
        [0x89ab_cdef, 0x0123_4567]
    );
    let other = 64 * MIB as u64;
    memory.write32(other, 0xdead_beef).unwrap();
    assert_eq!(memory.read32(other + 8 * MIB as u64).unwrap(), 0);
    assert_eq!(memory.read32(0).unwrap(), 0x0102_ff04);
}

#[test]
fn empty_bank_accesses_complete_without_allocating_or_retaining_data() {
    let mut memory = Ss5Dram::new([0; 8]).unwrap();
    for address in [0, 0x0800_0000, APERTURE_BYTES - 8] {
        memory.write_doubleword(address, [u32::MAX; 2]).unwrap();
        memory
            .write32_masked(address, 0x1234_5678, 0x00ff_00ff)
            .unwrap();
        memory.write8(address + 1, 0xff).unwrap();
        memory.write16(address + 2, 0xffff).unwrap();
        assert_eq!(memory.read_doubleword(address).unwrap(), [0; 2]);
        assert_eq!(memory.read8(address + 1).unwrap(), 0);
        assert_eq!(memory.read16(address + 2).unwrap(), 0);
        assert_eq!(memory.swap32(address, 1).unwrap(), 0);
        assert_eq!(memory.ldstub8(address).unwrap(), 0);
        assert_eq!(memory.read32(address).unwrap(), 0);
        assert_eq!(
            memory.write32(address + 1, 0).unwrap_err().kind,
            BusErrorKind::Misaligned
        );
        assert_eq!(
            memory
                .write_doubleword(address + 4, [1, 2])
                .unwrap_err()
                .kind,
            BusErrorKind::Misaligned
        );
    }
    assert_eq!(
        memory.read32(APERTURE_BYTES).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
}

#[test]
fn images_cannot_silently_wrap_or_disappear_in_mirrors_and_empty_banks() {
    let mut memory = Ss5Dram::new([8 * MIB, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let end = 8 * MIB as u64;
    memory
        .load_image(end - 4, &[0x12, 0x34, 0x56, 0x78])
        .unwrap();
    for (address, bytes) in [
        (end - 2, &[1, 2, 3][..]),
        (end, &[1][..]),
        (32 * MIB as u64, &[1][..]),
    ] {
        assert_eq!(
            memory.load_image(address, bytes).unwrap_err().kind,
            BusErrorKind::Unmapped
        );
    }
    assert_eq!(memory.read32(end - 4).unwrap(), 0x1234_5678);
    assert_eq!(memory.read32(0).unwrap(), 0);
}

#[test]
fn dram_aperture_is_reserved_and_installation_rejects_overlap_atomically() {
    let mut bus = SystemBus::default();
    bus.install_ss5_dram(Ss5Dram::new([0; 8]).unwrap()).unwrap();
    for address in [0, 0x0800_0000, APERTURE_BYTES - 4] {
        assert_eq!(bus.map_ram(address, 4), Err(MapError::Overlap));
        assert_eq!(bus.map_rom(address, &[0; 4]), Err(MapError::Overlap));
    }
    assert_eq!(
        bus.install_ss5_dram(Ss5Dram::new([0; 8]).unwrap()),
        Err(MapError::Overlap)
    );
    assert_eq!(bus.map_ram(u64::MAX - 3, 8), Err(MapError::InvalidRange));
    bus.map_ram(APERTURE_BYTES, 4).unwrap();
    let mut bus = SystemBus::default();
    bus.map_ram(0x0800_0000, 4).unwrap();
    assert_eq!(
        bus.install_ss5_dram(Ss5Dram::new([0; 8]).unwrap()),
        Err(MapError::Overlap)
    );
    assert!(bus.dram().is_none());
    assert_eq!(bus.read32(0).unwrap_err().kind, BusErrorKind::Unmapped);
}

#[test]
fn ss5_cpu_probes_empty_banks_and_reset_preserves_populated_storage() {
    // sta g2,[g1] ASI20; lda [g1] ASI20,g3; sta g2,[g4] ASI20 (software reset)
    let prom: Vec<_> = [0xc4a0_4400u32, 0xc680_4400, 0xc4a1_0400]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    let mut m = Machine::ss5(DEFAULT_RAM_BYTES, &prom).unwrap();
    m.cpu.write_reg(1, 0x0800_0000);
    m.cpu.write_reg(2, 1);
    m.cpu.write_reg(4, 0x71f0_0000);
    m.bus.write32(0x07ff_fffc, 0xfeed_face).unwrap();
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(m.cpu.read_reg(3), 0);
    assert_eq!(m.cpu.mmu.fault_status(), 0);
    assert!(matches!(
        m.step().unwrap(),
        helios::cpu::Step::SoftwareReset { .. }
    ));
    assert_eq!(m.bus.read32(0x07ff_fffc).unwrap(), 0xfeed_face);
    m.reset();
    assert_eq!(m.bus.read32(0x07ff_fffc).unwrap(), 0xfeed_face);
    assert_eq!(m.bus.read32(0x0800_0000).unwrap(), 0);
    // The synthetic harness keeps its precise, contiguous RAM boundary.
    let mut synthetic = Machine::sun4m(4096, &prom).unwrap();
    assert_eq!(
        synthetic.bus.read32(4096).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
}
