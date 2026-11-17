# Issue #5 parking-token review

Reviewed 2026-11-17 against the uncommitted token slice above `2f903cd`, with
two independent correction rechecks. **Not accepted yet: two MMU-proof
details remain.** All previously reported fixes pass their 24 fixtures.
A [tested review patch](performance-issue5-idle-parking-token-fix.patch)
addresses the remaining details; it passes all 27 fixtures in an isolated
copy. Production code remains untouched by the reviewer, and no commit was
made. Frontend sleeping, host pacing and SMP parking remain outside scope.
Issue #5 stays open.

## Latest correction recheck

The previous ten original and fourteen edge fixtures now pass **24/24**.
The latest standard validation also passes independently: **755 normal
executions / 754 unique tests**, **758 perf-counter executions / 757 unique
tests**, formatting, both all-target Clippy configurations with `-D warnings`,
and all four demos. Graphics/MXCC configuration epochs, steady `%o7`/`%l0`,
boot-mode rejection and rejection of existing relevant PTP2 state are fixed.

Three new fixtures fail on the submitted candidate and pass with the isolated
patch. They cover two remaining discrepancies in `parking_peek()`.

### 1. An earlier access can create the override used by a later one (P1)

The proof rejects a relevant PTP2 entry that already exists, but normal
translation calls `fill_ptp2()` on both fast and regular TLB hits. An ordinary
low-RAM L1 entry with a `pte_address` can therefore create new PTP2 state
during this same fourteen-instruction iteration.

Both reproductions start with the complete warmed idle fixture and **no
relevant PTP2 override**. All current token checks pass:

- Instruction case: retain the architectural L1 TLB, evict only the helper's
  fast-cache key, and put a PTP at the L1 bookkeeping source (`0x2004`). The
  first fetch fills I-PTP2. The later helper fetch now executes
  `add %g1, 1, %g1` instead of RETL. Token admission still succeeds.
- Data case: evict only the second load's fast-cache key, and put a PTP at
  `0x2024`. The first load fills D-PTP2. The second load reads **1**, although
  the proof read **0** from the base mapping, so normal execution leaves the
  idle path. Token admission still succeeds.

**Correction in the patch:** for a reference-MMU L1 entry with a backing PTE
address, derive the exact source word that `fill_ptp2()` will read. Inspect it
through the existing read-only ordinary-RAM peek and reject a PTP descriptor
or a non-RAM source. This is one bounded bookkeeping read, with no live MMU
walk or state update. Preserve the existing checks for already-cached PTP2
overrides and unsafe PROM subregions. Do not change normal translation or
`fill_ptp2()` behavior.

### 2. Global mappings can have a different diagnostic context (P1)

The diagnostic-readiness check builds its expected values with `self.context`,
while normal `record_diag_view()` uses **`entry.context`**. A supervisor-global
entry can match when those differ.

The fixture retains a diagnostic view with its context field still zero,
installs a global mapping by fetching instructions in context 7, then returns
to context 0. The token is admitted because its check treats the current zero
context as requiring no fill. Normal execution fills guest-visible diagnostic
offset `0x9100` with **7**. The iteration is not yet steady.

**Correction in the patch:** use `entry.context` in the readiness tuple, exactly
as `record_diag_view()` does. Fall back to normal execution while the field
remains fillable; do not fill it inside the observer.

### Tested patch and next action

Apply `clanker/performance-issue5-idle-parking-token-fix.patch` to the current
candidate. `git apply --check` succeeds against the reviewed source. The
isolated patched copy passes library Clippy with `-D warnings` and **27/27
review fixtures**. Full standard validation above applies to the submitted
candidate, not the isolated patch: rerun standard checks after applying it.

Promote the review fixtures into tracked tests before committing. The current
`tests/parking.rs` still has nine tests; the 24 edge/original fixtures and the
three new iteration fixtures remain in ignored `local/`. Consolidating shared
setup or overlapping cases is fine, but preserve every demonstrated failure
and the positive mapped-loop and observational-query controls.

Current evidence is in
`local/review-issue5-parking-token-2026-11-17/recheck2/`:

- `tests/parking_review.rs`, `tests/correction_edges.rs`,
  `tests/iteration_effects.rs`, `tests/support/mod.rs`;
- `fixtures.log`: candidate **24 pass / 3 fail**;
- `patched-validation.log`: isolated patch Clippy and **27 pass / 0 fail**;
- `tested-fix.patch`, source hashes, full validation log/summary and cleanup
  record.

The review used no guest media. Temporary builds and the redundant patched
source/test copies were removed; the patch and compact reproducible evidence
remain. The display-cost benchmark was not repeated in this recheck; the
bounded graphics validation is unchanged. Keep the frontend inactive and
return the corrected core checkpoint for final verification.

## First correction recheck (historical)

The following records the preceding candidate; its three findings were fixed
by the latest submission. Use the section above for current status.

Independent standard validation passes: **755 normal executions / 754 unique
tests**, **758 perf-counter executions / 757 unique tests**, formatting, both
all-target Clippy configurations with `-D warnings`, and all four demos.

The unchanged original review suite passes **10/10**. A further fourteen-case
suite passes **4/14**, with ten failures grouped into the three findings below.
The passing additional controls cover removal of execute/data-read permission
without query side effects, short/exact/past timer deadlines, and ordinary
graphics bus writes including nonvisual SX registers. The readiness and
disappearing-ESP-deadline corrections work in the retained fixtures.

Evidence is under `local/review-issue5-parking-token-2026-11-17/recheck/`:

- `tests/parking_review.rs`: unchanged original suite.
- `tests/correction_edges.rs` and `tests/support/mod.rs`: new fixtures and the
  fully mapped, warmed fourteen-instruction loop.
- `fixtures.log`: both suites run with `--no-fail-fast`.
- `validation.log.gz`, `validation-summary.json`, `reviewed-sources.json`:
  standard validation and the exact candidate source hashes.
- `display-overhead.log`: independently repeated display measurements and
  raw samples; `src/main.rs` is the release harness.

The review did not boot or alter any guest media. Its build directories were
removed after testing; compact fixtures and logs remain.

### A. The passive MMU lookup can disagree with actual execution (P1)

`src/mmu.rs:1887`, `parking_peek`, picks the smallest matching architectural
TLB entry. That is only part of the normal `translate()` decision. Three
fixtures preserve the expected code and dependencies in that base mapping,
but actual execution reads something else:

1. Set the SS-20 PCR boot-mode bit while retaining EN and the warmed RAM TLB.
   The token is issued. Normal stepping fetches PROM, sees `0xffffffff`, and
   stops with `UnsupportedInstruction` instead of executing the idle load.
2. Evict the instruction fast-cache key while retaining the L1 TLB, then set
   I-TLB PTP2 through ASI 5 to override the code subregion. The token still
   uses the base TLB. Normal stepping follows `reference_subregion()` and
   executes `add %g1, 1, %g1` at the same PC.
3. Apply the corresponding D-TLB PTP2 override through ASI 6 after a data
   fast-cache collision. The proof reads the old `1`, but the real first load
   reads `0`, which exits the idle path.

**Required correction:** reject boot mode and reject any translation state
whose actual fetch/load behavior is not covered by the passive proof. Account
for instruction/data fast-cache selection, reference PTP2 overrides and the
normal path's fault/side-effect conditions. Conservatively refusing the
special cases is acceptable for this narrow prototype. If sharing a lookup
helper, keep the interpreter's existing behavior authoritative and ensure the
observer has exactly its selection semantics without performing a live walk.
Do not change MMU replacement or reference-subregion behavior to make the
proof pass. Extend the proof tests with all three fixtures, not just the PCR
case. The two permission-removal controls already pass.

### B. Recognizing a path does not establish a steady boundary (P1)

`src/machine.rs:1304`, `idle_boundary_proof`, checks the loop words, `%o0`,
`%i0` and memory dependencies, but not all state that the loop writes.
Admission still succeeds in these three cases:

- `%o7` starts at `0xdeadbeef`; one normal iteration changes it to
  `0xf006fa58` through JMPL.
- `%l0` starts at `0xdeadbeef`; one normal iteration changes it to
  `0xf0071708` through SETHI and LD.
- Clear the D-TLB diagnostic tag at ASI 6 offset `0x9000`, retaining the
  functional L1 mapping. One normal iteration refills the guest-visible value
  with `0xf0240000`. Parking preserves zero instead.

These are pre-issuance states, so checking the token's snapshot for subsequent
mutation does not fix them. In particular, a first arrival at the PC must not
be confused with a completed stable iteration. An interrupt after parking
could observe register or MMU state different from normal execution.

**Required correction:** prove that a full iteration leaves all relevant
architectural state unchanged at the chosen boundary. At minimum, check the
known final `%o7` and `%l0` values and ensure the MMU will not perform the
missing guest-visible updates. A core-owned observation of a complete stable
iteration, with dependency invalidation, is another acceptable mechanism.
Let normal execution establish the state when it is not yet steady; do not
normalize registers or silently fill diagnostic state inside token issuance.
This requirement concerns guest-visible MMU state, not merely host cache-hit
statistics. Promote the three fixtures and retain the fully warmed positive
control and read-only query assertions.

### C. Graphics installation/removal can leave tokens valid (P2)

`src/system_bus.rs:1194` uses per-card generations, with zero for an absent
card. `install_sx`, `remove_sx` and `install_cgsix` do not advance the bus
mutation epoch. Four fixtures demonstrate surviving tokens after:

- Installing a fresh SX card: both absent and new-card generation are zero.
- Installing a fresh CGsix card: both generation and scanout key are zero.
- Removing a fresh SX card from a machine with an outstanding token.
- Replacing SX with another card at the same generation but different VRAM.

Ordinary graphics MMIO writes, including nonvisual registers, correctly stale
the token through the bus epoch. The gap is structural replacement. Presence
bits alone do not solve the equal-generation replacement case.

**Required correction:** advance a bus-owned lifetime/configuration epoch on
successful device installation, removal and replacement, and include it in
validation. Audit the corresponding public mutation methods so independent
per-device generations cannot recreate a valid old token. Keep graphics
validation bounded; full-framebuffer hashing must not return.

### Confirmed performance and next return

The independent release harness, thin LTO and one codegen unit, reports:

| Display | Issue median | Validate median | Advance(0), including validation median |
| --- | ---: | ---: | ---: |
| None | 112.397 µs | 112.507 µs | 112.908 µs |
| CGsix | 114.041 µs | 114.431 µs | 114.541 µs |
| SX | 114.526 µs | 114.667 µs | 114.926 µs |

Two warmups precede twenty samples; all raw values are retained. SX p95s are
119.706 / 120.888 / 119.635 µs. CGsix validation has a 286.620 µs p95 in this
small sample, so the measurements do not establish a strict latency bound.
The earlier 119 ms SX formatting/hash cost is gone. No standard validation
was running concurrently with this measurement.

The retained paired fixed-work log arithmetic matches the reported ALU
41.79 vs 41.90 ns/step and memory 54.02 vs 55.07 ns/step. This review did not
repeat those paired runs. No frontend sleeping or reduced-CPU-use claim is
being accepted from these API benchmarks.

Run and promote the additional fixtures using the pinned toolchain/cache:

```sh
cargo test --locked --offline \
  --manifest-path local/review-issue5-parking-token-2026-11-17/recheck/Cargo.toml \
  --test parking_review --test correction_edges --no-fail-fast \
  -- --test-threads=1
```

Return these core-only corrections for review with all positive controls
retained, the new negative cases promoted and standard validation. Preserve
the working readiness/deadline changes. No production code was modified by
this recheck and no commit was made.

## Original review record

The remainder describes the first candidate, before the corrections above.
Its counts and timings are historical; use the recheck for current status.

The normal validation report checks out independently: **752 executions / 751
unique tests**, **755 executions / 754 unique tests with perf-counters**,
formatting, both all-target Clippy configurations with `-D warnings`, and all
four demos pass. The additional independent suite is **2 passing / 8 failing**
across ten tests; the failures are described below.

Evidence, runnable fixtures and source hashes are retained under
`local/review-issue5-parking-token-2026-11-17/`. The review used no guest disk,
NVRAM, firmware or live catalog. Its temporary build directories were removed.

## 1. Admission does not prove that this is an idle boundary (P1)

`src/machine.rs:1304` checks the profile, CPU count, reset/error status and
PC/nPC/PSR. It does not check the instruction stream, translations, effective
load addresses, loaded values or indirect helper. Comparing the token's CPU
and mutation epoch later only proves that the state has not changed **since
issuance**. It cannot establish that the initial state was safe to skip.

The submitted `tests/parking.rs` helper makes this particularly clear: it
sets the Solaris PC on a machine without any executable mapping there. A
token is issued anyway. That test does not demonstrate guest recognition.

The independent fixture reconstructs the captured fourteen-instruction path,
with a real reference-MMU mapping and all three observed load dependencies:

| Dependency | Expected value at the captured boundary |
| --- | --- |
| `[o0 + 0xec]` at `0xf025f8ec` | `1` |
| `[i0 + 0x58]` at `0xf026dea0` | `0` |
| `[l0 + 0x1e8]` after the loop's SETHI, at `0xf025f9e8` | `0xf0071708`, the captured RETL/NOP helper |

The control executes the complete path, returns to the same boundary and
compares equal across the diagnostic snapshot, including CPU/MMU and RAM.
Issuance/validation are also observational in that control. Nevertheless the
candidate still issues tokens after each of these pre-issuance changes:

- No executable loop at all, with just the matching PC/nPC/PSR.
- Replacing the first instruction with `add %g1, 1, %g1`.
- Changing the first load to zero, which leaves the idle path.
- Changing the second load to one, which signals runnable work.
- Redirecting the indirect helper to active code.

**Correction:** establish an executable, conditional proof before admission.
Validate the complete known path, its effective register/data dependencies,
instruction/data access permissions and ordinary-RAM mappings, or require an
equivalent core-owned proof produced by passive observation of a complete
iteration. Track that proof's dependencies and invalidate it on changes.
Do not add diagnostic fetches through the live MMU: those can modify TLBs,
R/M bits, faults and diagnostic state. Unknown/faulting mappings and MMIO
dependencies must fall back to normal execution. If proof construction is a
separate slice, do not let this prototype issue usable execution-skipping
tokens until that proof exists.

Replace the address-only positive fixture with real loop execution. Promote
the changed-code, both exit conditions and helper-target negative cases.
Add mapping/execute-permission rejection and an observational-query check
for the chosen proof mechanism. Conservative global RAM invalidation is fine
for this stage; optimizing dependency granularity is not required.

## 2. Pending work can already require the CPU to run (P1)

`parking_token()` has no readiness check, and `advance_parked()` assumes
`timebase().next_ns` is sufficient. The timebase is an event-clock query; it
does not include every already-pending interrupt or pre-step service action.

Two reproductions use the fully mapped idle fixture:

- Enable software PIL 12 in CPU 0's interrupt bank. The controller reports
  `1 << 12`, while the captured PSR has ET enabled and PIL zero. The candidate
  advances **10,000 ns** instead of giving the CPU the current boundary.
- Configure TTYA receive interrupts, inject a byte before requesting the
  token, and leave the controller at its ordinary pre-service state. The SCC
  already asserts its IRQ; normal pre-step service would synchronize it. The
  candidate again advances **10,000 ns**.

**Correction:** use a read-only parking-eligibility check that accounts for
deliverable CPU interrupts and work the normal pre-step path would service,
including unsynchronized device IRQs. A pending token request must either be
rejected or yield zero advancement with a required-service result. Conservatively
rejecting extra states is safe. Recheck the relevant state at consumption.
Do not perform interrupt synchronization as a side effect of an eligibility
query: preserve the accepted held-SCSI-reset interrupt boundary and use normal
service to resolve it. Keep the existing DMA readiness and pending-reset
checks conservative too.

Mutation after issuance already invalidates the tested dependencies. The
missing case is input/work that is present **before issuance**, not a missing
host wakeup protocol. Host waiting remains a later slice.

## 3. A reached event can disappear from the next-deadline query (P1)

`src/machine.rs:1365` derives `service_required` solely from the timebase
**after** advancing. Some events transition from timed work to a pending IRQ;
their next timed deadline then disappears.

The independent ESP fixture selects an absent SCSI target with timeout 1,
clock factor 2 and DMA2 interrupt enable. The first deadline is 409,600 ns.
Advancing by exactly that interval returns:

```text
advanced_ns: 409600
clipped: false
next_ns: Some(1000000000)
service_required: false
```

At this point ESP IRQ is asserted and the controller reports PIL 4. The new
deadline is only the RTC tick. Both documented caller flags say no service
is needed, even though the timeout has just made the CPU runnable.

**Correction:** remember whether the advance reached the **original** first
deadline, including equality with the requested interval and an already-due
event. Report required service on that boundary even if the timed event
vanishes after advancement. Combine this with the readiness rule above;
do not rely on `clipped` or sticky timer pending bits. Test intervals shorter
than, equal to and longer than the event, repeated token requests before
service, and coincident events. Keep device clocks and CPU/DMA service
separate: this fix must not execute a handler inside `advance_parked()`.

## 4. The overhead measurement excludes installed graphics (P2)

`src/system_bus.rs:1156` fingerprints whole CGsix/SX objects using
`diagnostic::digest_debug`. That routine allocates a formatted Debug string
and hashes it. Derived Debug includes the complete framebuffer: 8 MiB of
bytes for SX and 1 MiB of words for CGsix, plus device registers/ROM. This is
appropriate for infrequent diagnostic snapshots, not repeated parking checks.

The submitted ~105 µs measurement uses `InstalledVideo::None`. Tokens do not
restrict video configuration; a serial-console machine can still have SX
installed. The independent release harness uses thin LTO, one codegen unit,
the mapped idle fixture and empty framebuffers. Two warmups precede twenty
samples per operation; p95 uses nearest rank. No full-core validation was
running concurrently with this measurement.

| Installed display | Issue median | Validate median | Advance(0), including validation median |
| --- | ---: | ---: | ---: |
| None | 111.871 µs | 111.926 µs | 112.853 µs |
| CGsix | 3.642 ms | 3.602 ms | 3.602 ms |
| SX | 119.073 ms | 118.190 ms | 118.790 ms |

SX p95 values are 122.661 / 120.265 / 121.891 ms respectively. A normal
issue-plus-consume cycle already costs about **238 ms** on this host, before
any wait. These are local measurements, not a latency guarantee or a frontend
benchmark. The prototype is unused by production stepping, so this is not a
claim that the current frontend has acquired that delay.

**Correction:** replace per-token full-framebuffer Debug hashing with bounded
state/generation validation, preserving invalidation for all public mutation
paths. Alternatively, explicitly reject configurations this prototype cannot
handle within its intended cost, and document the restricted scope. Do not
silently omit graphics state from validation. Measure every accepted display
configuration and retain raw samples. Refresh paired fixed-work results after
the corrections if mutation tracking changes the active write path; the rough
ranges in the implementation handoff do not establish zero overhead.

## Fixtures and return gate

The external suite is
`local/review-issue5-parking-token-2026-11-17/tests/parking_review.rs`.
Run it with the repository's pinned toolchain/cache:

```sh
cargo test --locked --offline \
  --manifest-path local/review-issue5-parking-token-2026-11-17/Cargo.toml \
  --test parking_review -- --test-threads=1
```

Its two passing controls prove actual idle-loop execution, unchanged query
state and stale tokens after post-issuance dependency writes. Five failures
cover admission; two cover already-pending interrupts; one covers the
disappearing ESP deadline. `fixtures.log` contains the results. The local
`src/main.rs`, `display-overhead.log` and raw samples reproduce the graphics
cost. `validation.log.gz`, `validation-summary.json` and
`reviewed-sources.json` record the independently checked candidate.

Promote/adapt these fixtures to the corrected proof mechanism without losing
the positive real-loop control or weakening the rejection cases. Return the
core-only correction for review with full standard validation and explicit
remaining limits. No frontend sleeps, host catch-up policy, SMP recognizer,
MMU replacement change or JIT work is authorized by this slice. Do not claim
reduced idle CPU usage from these core API measurements.
