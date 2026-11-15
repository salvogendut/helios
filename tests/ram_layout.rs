use helios::dram::DramLayout;
use helios::machine::{InstalledVideo, Machine, MachineError, MachineProfile, DEFAULT_RAM_BYTES};

const MIB: usize = 1024 * 1024;

fn reachable_ss5_totals(max_mib: usize) -> Vec<usize> {
    let mut reachable = vec![false; max_mib + 1];
    reachable[0] = true;
    for _socket in 0..8 {
        let previous = reachable.clone();
        for total in 0..=max_mib {
            if previous[total] {
                for module in [8, 32] {
                    if total + module <= max_mib {
                        reachable[total + module] = true;
                    }
                }
            }
        }
    }
    reachable
        .iter()
        .enumerate()
        .filter_map(|(total, &valid)| (total != 0 && valid).then_some(total))
        .collect()
}

fn reachable_ss20_totals(sockets: usize, max_mib: usize) -> Vec<usize> {
    let mut reachable = vec![false; max_mib + 1];
    reachable[0] = true;
    for _socket in 0..sockets {
        let previous = reachable.clone();
        for total in 0..=max_mib {
            if previous[total] {
                for module in [16, 32, 64] {
                    if total + module <= max_mib {
                        reachable[total + module] = true;
                    }
                }
            }
        }
    }
    reachable
        .iter()
        .enumerate()
        .filter_map(|(total, &valid)| (total != 0 && valid).then_some(total))
        .collect()
}
#[test]
fn valid_ss5_totals_follow_eight_socket_8_and_32_mib_geometry() {
    let maximum_mib = 272;
    let reachable = reachable_ss5_totals(maximum_mib);
    let totals = DramLayout::valid_ss5_totals();
    assert_eq!(
        totals,
        reachable
            .iter()
            .map(|total| total * MIB)
            .collect::<Vec<_>>()
    );
    assert!(totals.windows(2).all(|pair| pair[0] < pair[1]));
    for total_mib in 0..=maximum_mib {
        let expected = reachable.contains(&total_mib);
        let result = DramLayout::ss5_total(total_mib * MIB);
        assert_eq!(result.is_ok(), expected, "{total_mib} MiB");
        if let Ok(layout) = result {
            assert_eq!(layout.total_bytes(), total_mib * MIB);
            assert!(layout
                .bank_sizes()
                .iter()
                .all(|&size| [0, 8 * MIB, 32 * MIB].contains(&size)));
        }
    }
    assert!(totals.contains(&(8 * MIB)));
    assert!(totals.contains(&(232 * MIB)));
    assert!(totals.contains(&(256 * MIB)));
    assert!(!totals.contains(&(240 * MIB)));
    for total in [4, 7 * MIB, usize::MAX, usize::MAX - 7] {
        assert!(DramLayout::ss5_total(total).is_err(), "{total} must fail");
    }
}

#[test]
fn valid_ss20_totals_respect_vsimm_bank_reservation() {
    let maximum_mib = 528;
    for sockets in 0..=8 {
        let reachable = reachable_ss20_totals(sockets, maximum_mib);
        let totals = DramLayout::valid_ss20_totals(sockets);
        assert_eq!(
            totals,
            reachable
                .iter()
                .map(|total| total * MIB)
                .collect::<Vec<_>>(),
            "{sockets} sockets"
        );
        for total_mib in 0..=maximum_mib {
            let expected = reachable.contains(&total_mib);
            let result = DramLayout::ss20_total(total_mib * MIB, sockets);
            assert_eq!(
                result.is_ok(),
                expected,
                "{total_mib} MiB / {sockets} sockets"
            );
            if let Ok(layout) = result {
                assert_eq!(layout.total_bytes(), total_mib * MIB);
                assert!(layout
                    .bank_sizes()
                    .iter()
                    .all(|&size| [0, 16 * MIB, 32 * MIB, 64 * MIB].contains(&size)));
                if sockets < 8 {
                    assert_eq!(layout.bank_sizes()[sockets..], [0; 8][sockets..]);
                }
            }
        }
    }

    let sx = DramLayout::valid_ss20_totals(7);
    assert!(sx.contains(&(16 * MIB)));
    assert!(sx.contains(&(416 * MIB)));
    assert!(sx.contains(&(448 * MIB)));
    assert!(!sx.contains(&(432 * MIB)));
    assert!(!sx.contains(&(512 * MIB)));
    let layout = DramLayout::ss20_total(448 * MIB, 7).unwrap();
    assert_eq!(layout.bank_sizes()[7], 0);

    let no_sx = DramLayout::valid_ss20_totals(8);
    assert!(no_sx.contains(&(480 * MIB)));
    assert!(no_sx.contains(&(512 * MIB)));
    assert!(!no_sx.contains(&(496 * MIB)));
    assert!(!no_sx.contains(&(528 * MIB)));
    assert!(DramLayout::valid_ss20_totals(9).is_empty());
    assert!(DramLayout::ss20_total(16 * MIB, 9).is_err());
    assert!(DramLayout::ss20_total(432 * MIB, 7).is_err());
    for total in [4, 8 * MIB, 496 * MIB, usize::MAX] {
        assert!(
            DramLayout::ss20_total(total, 8).is_err(),
            "{total} must fail"
        );
    }
}

#[test]
fn validated_layouts_expose_bank7_reservation_without_guest_allocation() {
    let sx = DramLayout::ss20_total(448 * MIB, 7).unwrap();
    let mut seven_banks = [64 * MIB; 8];
    seven_banks[7] = 0;
    assert_eq!(sx.bank_sizes(), seven_banks);
    assert_eq!(sx.total_bytes(), 448 * MIB);
    assert_eq!(sx.aperture_bytes(), 512 * MIB as u64);

    let no_sx = DramLayout::ss20_total(512 * MIB, 8).unwrap();
    assert_eq!(no_sx.bank_sizes(), [64 * MIB; 8]);
    assert_eq!(no_sx.total_bytes(), 512 * MIB);
}

#[test]
fn profiles_select_defaults_and_reject_impossible_video_ram_combinations() {
    assert_eq!(MachineProfile::Ss5.default_video(), InstalledVideo::None);
    assert_eq!(MachineProfile::Ss20.default_video(), InstalledVideo::Sx);
    assert_eq!(
        MachineProfile::Axil320.default_video(),
        InstalledVideo::None
    );
    assert_eq!(
        MachineProfile::Ss5
            .valid_ram_totals(InstalledVideo::None)
            .unwrap(),
        DramLayout::valid_ss5_totals()
    );
    assert_eq!(
        MachineProfile::Ss20
            .valid_ram_totals(InstalledVideo::Sx)
            .unwrap(),
        DramLayout::valid_ss20_totals(7)
    );
    assert_eq!(
        MachineProfile::Ss20
            .valid_ram_totals(InstalledVideo::None)
            .unwrap(),
        DramLayout::valid_ss20_totals(8)
    );
    assert_eq!(
        MachineProfile::Axil320
            .valid_ram_totals(InstalledVideo::None)
            .unwrap(),
        DramLayout::valid_axil_totals()
    );
    assert!(MachineProfile::Axil320
        .valid_ram_totals(InstalledVideo::Sx)
        .is_err());

    let ss5 = MachineProfile::Ss5
        .ram_layout(232 * MIB, InstalledVideo::None)
        .unwrap();
    assert_eq!(ss5.total_bytes(), 232 * MIB);
    assert!(matches!(
        MachineProfile::Ss5.ram_layout(232 * MIB, InstalledVideo::Sx),
        Err(MachineError::InvalidVideo { .. })
    ));
    assert!(matches!(
        MachineProfile::Axil320.ram_layout(512 * MIB, InstalledVideo::Sx),
        Err(MachineError::InvalidVideo { .. })
    ));

    let sx = MachineProfile::Ss20
        .ram_layout(448 * MIB, InstalledVideo::Sx)
        .unwrap();
    let mut seven_banks = [64 * MIB; 8];
    seven_banks[7] = 0;
    assert_eq!(sx.bank_sizes(), seven_banks);
    assert!(matches!(
        MachineProfile::Ss20.ram_layout(512 * MIB, InstalledVideo::Sx),
        Err(MachineError::InvalidRamCapacity { .. })
    ));
    let error = MachineProfile::Ss20
        .ram_layout(512 * MIB, InstalledVideo::Sx)
        .unwrap_err()
        .to_string();
    assert!(error.contains("SPARCstation 20"));
    assert!(error.contains("448 MiB"));
    let error = MachineProfile::Axil320
        .ram_layout(512 * MIB, InstalledVideo::Sx)
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not implement SX VSIMM"));

    let no_sx = MachineProfile::Ss20
        .ram_layout(512 * MIB, InstalledVideo::None)
        .unwrap();
    assert_eq!(no_sx.bank_sizes(), [64 * MIB; 8]);
    let axil = MachineProfile::Axil320
        .ram_layout(512 * MIB, InstalledVideo::None)
        .unwrap();
    assert_eq!(axil.bank_sizes(), [64 * MIB; 8]);
}

#[test]
fn axil_totals_use_populations_recognized_by_the_original_219_rom() {
    // Independently enumerate per-socket choices, including the empty socket.
    let mut reachable = vec![false; 529];
    reachable[0] = true;
    for _ in 0..8 {
        let previous = reachable.clone();
        for total in 0..reachable.len() {
            reachable[total] = [0, 16, 64]
                .iter()
                .any(|&module| total >= module && previous[total - module]);
        }
    }
    let totals = MachineProfile::Axil320
        .valid_ram_totals(InstalledVideo::None)
        .unwrap();
    for (mib, &can_populate) in reachable.iter().enumerate() {
        let expected = mib != 0 && can_populate;
        assert_eq!(totals.contains(&(mib * MIB)), expected, "{mib} MiB");
        let layout = MachineProfile::Axil320.ram_layout(mib * MIB, InstalledVideo::None);
        assert_eq!(layout.is_ok(), expected, "{mib} MiB");
        if let Ok(layout) = layout {
            assert_eq!(layout.total_bytes(), mib * MIB);
            assert!(layout
                .bank_sizes()
                .iter()
                .all(|&size| [0, 16 * MIB, 64 * MIB].contains(&size)));
        }
    }
    assert_eq!(
        DramLayout::axil_total(32 * MIB).unwrap().bank_sizes(),
        [16 * MIB, 16 * MIB, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        DramLayout::axil_total(416 * MIB).unwrap().bank_sizes(),
        [
            64 * MIB,
            64 * MIB,
            64 * MIB,
            64 * MIB,
            64 * MIB,
            64 * MIB,
            16 * MIB,
            16 * MIB
        ]
    );
    for bytes in [8 * MIB, 432 * MIB, 480 * MIB, usize::MAX, usize::MAX - 15] {
        assert!(DramLayout::axil_total(bytes).is_err());
    }
    assert_eq!(
        DramLayout::axil_total(512 * MIB).unwrap().bank_sizes(),
        [64 * MIB; 8]
    );
}

#[test]
fn axil_small_banks_answer_the_roms_16_mib_alias_probe() {
    use helios::bus::Bus;
    use helios::dram::Ss5Dram;
    let mut dram = Ss5Dram::from_layout(DramLayout::axil_total(32 * MIB).unwrap()).unwrap();
    // Values and offsets are from the original Axil 2.19 memory sizing probe.
    // Each 16 MiB bank must alias +16 MiB back to its base. The former contiguous
    // 32 MiB simulation left 0x04000000 at the base and incorrectly reported 64 MiB.
    for base in [0, 64 * MIB as u64] {
        dram.write32(base, 0x0400_0000).unwrap();
        dram.write32(base + 16 * MIB as u64, 0x0100_0000).unwrap();
        assert_eq!(dram.read32(base).unwrap(), 0x0100_0000);
    }
}

#[test]
fn explicit_video_construction_preserves_defaults_and_installs_sx_only_when_asked() {
    let rom = 0u32.to_be_bytes();

    let ss5 = Machine::with_profile(
        MachineProfile::Ss5,
        DEFAULT_RAM_BYTES,
        &rom,
        MachineProfile::Ss5.default_nvram(),
    )
    .unwrap();
    assert!(ss5.bus.sx().is_none());
    assert_eq!(
        ss5.bus.dram().unwrap().bank_sizes(),
        [32 * MIB, 32 * MIB, 32 * MIB, 32 * MIB, 0, 0, 0, 0]
    );

    let ss20 = Machine::with_profile(
        MachineProfile::Ss20,
        DEFAULT_RAM_BYTES,
        &rom,
        MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    assert!(ss20.bus.sx().is_some());
    assert_eq!(
        ss20.bus.dram().unwrap().bank_sizes(),
        [64 * MIB, 64 * MIB, 0, 0, 0, 0, 0, 0]
    );

    let ss20_no_sx = Machine::ss20_without_sx(
        DEFAULT_RAM_BYTES,
        &rom,
        MachineProfile::Ss20.default_nvram(),
    )
    .unwrap();
    assert!(ss20_no_sx.bus.sx().is_none());
    assert_eq!(
        ss20_no_sx.bus.dram().unwrap().bank_sizes(),
        [64 * MIB, 64 * MIB, 0, 0, 0, 0, 0, 0]
    );

    let axil = Machine::with_profile(
        MachineProfile::Axil320,
        DEFAULT_RAM_BYTES,
        &rom,
        MachineProfile::Axil320.default_nvram(),
    )
    .unwrap();
    assert!(axil.bus.sx().is_none());
    assert_eq!(
        axil.bus.dram().unwrap().bank_sizes(),
        [64 * MIB, 64 * MIB, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn invalid_video_and_prom_are_rejected_before_guest_allocation() {
    let rom = 0u32.to_be_bytes();
    assert!(matches!(
        Machine::with_profile_video(
            MachineProfile::Ss20,
            512 * MIB,
            &rom,
            MachineProfile::Ss20.default_nvram(),
            InstalledVideo::Sx,
        ),
        Err(MachineError::InvalidRamCapacity { .. })
    ));
    assert!(matches!(
        Machine::with_profile_video(
            MachineProfile::Ss5,
            16 * MIB,
            &rom,
            MachineProfile::Ss5.default_nvram(),
            InstalledVideo::Sx,
        ),
        Err(MachineError::InvalidVideo { .. })
    ));
    assert!(matches!(
        Machine::with_profile_video(
            MachineProfile::Ss20,
            512 * MIB,
            &[],
            MachineProfile::Ss20.default_nvram(),
            InstalledVideo::None,
        ),
        Err(MachineError::InvalidPromSize)
    ));
}
