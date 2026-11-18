# Issue #5: CWP proof review

Status: proof checkpoint accepted after the real paired-iteration audit below.
The initial findings and intermediate correction review are retained as
history. No production source or submitted test was edited by this review.
Production changes remain a separate bounded slice.

Reviewed core: `ff078fa`; submitted `tests/parking_windows.rs` SHA-256:
`2e2446e45f93a3afa8653c4f2ac9394f1f278bdfa56b964c8b96009a5c10bd64`.

## Findings and tested corrections

### 1. Compare the entire stable snapshot, including devices

`seed_logical_operands()` executes WRPSR at physical `0x2000`. The checkpoint
therefore has IoControl `memory_address=0x2000`; the first loop ends at
`0x06fa64`. The submitted test skips `devices_digest` and consequently does
not establish complete loop-state invariance.

This field is readable at IoControl `+0x1054`, and both memory-request capture
fields are included by `IoControl::fingerprint` in `parking_device_digest`.
Calling it neither guest-visible nor a parking input is incorrect. Keep the
existing model and fingerprint unchanged.

Tested correction: after seeding, execute one explicitly documented ordinary
warmup iteration. Take a fresh `Machine::diagnostic_snapshot()`, replay the
14 instructions, and require `before.differences(&after).is_empty()` with no
normalization or omitted device digest. This passes for CWPs 0 through 7,
including a rotated WIM pattern whose CWP-7 case is the observed WIM=1.
Retain a separate test showing the initial latch change; do not describe the
pre-warmup state as invariant.

### 2. The captured real state has not been replayed and is not steady

`captured-cwp7.txt` has CWP=7, WIM=1 and the expected PC/nPC, but its logical
registers are:

| Register | Captured | Existing proof requires |
| --- | --- | --- |
| `%o0` | `0xf025f800` | `0xf025f800` |
| `%i0` | `0xf026de48` | `0xf026de48` |
| `%o7` | `0xf006fa44` | `0xf006fa58` |
| `%l0` | `0xf025f800` | `0xf0071708` |

The capture hook writes the first matching snapshot and does not replay it.
Its bus state contains fingerprints rather than memory contents, so this text
alone is not a restorable real-machine checkpoint. The submitted tests instead
seed steady operands in a synthetic machine. Reproducing the captured logical
values in that synthetic mapping confirms that one loop changes `%o7`/`%l0`;
the second loop is invariant. This is a synthetic explanation, not a replay
of all captured guest mappings and dependencies.

Preserve the raw first-entry capture. In a disposable guest, observe normal
execution to the next natural loop boundary, then record one complete steady
iteration for CWP 7 and a CWP-0 control. Retain returned raw instruction words,
PC/nPC, full CPU/MMU snapshots, resolved RAM dependencies and device state.
Do not rewrite registers or step the live CPU outside normal machine-time
accounting to create a favorable checkpoint. Device time will advance under
normal Machine stepping: report it separately and check that register/MMU/RAM
and memory-request state are steady rather than silently discarding changes.
The synthetic CPU-only replay remains the zero-device-time equality lemma.

### 3. Strengthen the counterexamples and their claims

`non_idle_psr_is_a_counterexample` currently only writes PIL and checks that
PSR differs. It calls neither replay nor admission. The reviewed PIL-only
case at CWP 0 preserves the loop state but is rejected by production policy.
Assert the rejection separately and label it a conservative policy exclusion,
not evidence that the loop cannot be invariant with a different PIL.

`wrong_operand_window_is_a_counterexample` corrupts `%o0` in the current
window; it does not place correct operands only in a different physical
window. Retain that bad-current-operand check with an accurate name, and add
the intended wrong-bank fixture. Likewise make `%o7` and `%l0` convergence
explicit: the existing `%l0=0` case is unsteady initial state even though the
same instruction path can still execute. Preserve the current rejection of
CWP 7 and all existing MMU/fault/readiness/token tests.

## Independent validation and handoff

Retained fixture:
`local/review-issue5-cwp-proof-2026-11-18/cwp_review.rs`. It includes the submitted
six tests plus four independent checks:

- guest-readable initial memory-address change;
- full snapshot invariance after ordinary warmup for every window;
- the captured logical-register mismatch and its natural convergence;
- PIL-only policy rejection separated from loop invariance.

All ten tests pass in both default and `parking-profile` builds. Commands:

```sh
cargo test --locked --offline --test cwp_review
cargo test --locked --offline --test cwp_review --features parking-profile
```

An isolated source/target tree was used. Build identity and logs are retained
beside the fixture; the initial empty-cache failure is recorded separately.
This review did not rerun the full project validation or a guest boot. Temporary
source/build output was cleaned; source media and the live catalog were untouched.

Promote the tested corrections, complete the steady real-boundary observation,
and return uncommitted with the project-required validation. The CWP-transition
cause may remain a stated limit; it must not be replaced by a claim of real
replay from a static seeded fixture. Keep the production predicate, full CPU
token snapshot, all mapped proof/readiness checks and the 2 ms cap unchanged.
No additional broad performance study is required for these corrections.

## Corrected proof review

The updated `tests/parking_windows.rs` has SHA-256
`a1a319054cd3dc98f60714a5949579f59fc339b9289f649e0b97db91846187ac`.
The nine submitted tests plus the four independent review fixtures pass in
both default and `parking-profile` builds (13/13 each). Explicit warmup now
precedes full snapshot equality, the pre-warmup device change remains a
separate test, and the policy and wrong-window counterexamples are corrected.
The retained full-suite logs contain 792/803 successful test executions and
zero failures, with clean formatting/Clippy logs. Full-suite results are
retained evidence; this review reran the focused 13-test sets independently.

The seven new real snapshot files cover CWP 0–5 and 7. There is no CWP-6
capture in this run, as the capture build note already records. CWP-0 and
CWP-7 logical operands, PC/nPC and WIM check out. Synthetic coverage still
includes all eight CWPs.

One gate remains: `diagnostic-steady-capture.patch` stores only the previous
`(%o7, %l0)` for each CWP, then emits a single full snapshot when those two
values match again. It retains neither the earlier full state nor the actual
instruction path. The samples establish that the expected operands occur in
the real guest, not that a whole real iteration preserves CPU/MMU/RAM and
observation state. This is the same paired-iteration requirement from the
original handoff, not a new broad investigation.

### Finish with one passive paired capture per CWP 0 and 7

Use one disposable guest run and retain two successful pairs. Do not repeat
all timing comparisons or seek real captures for every other CWP.

1. Immediately **before an ordinary worker CPU step**, arm a bounded observer
   at the expected PC/nPC, non-CWP PSR and steady logical operands for CWP 0
   or 7. Save the full `MachineSnapshot` and read-only IoControl/device state.
   Arming immediately before execution avoids including a preceding frontend
   wait in the CPU-iteration interval. Do not execute an extra CPU step from
   the observer or change any guest registers, timer, input or parking policy.
2. After each ordinary step, feed its returned `Step` into the observer.
   Require all 14 raw PC/instruction pairs from the tracked fixture, in order.
   Retain PC/nPC, outcome kind and CWP for each. A trap, interrupt, reset,
   unexpected instruction or changed path invalidates that attempt; record
   the reason and allow a later naturally occurring attempt.
3. Immediately after the fourteenth ordinary step, **before any frontend
   parking attempt**, save the second full snapshot. Assert equal complete
   CPU/MMU state, all register windows, WIM/PSR, RAM/DRAM and IoControl
   observation state. Retain the actual comparisons and both snapshots, not
   just matching `%o7`/`%l0` or a success label.
4. Normal `Machine::step()` advances virtual/device time; record its delta,
   clock configuration, and scheduler/device differences separately. Keep
   enough read-only device state to identify timing-derived differences.
   Do not require artificial zero elapsed time and do not erase unrelated
   changes from the result. The CPU-only synthetic test remains the separate
   zero-device-time full-equality lemma.
5. Save the feature-gated observer patch, exact build recipe, two before/after
   pairs, 14-word traces, comparison report and aborted-attempt counts. Stop
   after both pairs are accepted. Clean the disposable build/media and return
   uncommitted with production admission unchanged.

The real transition's root cause may remain an explicit limit. The immediate
goal is the invariant needed for the proposed CWP change. A failed real pair
must be reported with its exact differences rather than normalized to agree.

Corrected review artifacts and focused logs are under
`local/review-issue5-cwp-proof-corrected-2026-11-18/`. Temporary review source
and target directories were removed; no source media or live catalog changed.

## Final pair acceptance

The retained `loop-pairs/cwp0.steps` and `cwp7.steps` each contain complete
before/after snapshots and 14 returned execution records. Independent audit
confirms the expected words, PC/nPC chain and delay slots, no traps or
interrupts, unchanged CWP, and a return to `0xf006fa24/0xf006fa28`. CPU text
(including all windows, MMU and FPU) is identical, and memory fingerprints
are unchanged. The only raw snapshot differences are `logical_ns`, deferred
`device_ns`, and `devices_digest`.

Logical time advances by 1,400 ns in each pair. Both apply 2,000 ns to devices:
`1100 + 1400 - 500` for CWP 0 and `600 + 1400 - 0` for CWP 7. The old summary
label calling 1,400 ns applied device time is corrected in the handoff; the
original evidence is preserved. A changed device hash does not independently
attribute its constituent field changes. Do not claim timed device-field
invariance from these files. Device-inclusive equality is demonstrated by
the separate zero-device-time warmed fixtures, not by normal timed stepping.

Taken together, the controlled all-window replay and real CPU/MMU/RAM pairs
support proceeding with the narrow CWP eligibility change, while keeping the
existing device proof/readiness, fingerprint, time and wakeup contracts intact.
This acceptance does not establish the cause of the real CWP transition,
production acceptance of CWP 7, or an end-to-end performance gain.

Audit script, hashes, exact textual differences and time arithmetic:
`local/review-issue5-cwp-proof-corrected-2026-11-18/pair_audit.{py,json}`.
No tests changed since the independently passing 13/13 focused runs in both
configurations. This final review created only small audit/document artifacts;
no guest was launched, and no additional temporary build or media was created.

Next work order: [CWP admission implementation](performance-issue5-cwp-admission-handoff.md).
