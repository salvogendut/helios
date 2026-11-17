# Issue #5: core parking token and parked advancement

Status: review patch applied, **pending final acceptance**, based on `2f903cd`.
The independent [token review](performance-issue5-idle-parking-token-review.md)
records the final two MMU-proof details and provides the tested patch now
applied here. The patch checks PTP2 descriptors created by earlier accesses and
uses `entry.context` for global-mapping diagnostic readiness. All 27 review
fixtures are promoted as tracked tests and pass. This remains a core-only
prototype for the observed single-CPU SS-20 Solaris serial-idle boundary.
**No frontend sleeping, host pacing, SMP parking or generic idle recognizer is
included.** Issue #5 remains open. The accepted foundation review remains
[`performance-issue5-idle-parking-foundation-review.md`](performance-issue5-idle-parking-foundation-review.md).

## Scope and boundary

`Machine::parking_token()` issues a token only when all of the following hold:

- machine profile is `Ss20`;
- no secondary CPU is present;
- no CPU error or software-reset request;
- `PC == SOLARIS_IDLE_PC` (`0xf006fa24`);
- `nPC == SOLARIS_IDLE_NPC` (`0xf006fa28`);
- `PSR == SOLARIS_IDLE_PSR` (`0x404000e0`).

These constants are the observed Solaris 2.6 boundary from the foundation
trace. They are intentionally image- and profile-specific; this API is not a
general "PC loop" detector. OBP, SS-5/Axil, SMP and unknown guests return
`NotIdleBoundary` or `Unsupported`. Admission additionally requires the
complete mapped instruction path, TLB execute/read permissions, ordinary RAM,
the three captured load dependencies and no pending/deliverable interrupt or
pre-step service work.

## API

```rust
pub fn parking_token(&self) -> Result<ParkingToken, ParkingError>;
pub fn validate_parking_token(&self, token: &ParkingToken) -> bool;
pub fn advance_parked(
    &mut self,
    token: ParkingToken,
    max_ns: u64,
) -> Result<ParkedAdvance, ParkingError>;

pub struct ParkedAdvance {
    pub requested_ns: u64,
    pub advanced_ns: u64,
    pub clipped: bool,
    pub next_ns: Option<u64>,
    pub service_required: bool,
}
```

Issue and validation are read-only. `ParkingToken` is opaque, does not
implement `Clone`, and is consumed by `advance_parked`, so a token cannot be
reused accidentally. Invalid/unsupported tokens fail before any mutation.

`advance_parked` does **not** execute a CPU step, service DMA/IRQ/handlers,
read host time or sleep. It advances device time by an explicit virtual
interval, clipped to the earliest `timebase().next_ns` event, and returns so the
caller can perform normal service. A caller that gets `service_required` or
`clipped` must service the boundary before requesting another token.

## Admission proof

Before issuing a token, `idle_boundary_proof()` verifies the complete captured
14-instruction path, not just PC/PSR. It requires the exact instruction words,
TLB execute/data permissions, ordinary installed RAM for every code/data
location, the three observed dependencies:

| Dependency | Expected |
| --- | --- |
| `[%o0 + 0xec]`, with `%o0 == 0xf025f800` | `1` at `0xf025f8ec` |
| `[%i0 + 0x58]`, with `%i0 == 0xf026de48` | `0` at `0xf026dea0` |
| `[%l0 + 0x1e8]` after the loop's SETHI | `0xf0071708` at `0xf025f9e8` |

and the `generic_idle_cpu` RETL/NOP helper. It uses the same fast-cache/TLB
selection as normal translation, rejects boot mode, PTP2 override state and a
normal walk/fill requirement, then reads ordinary RAM without a live-MMU walk,
TLB fill, R/M update, fault recording or diagnostic fetch. It also requires
the steady `%o7`/`%l0` values and an already-filled guest-visible data
diagnostic view. A TLB miss, changed code, changed load value, changed helper
target, unsteady register/MMU state or MMIO/non-RAM dependency rejects
issuance and falls back to normal execution. For reference-MMU L1 entries, the
observer also peeks the exact ordinary-RAM source word that `fill_ptp2()`
would read and rejects a PTP descriptor or non-RAM source, so an earlier
access cannot create an override used by a later one. Diagnostic readiness uses
`entry.context` (including supervisor-global entries), matching
`record_diag_view()`. The dependency set is deliberately narrow to this
observed image; runtime mapping changes remain covered by the token epoch/CPU
snapshot checks.

## Invalidation

The token holds a conservative validation snapshot:

- a unique machine owner;
- the machine reset epoch;
- the exact boundary CPU state, compared through the existing deep CPU/MMU/FPU
  snapshot (including TLB/diagnostic and MXCC state);
- the logical and deferred device-time values;
- a bus mutation epoch incremented by bus write attempts, RAM image loads,
  successful mappings and reset;
- a bus configuration epoch incremented by graphics/MXCC installation,
  removal and replacement, so equal per-card generations cannot reuse a token;
- a non-RAM device-state fingerprint covering timers, serial, keyboard/mouse,
  interrupt/DMA state, RTC phase, ESP/SCSI, Ethernet, CGsix and SX.

This rejects stale tokens after CPU/MMU/FPU changes, RAM writes, mapping changes,
device register/direct-device changes, external virtual-time advancement,
deferred-time flushing, reset and cross-machine use. The checks are
conservative: even an unsuccessful or read-only-looking bus store invalidates
the token, and writes to unrelated RAM invalidate it. That is deliberate for
the prototype; no live-MMU diagnostic fetch or full-RAM hash is performed on
validation.

## Regressions

`tests/parking.rs` contains nine focused tests:

1. Token issuance succeeds only at the validated boundary with the complete
   mapped path and dependencies; address-only admission is rejected.
   Additional negatives reject changed code, both load exit conditions and a
   changed helper target before issuance.
2. A short explicit interval returns unclipped with the next deadline intact,
   and the token cannot be reused as a value.
3. CPU register, bus memory, MMU mapping, direct device, time and reset changes
   each make the token stale; a token from another machine is rejected too.
4. Software PIL 12 and an unsynchronized TTYA receive source reject admission
   with `ServiceRequired`.
5. A reached ESP timeout reports `service_required` even though its timed
   deadline is replaced by a pending IRQ.
6. Two coincident timer deadlines are clipped to the earlier event and both
   timer IRQs are pending on return.
7. `advance_parked` preserves the complete CPU/MMU state, RAM digest and DRAM
   fingerprint at an event boundary.
8. A reset invalidates an outstanding token.

All nine pass. The review fixtures are promoted as tracked tests:
`tests/parking_review.rs` (10), `tests/parking_correction_edges.rs` (14) and
`tests/parking_iteration_effects.rs` (3), sharing `tests/support/mod.rs`.
All **27/27** pass, including real loop execution, changed code/load/helper,
boot-mode/PTP2/steady-state, PTP2-fill iteration effects, global-context
diagnostics, pending software/serial work, the ESP deadline-to-IRQ transition
and graphics installation/removal/replacement. The existing `tests/timebase.rs`
and promoted `tests/timebase_deadlines.rs` remain green.

## Measured token-validation overhead

The bounded video generations replaced whole-framebuffer Debug hashing. The
independent release harness now measures the same idle fixture with each
installed display (two warmups, twenty samples, nearest-rank p95):

| Installed display | `parking_token()` median | `validate_parking_token()` median | `advance_parked(0)` with validation median |
| --- | ---: | ---: | ---: |
| None | 111.260 µs | 111.355 µs | 111.880 µs |
| CGsix | 114.636 µs | 114.646 µs | 115.132 µs |
| SX | 110.704 µs | 111.595 µs | 111.410 µs |

SX p95s are 111.921 / 114.666 / 112.543 µs; the complete raw sample sets are
in `local/issue5-parking-token-2026-11-17/display-overhead-corrected-final.log`.
The dominant cost is now the deep CPU/MMU/FPU snapshot plus bounded device
fingerprints, not framebuffer size. This is a prototype validation cost, not a
claim about parked interval throughput. Paired fixed-work runs pinned to CPU 2
(5,000,000 iterations, alternating current/`2f903cd`) measured ALU median
**41.79 vs 41.90 ns/step** (-0.26%) and memory **54.02 vs 55.07 ns/step**
(-1.91%) on this host. Raw logs are under
`local/issue5-parking-token-2026-11-17/active-abba/`; the mutation epoch adds
one counter update per bus write attempt, not per CPU read/step.

## Standard validation

Pinned nightly toolchain, `/tmp/heliacal-cargo`:

- `cargo fmt --all --check`: pass
- `cargo clippy --locked --offline --all-targets -- -D warnings`: pass
- `cargo clippy --locked --offline --all-targets --features perf-counters -- -D warnings`: pass
- `cargo test --locked --offline`: **782 executions (781 unique names), 0 failed**
- `cargo test --locked --offline --features perf-counters`: **785 executions (784 unique names), 0 failed**
- `demo`, `trap-demo`, `serial-demo`, `timer-demo`: pass

Logs are under `local/issue5-parking-token-2026-11-17/validation/`. The source
media and firmware were not modified.

## Open review questions

1. Is the TLB-only passive proof (with fallback on misses) sufficient for the
   first prototype, or should a future recognizer maintain a stronger
   core-owned observation of complete iterations?
2. Is the conservative bus mutation epoch the right invalidation boundary, or
   should the next slice use page/PTE-level tracking to avoid unrelated-write
   invalidation?
3. Should `validate_parking_token` remain public for diagnostics, and should
   expiry/clipping be represented by a richer wake reason?
4. Is the ~115 µs validation cost acceptable for the first token prototype, or
   should a compact CPU/device fingerprint replace the deep snapshot before
   frontend waiting is proposed?
