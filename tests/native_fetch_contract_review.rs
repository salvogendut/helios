#![cfg(feature = "jit-native")]

use helios::bus::Bus;
use helios::cpu::{FaultKind, Step};
use helios::machine::{Machine, MachineProfile};
use helios::mmu::{FaultType, Width, PCR_EN};

const VA: u32 = 0x1234_5000;
const LEAF: u64 = 0x4014;

fn mapped(profile: MachineProfile, native: bool) -> Machine {
    let mut m = if profile == MachineProfile::Ss5 {
        Machine::bare_metal(0x10000, 0).unwrap()
    } else {
        Machine::with_profile(
            profile,
            16 * 1024 * 1024,
            &0x0100_0000u32.to_be_bytes(),
            profile.default_nvram(),
        )
        .unwrap()
    };
    for (addr, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (LEAF, 0x088e),
        (0x8000, 0x8200_6001),
    ] {
        m.bus.write32(addr, word).unwrap();
    }
    m.cpu
        .mmu
        .store(&mut m.bus, 0x100, 4, Width::Word, 0x100)
        .unwrap();
    m.cpu
        .mmu
        .store(&mut m.bus, 0, 4, Width::Word, PCR_EN)
        .unwrap();
    m.cpu.set_pc_npc(VA, VA + 4);
    m.set_native_execution_enabled(native);
    m
}

fn permission_sequence(profile: MachineProfile) {
    for native in [false, true] {
        let mut m = mapped(profile, native);
        assert!(matches!(m.step().unwrap(), Step::Executed { .. }));
        assert_eq!(m.cpu.read_reg(1), 1);
        // A physical PTE edit does not by itself invalidate the live ITLB.
        m.bus.write32(LEAF, 0x0882).unwrap();
        m.cpu.set_pc_npc(VA, VA + 4);
        let second = m.step();
        println!("{profile:?} native={native} warmed PTE edit, no flush: {second:?}");
        assert!(matches!(second.unwrap(), Step::Executed { .. }));
        assert_eq!(m.cpu.read_reg(1), 2);
        assert_eq!(m.cpu.mmu.fault_status(), 0);
        assert_eq!(m.native_execution_count(), if native { 2 } else { 0 });

        // ASI 3 page demap removes the cached permission; the next fetch walks.
        m.cpu.mmu.store(&mut m.bus, VA, 3, Width::Word, 0).unwrap();
        m.cpu.set_pc_npc(VA, VA + 4);
        let error = m.step().unwrap_err();
        println!("{profile:?} native={native} after page flush: {error:?}");
        assert!(matches!(error.kind, FaultKind::InstructionMmu(fault)
            if fault.kind == FaultType::Protection && fault.address == VA
                && fault.level == 3 && fault.access_type == 3));
        assert_eq!(m.cpu.read_reg(1), 2);
        assert_eq!(m.native_execution_count(), if native { 2 } else { 0 });
        let status = m.cpu.mmu.fault_status();
        assert_eq!(status, 0x368); // L3, supervisor instruction, protection; no overwrite.
                                   // Instruction protection does not set FAV or overwrite the old SFAR.
        assert_eq!(m.cpu.mmu.fault_address(), 0);
        assert_eq!(m.step().unwrap_err(), error);
        assert_eq!(m.cpu.mmu.fault_status(), status);
    }
}

#[test]
fn ss5_cached_permission_survives_pte_edit_until_page_flush() {
    permission_sequence(MachineProfile::Ss5);
}

#[test]
fn ss20_cached_permission_survives_pte_edit_until_page_flush() {
    permission_sequence(MachineProfile::Ss20);
}

#[test]
fn cold_permission_removal_faults_before_execution() {
    for profile in [MachineProfile::Ss5, MachineProfile::Ss20] {
        for native in [false, true] {
            let mut m = mapped(profile, native);
            m.bus.write32(LEAF, 0x0882).unwrap();
            let error = m.step().unwrap_err();
            assert!(matches!(error.kind, FaultKind::InstructionMmu(fault)
                if fault.kind == FaultType::Protection && fault.address == VA));
            assert_eq!(m.cpu.read_reg(1), 0);
            assert_eq!(m.native_execution_count(), 0);
        }
    }
}
