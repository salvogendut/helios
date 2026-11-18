#![cfg(feature = "parking-profile")]

mod support;

use helios::bus::Bus;
use helios::machine::{ParkingError, SOLARIS_IDLE_NPC, SOLARIS_IDLE_PC};
use helios::parking_profile as profile;
use support::idle_machine;

fn assert_no_proof_or_digest() {
    let p = profile::snapshot();
    assert_eq!(p.token_rejected, 1);
    assert_eq!(p.token_success, 0);
    assert_eq!(
        p.proof_ok + p.proof_failed,
        0,
        "cheap rejection must short-circuit the mapped proof"
    );
    assert_eq!(p.readiness_ok + p.readiness_failed, 0);
    assert_eq!(p.digest_calls, 0);
    assert_eq!(p.snapshot_calls, 0);
}

#[test]
fn wrong_pc_short_circuits_mapped_proof() {
    let mut m = idle_machine();
    m.cpu.set_pc_npc(SOLARIS_IDLE_PC + 4, SOLARIS_IDLE_NPC);
    profile::reset();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    assert_no_proof_or_digest();
}

#[test]
fn wrong_npc_short_circuits_mapped_proof() {
    let mut m = idle_machine();
    m.cpu.set_pc_npc(SOLARIS_IDLE_PC, SOLARIS_IDLE_NPC + 4);
    profile::reset();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    assert_no_proof_or_digest();
}

#[test]
fn reset_request_short_circuits_mapped_proof() {
    let mut m = idle_machine();
    m.bus.devices_mut().unwrap().reset.write(1);
    profile::reset();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    assert_no_proof_or_digest();
}

#[test]
fn successful_calls_have_exact_nonduplicated_component_counts() {
    let mut m = idle_machine();
    let before = m.diagnostic_snapshot();
    profile::reset();
    let token = m.parking_token().unwrap();
    assert!(m.validate_parking_token(&token));
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
    let advanced = m.advance_parked(token, 1).unwrap();
    assert_eq!(advanced.advanced_ns, 1);
    assert!(!advanced.service_required);
    let after = m.diagnostic_snapshot();
    assert_eq!(before.cpu, after.cpu);
    assert_eq!(before.bus.memory_digest, after.bus.memory_digest);
    assert_eq!(before.bus.dram_digest, after.bus.dram_digest);
    assert_eq!(m.virtual_ns(), 1);
    let p = profile::snapshot();
    assert_eq!((p.token_success, p.token_rejected), (1, 0));
    assert_eq!((p.validate_success, p.validate_rejected), (2, 0));
    assert_eq!((p.advance_success, p.advance_rejected), (1, 0));
    assert_eq!((p.proof_ok, p.proof_failed), (1, 0));
    assert_eq!((p.readiness_ok, p.readiness_failed), (2, 0));
    assert_eq!(
        (p.digest_calls, p.arbitration_calls, p.snapshot_calls),
        (3, 3, 3)
    );
    assert_eq!((p.snapshot_eq_ok, p.snapshot_eq_failed), (2, 0));
    assert_eq!((p.deadline_calls, p.advancement_calls), (2, 1));
    assert_eq!(p.token_ns, p.token_success_ns + p.token_rejected_ns);
    assert_eq!(
        p.validate_ns,
        p.validate_success_ns + p.validate_rejected_ns
    );
    assert_eq!(p.advance_ns, p.advance_success_ns + p.advance_rejected_ns);
}

#[test]
fn stale_memory_epoch_skips_expensive_validation() {
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.bus.write32(0x8000, 0x1234_5678).unwrap();
    let before = m.diagnostic_snapshot();
    profile::reset();
    assert!(!m.validate_parking_token(&token));
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
    let p = profile::snapshot();
    assert_eq!((p.validate_success, p.validate_rejected), (0, 2));
    assert_eq!((p.advance_success, p.advance_rejected), (0, 1));
    assert_eq!(
        p.digest_calls + p.snapshot_calls + p.deadline_calls + p.advancement_calls,
        0
    );
}

#[test]
fn stale_cpu_snapshot_records_one_failed_equality() {
    let mut m = idle_machine();
    let token = m.parking_token().unwrap();
    m.cpu.write_reg(1, 0x1234_5678);
    profile::reset();
    assert_eq!(m.advance_parked(token, 1).unwrap_err(), ParkingError::Stale);
    let p = profile::snapshot();
    assert_eq!((p.validate_success, p.validate_rejected), (0, 1));
    assert_eq!((p.advance_success, p.advance_rejected), (0, 1));
    assert_eq!(
        (p.digest_calls, p.arbitration_calls, p.snapshot_calls),
        (1, 1, 1)
    );
    assert_eq!((p.snapshot_eq_ok, p.snapshot_eq_failed), (0, 1));
    assert_eq!(
        p.readiness_ok + p.readiness_failed + p.deadline_calls + p.advancement_calls,
        0
    );
}

#[test]
fn code_proof_rejects_before_readiness_or_digest() {
    let mut m = idle_machine();
    m.bus.write32(0x06fa24, 0x0100_0000).unwrap();
    profile::reset();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::NotIdleBoundary
    );
    let p = profile::snapshot();
    assert_eq!((p.token_rejected, p.proof_failed), (1, 1));
    assert_eq!(
        p.proof_ok + p.readiness_ok + p.readiness_failed + p.digest_calls,
        0
    );
}

#[test]
fn pending_interrupt_rejects_after_proof_without_digest() {
    let mut m = idle_machine();
    let layout = m.bus.layout();
    m.bus
        .write32(layout.system_interrupt + 8, u32::MAX)
        .unwrap();
    m.bus.write32(layout.cpu_interrupt + 8, 1 << 28).unwrap();
    profile::reset();
    assert_eq!(
        m.parking_token().unwrap_err(),
        ParkingError::ServiceRequired
    );
    let p = profile::snapshot();
    assert_eq!(
        (p.token_rejected, p.proof_ok, p.readiness_failed),
        (1, 1, 1)
    );
    assert_eq!(p.digest_calls + p.arbitration_calls + p.snapshot_calls, 0);
}

#[test]
fn profile_reset_and_other_threads_preserve_token_and_machine() {
    let m = idle_machine();
    let before = m.diagnostic_snapshot();
    profile::reset();
    let token = m.parking_token().unwrap();
    assert_eq!(profile::snapshot().token_success, 1);
    std::thread::spawn(|| {
        assert_eq!(profile::snapshot().token_success, 0);
        let other = idle_machine();
        other.parking_token().unwrap();
        assert_eq!(profile::snapshot().token_success, 1);
        profile::reset();
    })
    .join()
    .unwrap();
    assert_eq!(profile::snapshot().token_success, 1);
    profile::reset();
    assert_eq!(profile::snapshot().token_success, 0);
    assert!(m.validate_parking_token(&token));
    assert!(before.differences(&m.diagnostic_snapshot()).is_empty());
}

#[test]
fn device_mutation_invalidates_a_token() {
    let mut machine = idle_machine();
    let token = machine.parking_token().unwrap();
    machine.bus.devices_mut().unwrap().auxiliary.write(0x02);
    assert_eq!(
        machine.advance_parked(token, 1).unwrap_err(),
        ParkingError::Stale
    );
}

#[test]
fn staged_serial_payload_invalidates_a_token() {
    let mut machine = idle_machine();
    let token = machine.parking_token().unwrap();
    let serial = &mut machine.bus.devices_mut().unwrap().serial;
    serial.write(4, 3).unwrap();
    serial.write(4, 1).unwrap();
    assert!(serial.receiver_enabled(helios::devices::serial::Channel::A));
    assert!(serial.receive(helios::devices::serial::Channel::A, b'x'));
    assert_eq!(
        machine.advance_parked(token, 1).unwrap_err(),
        ParkingError::Stale
    );
}
