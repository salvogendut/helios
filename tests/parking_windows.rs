use helios::bus::Bus;
use helios::cpu::Step;
use helios::machine::{
    solaris_idle_psr_matches, Machine, ParkingError, SOLARIS_IDLE_NPC, SOLARIS_IDLE_PC,
    SOLARIS_IDLE_PSR,
};

#[path = "support/mod.rs"]
mod support;

const PSR_BASE: u32 = 0x4040_00e0;
const WRPSR: u32 = 0x8188_6000;
const CODE_ADDR: u64 = 0x2000;
const CODE_VADDR: u32 = 0xf000_2000;
const LOOP: [u32; 14] = [
    0xd002_20ec,
    0x80a2_2001,
    0x3280_000f,
    0xd006_2058,
    0x80a2_2000,
    0x0280_0006,
    0x213c_097e,
    0xe004_21e8,
    0x9fc4_0000,
    0x0100_0000,
    0x81c3_e008,
    0x0100_0000,
    0x10bf_fff1,
    0x113c_097e,
];
const PCS: [u32; 14] = [
    0xf006_fa24,
    0xf006_fa28,
    0xf006_fa2c,
    0xf006_fa34,
    0xf006_fa38,
    0xf006_fa3c,
    0xf006_fa40,
    0xf006_fa54,
    0xf006_fa58,
    0xf006_fa5c,
    0xf007_1708,
    0xf007_170c,
    0xf006_fa60,
    0xf006_fa64,
];

fn set_cwp(machine: &mut Machine, cwp: u32) {
    machine.bus.write32(CODE_ADDR, WRPSR).unwrap();
    machine.cpu.write_reg(1, PSR_BASE | cwp);
    machine.cpu.set_pc_npc(CODE_VADDR, CODE_VADDR + 4);
    assert!(matches!(
        machine.cpu.step(&mut machine.bus).unwrap(),
        Step::Executed { .. }
    ));
    machine.cpu.set_pc_npc(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC);
    assert_eq!(machine.cpu.psr(), PSR_BASE | cwp);
}

fn seed_all_windows(machine: &mut Machine) {
    for cwp in 0..8u32 {
        set_cwp(machine, cwp);
        for reg in 8..24u8 {
            machine
                .cpu
                .write_reg(reg, 0x5a00_0000 | (cwp << 8) | u32::from(reg));
        }
    }
}

fn seed_logical_operands(machine: &mut Machine, cwp: u32) {
    set_cwp(machine, cwp);
    machine.cpu.write_reg(8, 0xf025_f800);
    machine.cpu.write_reg(15, 0xf006_fa58);
    machine.cpu.write_reg(16, 0xf007_1708);
    machine.cpu.write_reg(24, 0xf026_de48);
}

fn replay(machine: &mut Machine) -> bool {
    for (pc, word) in PCS.into_iter().zip(LOOP) {
        if machine.cpu.pc() != pc {
            return false;
        }
        match machine.cpu.step(&mut machine.bus) {
            Ok(Step::Executed {
                instruction,
                pc: executed_pc,
                ..
            }) if instruction == word && executed_pc == pc => {}
            _ => return false,
        }
    }
    machine.cpu.pc() == SOLARIS_IDLE_PC && machine.cpu.npc() == SOLARIS_IDLE_NPC
}

fn seeded(cwp: u32) -> Machine {
    let mut machine = support::idle_machine();
    seed_all_windows(&mut machine);
    seed_logical_operands(&mut machine, cwp);
    machine
}

#[test]
fn pre_warmup_device_latch_is_not_invariant() {
    let mut machine = seeded(7);
    let before = machine.diagnostic_snapshot();
    assert!(replay(&mut machine));
    assert_ne!(
        before.bus.devices_digest,
        machine.diagnostic_snapshot().bus.devices_digest,
        "the initial IoControl latch change must remain visible"
    );
}

#[test]
fn idle_loop_preserves_state_for_every_window() {
    for cwp in 0..8 {
        let mut machine = seeded(cwp);
        assert_eq!(machine.cpu.psr(), PSR_BASE | cwp);
        // One ordinary iteration settles the IoControl observation latch and
        // the loop's own o7/l0 values before the equality checkpoint.
        assert!(replay(&mut machine), "CWP {cwp} warmup loop did not replay");
        let before_cpu = machine.cpu.clone();
        let before = machine.diagnostic_snapshot();
        assert!(replay(&mut machine), "CWP {cwp} loop did not replay");
        assert_eq!(machine.cpu.current_window(), cwp as usize);
        assert_eq!(machine.cpu, before_cpu, "CWP {cwp} CPU state changed");
        let after = machine.diagnostic_snapshot();
        let differences = before.differences(&after);
        assert!(
            differences.is_empty(),
            "CWP {cwp} stable snapshot changed: {differences:?}"
        );
    }
}

#[test]
fn bad_current_operand_is_a_counterexample() {
    let mut machine = seeded(7);
    set_cwp(&mut machine, 7);
    machine.cpu.write_reg(8, 0xffff_ffff);
    let before = machine.cpu.clone();
    let replayed = replay(&mut machine);
    assert!(!replayed || machine.cpu != before);
}

#[test]
fn pil_only_is_a_conservative_policy_exclusion() {
    let mut machine = seeded(0);
    set_cwp(&mut machine, 0);
    machine.cpu.write_reg(1, PSR_BASE | (1 << 8)); // PIL differs
    machine.cpu.set_pc_npc(CODE_VADDR, CODE_VADDR + 4);
    machine
        .cpu
        .step(&mut machine.bus)
        .expect("PIL write executes");
    machine.cpu.set_pc_npc(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC);
    assert_ne!(machine.cpu.psr(), SOLARIS_IDLE_PSR);
    assert!(replay(&mut machine), "PIL warmup loop should replay");
    let before = machine.diagnostic_snapshot();
    assert!(replay(&mut machine), "PIL loop should still replay");
    assert!(
        before
            .differences(&machine.diagnostic_snapshot())
            .is_empty(),
        "PIL is not a loop-invariance failure: {:?}",
        before.differences(&machine.diagnostic_snapshot())
    );
    assert_eq!(
        machine.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
}

#[test]
fn correct_operands_in_wrong_window_are_a_counterexample() {
    let mut machine = seeded(7);
    set_cwp(&mut machine, 7);
    machine.cpu.write_reg(8, 0); // corrupt target window
    set_cwp(&mut machine, 6);
    machine.cpu.write_reg(8, 0xf025_f800);
    machine.cpu.write_reg(15, 0xf006_fa58);
    machine.cpu.write_reg(16, 0xf007_1708);
    machine.cpu.write_reg(24, 0xf026_de48);
    set_cwp(&mut machine, 7);
    let before = machine.cpu.clone();
    let replayed = replay(&mut machine);
    assert!(!replayed || machine.cpu != before);
}

#[test]
fn captured_initial_operands_converge_after_one_loop() {
    let mut machine = seeded(7);
    set_cwp(&mut machine, 7);
    machine.cpu.write_reg(15, 0xf006_fa44);
    machine.cpu.write_reg(16, 0xf025_f800);
    assert!(replay(&mut machine), "first unsteady loop should execute");
    let steady = machine.cpu.clone();
    assert!(replay(&mut machine), "second loop should execute");
    assert_eq!(machine.cpu, steady, "second loop is not invariant");
}

#[test]
fn wrong_helper_operand_is_a_counterexample() {
    let mut machine = seeded(7);
    set_cwp(&mut machine, 7);
    machine.cpu.write_reg(16, 0);
    let before = machine.cpu.clone();
    let replayed = replay(&mut machine);
    assert!(!replayed || machine.cpu != before);
}

#[test]
fn production_admits_every_implemented_window() {
    for cwp in 0..8 {
        let mut machine = seeded(cwp);
        let token = machine.parking_token().expect("CWP should be admitted");
        assert!(machine.validate_parking_token(&token));
        let advance = machine.advance_parked(token, 1).expect("advance");
        assert_eq!(advance.advanced_ns, 1);
    }
}

#[test]
fn cwp_change_stales_an_existing_token() {
    let mut machine = seeded(0);
    let token = machine.parking_token().unwrap();
    set_cwp(&mut machine, 1);
    assert!(!machine.validate_parking_token(&token));
    assert_eq!(
        machine.advance_parked(token, 1).unwrap_err(),
        ParkingError::Stale
    );
}

#[test]
fn inactive_windows_and_wim_changes_stale_tokens() {
    let mut machine = seeded(0);
    let token = machine.parking_token().unwrap();
    machine.cpu.write_reg(24, 0x1234_5678); // inactive-window operand
    assert!(!machine.validate_parking_token(&token));

    let mut machine = seeded(0);
    let token = machine.parking_token().unwrap();
    machine
        .cpu
        .set_window_invalid_mask(machine.cpu.window_invalid_mask() ^ 1);
    assert!(!machine.validate_parking_token(&token));
}

#[test]
fn invalid_cwp_encodings_are_not_eligible() {
    for cwp in 8..32 {
        assert!(!solaris_idle_psr_matches(PSR_BASE | cwp));
    }
    assert!(solaris_idle_psr_matches(PSR_BASE | 7));
}

#[test]
fn changed_helper_is_a_counterexample() {
    let mut machine = seeded(7);
    machine.bus.write32(0x071708, 0x0100_0000).unwrap();
    assert!(!replay(&mut machine));
}
