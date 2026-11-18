use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub struct ParkingProfile {
    pub token_success: u64,
    pub token_rejected: u64,
    pub token_success_ns: u64,
    pub token_rejected_ns: u64,
    pub token_ns: u64,
    pub validate_success: u64,
    pub validate_rejected: u64,
    pub validate_success_ns: u64,
    pub validate_rejected_ns: u64,
    pub validate_ns: u64,
    pub advance_success: u64,
    pub advance_rejected: u64,
    pub advance_success_ns: u64,
    pub advance_rejected_ns: u64,
    pub advance_ns: u64,
    pub proof_ok: u64,
    pub proof_failed: u64,
    pub proof_ns: u64,
    pub readiness_ok: u64,
    pub readiness_failed: u64,
    pub readiness_ns: u64,
    pub digest_calls: u64,
    pub digest_ns: u64,
    pub arbitration_calls: u64,
    pub arbitration_ns: u64,
    pub snapshot_calls: u64,
    pub snapshot_ns: u64,
    pub snapshot_eq_ok: u64,
    pub snapshot_eq_failed: u64,
    pub snapshot_eq_ns: u64,
    pub deadline_calls: u64,
    pub deadline_ns: u64,
    pub advancement_calls: u64,
    pub advancement_ns: u64,
}

thread_local! {
    static PROFILE: RefCell<ParkingProfile> = const { RefCell::new(ParkingProfile::new()) };
}

impl ParkingProfile {
    pub const fn new() -> Self {
        Self {
            token_success: 0,
            token_rejected: 0,
            token_success_ns: 0,
            token_rejected_ns: 0,
            token_ns: 0,
            validate_success: 0,
            validate_rejected: 0,
            validate_success_ns: 0,
            validate_rejected_ns: 0,
            validate_ns: 0,
            advance_success: 0,
            advance_rejected: 0,
            advance_success_ns: 0,
            advance_rejected_ns: 0,
            advance_ns: 0,
            proof_ok: 0,
            proof_failed: 0,
            proof_ns: 0,
            readiness_ok: 0,
            readiness_failed: 0,
            readiness_ns: 0,
            digest_calls: 0,
            digest_ns: 0,
            arbitration_calls: 0,
            arbitration_ns: 0,
            snapshot_calls: 0,
            snapshot_ns: 0,
            snapshot_eq_ok: 0,
            snapshot_eq_failed: 0,
            snapshot_eq_ns: 0,
            deadline_calls: 0,
            deadline_ns: 0,
            advancement_calls: 0,
            advancement_ns: 0,
        }
    }
}

pub fn reset() {
    PROFILE.with_borrow_mut(|profile| *profile = ParkingProfile::new());
}

pub fn snapshot() -> ParkingProfile {
    PROFILE.with_borrow(Clone::clone)
}

pub(crate) fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

pub(crate) fn token(ns: u64, success: bool) {
    PROFILE.with_borrow_mut(|profile| {
        let (calls, split) = if success {
            (&mut profile.token_success, &mut profile.token_success_ns)
        } else {
            (&mut profile.token_rejected, &mut profile.token_rejected_ns)
        };
        *calls += 1;
        *split += ns;
        profile.token_ns += ns;
    });
}

pub(crate) fn validate(ns: u64, success: bool) {
    PROFILE.with_borrow_mut(|profile| {
        let (calls, split) = if success {
            (
                &mut profile.validate_success,
                &mut profile.validate_success_ns,
            )
        } else {
            (
                &mut profile.validate_rejected,
                &mut profile.validate_rejected_ns,
            )
        };
        *calls += 1;
        *split += ns;
        profile.validate_ns += ns;
    });
}

pub(crate) fn advance(ns: u64, success: bool) {
    PROFILE.with_borrow_mut(|profile| {
        let (calls, split) = if success {
            (
                &mut profile.advance_success,
                &mut profile.advance_success_ns,
            )
        } else {
            (
                &mut profile.advance_rejected,
                &mut profile.advance_rejected_ns,
            )
        };
        *calls += 1;
        *split += ns;
        profile.advance_ns += ns;
    });
}

pub(crate) fn proof(ns: u64, success: bool) {
    PROFILE.with_borrow_mut(|profile| {
        if success {
            profile.proof_ok += 1;
        } else {
            profile.proof_failed += 1;
        }
        profile.proof_ns += ns;
    });
}

pub(crate) fn readiness(ns: u64, success: bool) {
    PROFILE.with_borrow_mut(|profile| {
        if success {
            profile.readiness_ok += 1;
        } else {
            profile.readiness_failed += 1;
        }
        profile.readiness_ns += ns;
    });
}

pub(crate) fn digest(ns: u64) {
    PROFILE.with_borrow_mut(|profile| {
        profile.digest_calls += 1;
        profile.digest_ns += ns;
    });
}

pub(crate) fn arbitration(ns: u64) {
    PROFILE.with_borrow_mut(|profile| {
        profile.arbitration_calls += 1;
        profile.arbitration_ns += ns;
    });
}

pub(crate) fn snapshot_cpu(ns: u64) {
    PROFILE.with_borrow_mut(|profile| {
        profile.snapshot_calls += 1;
        profile.snapshot_ns += ns;
    });
}

pub(crate) fn snapshot_eq(ns: u64, equal: bool) {
    PROFILE.with_borrow_mut(|profile| {
        if equal {
            profile.snapshot_eq_ok += 1;
        } else {
            profile.snapshot_eq_failed += 1;
        }
        profile.snapshot_eq_ns += ns;
    });
}

pub(crate) fn deadline(ns: u64) {
    PROFILE.with_borrow_mut(|profile| {
        profile.deadline_calls += 1;
        profile.deadline_ns += ns;
    });
}

pub(crate) fn advancement(ns: u64) {
    PROFILE.with_borrow_mut(|profile| {
        profile.advancement_calls += 1;
        profile.advancement_ns += ns;
    });
}

pub(crate) fn timed<T>(f: impl FnOnce() -> T, record: impl FnOnce(u64, &T)) -> T {
    let started = Instant::now();
    let result = f();
    record(nanos(started.elapsed()), &result);
    result
}
