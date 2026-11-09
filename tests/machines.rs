use helios::bus::{Bus, BusErrorKind, MemoryBus};
use helios::devices::ecc::Ecc;
use helios::devices::nvram::{Nvram, IDPROM_OFFSET};
use helios::dram::Ss5Dram;
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{MmuConfig, Srmmu, Width};

fn idprom(nvram: &Nvram) -> [u8; 16] {
    let image = nvram.image();
    image[IDPROM_OFFSET..IDPROM_OFFSET + 16].try_into().unwrap()
}

#[test]
fn ss20_and_axil_idprams_carry_their_documented_identities() {
    let ss20 = idprom(&MachineProfile::Ss20.default_nvram());
    assert_eq!(&ss20[..8], &[1, 0x72, 0x08, 0x00, 0x20, 0xc0, 0xff, 0xee]);
    assert_eq!(&ss20[12..15], &[0xc0, 0xff, 0xee]);
    assert_eq!(ss20.iter().fold(0, |sum, byte| sum ^ byte), 0);

    let axil = idprom(&MachineProfile::Axil320.default_nvram());
    assert_eq!(&axil[..8], &[1, 0x72, 0x00, 0x00, 0x3c, 0x13, 0xde, 0xad]);
    assert_eq!(&axil[12..15], &[0xc0, 0xff, 0xee]);
    assert_eq!(axil.iter().fold(0, |sum, byte| sum ^ byte), 0);
}

#[test]
fn ss20_layout_uses_the_high_sun4m_map_and_reference_mmu() {
    let layout = MachineProfile::Ss20.layout();
    assert_eq!(layout.prom, 0xf_f000_0000);
    assert_eq!(layout.io_control, 0xf_e000_0000);
    assert_eq!(layout.serial, 0xf_f110_0000);
    assert_eq!(layout.nvram, 0xf_f120_0000);
    assert_eq!(layout.esp, 0xe_f080_0000);
    assert_eq!(layout.ethernet, 0xe_f0c0_0000);
    assert_eq!(layout.ecc, Some(0x0f_0000_0000));
    assert_eq!(layout.cgsix, 0xe_2000_0000);
    assert_eq!(layout, MachineProfile::Axil320.layout());

    let mut ss20 = Machine::with_profile(
        MachineProfile::Ss20,
        16 * 1024 * 1024,
        &0u32.to_be_bytes(),
        MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    assert!(ss20.bus.sx().is_some());
    let mut axil = Machine::with_profile(
        MachineProfile::Axil320,
        16 * 1024 * 1024,
        &0u32.to_be_bytes(),
        MachineProfile::Axil320.default_nvram(),
    )
    .unwrap();
    assert!(axil.bus.sx().is_none());
    let _ = (&mut ss20, &mut axil);

    let mmu = MachineProfile::Ss20.mmu();
    assert_eq!(mmu, MmuConfig::ss20());
    assert_eq!(mmu.prom_base, layout.prom);
    assert_eq!(mmu.boot_mode_bit, 1 << 13);
    assert!(mmu.reference_mmu);
}

#[test]
fn ss20_console_and_sbus_placeholder_windows_return_zero() {
    let profile = MachineProfile::Ss20;
    let mut machine = Machine::with_profile(
        profile,
        16 * 1024 * 1024,
        &0u32.to_be_bytes(),
        profile.default_nvram(),
    )
    .unwrap();
    // SX, DBRI PROM/register windows and the legacy high VSIMM aliases are
    // decoded as empty slots (QEMU uses unimplemented devices at these bases).
    for (asi, address) in [
        (0x2f, 0x8000_0000), // 0xf_8000_0000 SX
        (0x2e, 0xe000_1000), // 0xe_e000_1000 DBRI PROM
        (0x2e, 0xe001_0000), // 0xe_e001_0000 DBRI registers
        (0x2f, 0xf8c0_0000), // 0xf_f8c0_0000 legacy high VSIMM alias
    ] {
        assert_eq!(
            machine
                .cpu
                .mmu
                .load(&mut machine.bus, address, asi, Width::Word)
                .unwrap(),
            0
        );
    }
}

#[test]
fn ss20_and_axil_fetch_from_their_high_prom_aperture_after_reset() {
    for profile in [MachineProfile::Ss20, MachineProfile::Axil320] {
        let mut machine = Machine::with_profile(
            profile,
            16 * 1024 * 1024,
            &0x0100_0000u32.to_be_bytes(),
            profile.default_nvram(),
        )
        .unwrap();
        let step = machine.step().unwrap();
        assert!(matches!(step, helios::cpu::Step::Executed { pc: 0, .. }));
        assert_eq!(machine.cpu.pc(), 4);
        assert_eq!(machine.cpu.npc(), 8);
    }
}

#[test]
fn asi_2f_passthrough_reaches_high_physical_memory() {
    let mut mmu = Srmmu::reset_with(MmuConfig::ss20());
    let mut bus = MemoryBus::default();
    bus.map_ram(0x0f_0000_0000, 0x2000).unwrap();
    bus.write32(0x0f_0000_1000, 0x1234_5678).unwrap();
    // Translation is disabled at reset; passthrough ASIs still address the
    // full 36-bit physical space encoded in the ASI's low nibble.
    assert_eq!(
        mmu.load(&mut bus, 0x1000, 0x2f, Width::Word).unwrap(),
        0x1234_5678
    );
}

#[test]
fn ss20_dsimm_banks_are_64_mib_and_empty_banks_read_zero() {
    let mut dram = Ss5Dram::from_ss20_total(128 * 1024 * 1024).unwrap();
    assert_eq!(dram.bank_sizes(), [64 << 20, 64 << 20, 0, 0, 0, 0, 0, 0]);
    assert_eq!(dram.aperture_bytes(), 8 * (64 << 20) as u64);
    dram.write32(0, 0xabcd_ef01).unwrap();
    assert_eq!(dram.read32(0).unwrap(), 0xabcd_ef01);
    assert_eq!(dram.read32(0x1c00_0000).unwrap(), 0);
    assert_eq!(
        dram.read32(0x2000_0000).unwrap_err().kind,
        BusErrorKind::Unmapped
    );
}

#[test]
fn smc_ecc_registers_retain_configuration_and_identity() {
    let mut ecc = Ecc::new(0x2000_0000);
    assert_eq!(ecc.read(0).unwrap(), 0x2000_0000);
    assert_eq!(ecc.read(4).unwrap(), 0x20);
    assert_eq!(ecc.read(0x10).unwrap(), 0x07c0_0000);
    ecc.write(0, 0xffff_ffff).unwrap();
    assert_eq!(ecc.read(0).unwrap(), 0x2000_0bff);
    ecc.write(0x10, 0x1234_5678).unwrap();
    assert_eq!(ecc.read(0x10).unwrap(), 0x1234_5678);
    ecc.write(0x24, 0xdead_beef).unwrap();
    assert_eq!(ecc.read(0x24).unwrap(), 0xdead_beef);
    assert_eq!(ecc.read(2).unwrap_err(), BusErrorKind::Misaligned);
}
