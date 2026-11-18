# Issue #5: admit proved idle loops across CWP 0–7

Status: **accepted as an experimental, default-off checkpoint**, committed
as Helios `3a1cf6f` and Heliacal `e8e2cfc`.
The correctly composed rerun establishes idle CPU savings and a command-
response latency cost. Earlier wrong-frontend measurements remain historical
evidence only. See the final [review](performance-issue5-cwp-admission-review.md)
for the acceptance scope. Issue #5 stays open; this does not approve a default
switch or resolve latency. Current admission accepts only proved CWP 0–7
states; all other PSR bits remain exact.

Independent [implementation review](performance-issue5-cwp-admission-review.md)
passes 19/20 focused normal/profile tests. The source change and now-promoted
isolation fixtures check out. CWP-specific diagnostic commits are recorded,
the corrected unprofiled hashes match both intended builds, and independent
ordinary/profile frontend CWP-0/7 wait/input/BREAK checks pass.

## Problem and scope

The observed Solaris SS-20 idle loop preserves state with any of the eight
implemented register windows when its existing proof conditions hold. The
baseline core/frontend predicates require the whole PSR to equal `0x404000e0`,
so they exclude CWP 7 despite otherwise matching the boundary. The investigation
identified millions of these excluded idle entries.

Accepted evidence: [proof](performance-issue5-cwp-proof-handoff.md) and
[independent review](performance-issue5-cwp-proof-review.md). All-window warmed
fixtures establish full CPU/bus/device snapshot equality without advancing
device time. Real CWP-0/7 pairs establish the actual 14-word path and unchanged
CPU/MMU/RAM under normal stepping; their timed device hash is not a
field-by-field device comparison. Do not weaken device checks on that basis.

Core production baseline is `ff078fa`; frontend baseline is `8e8d82a`.
Retain the uncommitted proof tests/docs as part of the final reviewable work.

## Implementation

1. Define the cheap boundary/PSR eligibility check once in Helios and reuse it
   in core token admission/validation and Heliacal's ordinary and profiling
   candidate paths. Accept implemented CWP values 0–7 while requiring every
   non-CWP PSR bit to equal the existing value. Reject unsupported CWP encodings
   8–31; do not simply clear all five CWP bits without checking the range.
   Keep PC/nPC, SS-20-only, single-CPU and reset/error restrictions unchanged.
   Candidate eligibility is only a cheap hint; it must not replace the mapped
   proof/readiness checks or issue a token by itself.
2. Keep the current-window register reads and exact code/data/helper proof.
   Never rotate or normalize the CPU to CWP 0, copy operands between windows,
   or normalize CPU equality. Unsteady `%o7`/`%l0`, a wrong active window's
   operands, altered code/mapping and pending service still reject.
3. Keep full deep CPU/MMU/FPU/MXCC snapshots in each token. Validation must
   compare the actual full CWP, WIM and every window against issuance state.
   Moving from one eligible CWP to another after issuance must stale the old
   token. No change to digest coverage, epochs, ownership or single-use rules.
4. Preserve the 2 ms host-wait cap, wait-before-commit protocol, measured-time
   clipping, deadline service, serial/BREAK/input/network wakeups, default-off
   setting and SMP exclusion. Keep profiling compiled out by default. No MMU,
   CPU-window, device-time, firmware or guest scheduling change belongs here.

## Required regressions

- Parameterize positive token admission/validation/advance checks over CWP
  0–7 at steady mapped boundaries, with distinguishable inactive windows and
  preserved complete CPU state. Update the old production-CWP-7-rejection
  fixture for the deliberate new behavior; retain the original unsteady
  capture and the remaining negative cases.
- Exercise non-CWP PSR fields and invalid CWP encodings against the shared
  predicate. Demonstrate identical decisions in normal/profile frontend
  paths. Keep cheap rejection ordering: no allocation, digest construction,
  MMU access or per-step diagnostic work in the new candidate check.
- Prove stale-token rejection after changing CWP to another eligible window,
  active or inactive registers, and WIM. Where practical isolate CPU mutations
  from bus/time mutations so a test cannot pass solely because an unrelated
  epoch changed. A new eligible state may issue a new token; it cannot reuse
  the old one.
- At CWP 7, retain meaningful mapped-proof, wrong-window/unsteady-operand,
  permission/fault and pending-work counterexamples, plus deadline clipping,
  coincident IRQ wakeup and pre/post-wait input/BREAK checks. Reuse existing
  fixtures where possible rather than duplicating every test matrix.

## Validation and measurement

Run project-required formatting, Clippy, tests and four demos, including
affected profiling configurations and existing MMU/token regressions. Run
Heliacal tests/lints in normal and `parking-profile` configurations and rebuild
the normal release binary against the reviewed core.

Use disposable SS-20 Solaris clones to confirm real CWP-7 admission and input
responsiveness, with clean sync/halt. For the performance comparison use the
accepted typed-digest baseline and candidate, separate source/target
directories, identical RTC/NVRAM/media settings and alternating order. Retain
both pre-command and later idle windows, command latency, and a bounded active
work/checksum control. Actual host CPU/latency results must use unprofiled
builds; optional diagnostic counts/CWP distributions belong in separate runs.

Report guest virtual-time endpoints alongside host windows. More accepted
parking intentionally removes executed idle iterations, so equal raw outcome
counts are not an acceptance condition. Verify guest results, wakeups and
state-preservation tests instead. Do not discard either idle regime or claim
an end-to-end gain from the synthetic proof alone. Investigate any active-work
or latency regression before recommending acceptance.

Update human docs with the new CWP scope while preserving the experimental,
default-off and SS-20/UP limits. Record evidence and limitations in `clanker/`
and its index. Keep source media and the live catalog unchanged, use SCSI-ID
filenames for disposable disks, retain compact reproducible evidence and
clean temporary builds/media. Stop for review with no commit.


## Implementation result

Helios now exposes one shared predicate: `solaris_idle_psr_matches` and
`solaris_idle_boundary` accept implemented CWP 0–7, require every non-CWP PSR
bit to match `0x404000e0`, and reject CWP encodings 8–31. Core token issuance,
core validation and both ordinary/profiled Heliacal candidate paths call the
same helper. The mapped proof, full CPU/MMU/FPU/MXCC snapshot, current-window
operand checks, readiness/wakeup handling and 2 ms wait cap are unchanged.

Tracked tests now include positive token admission/validation/advance over
CWP 0–7, invalid CWP encodings, non-CWP PSR policy rejection, and stale-token
rejection after changing CWP, an inactive window or WIM. Full warm-up snapshot
invariance remains tested with the device digest included. The real CWP-0/7
before/after pairs remain as evidence; the unreplayed first-entry capture and
its convergence are retained.

## Initial frontend measurement (superseded)

Two alternating runs per arm with disposable media/NVRAM, identical RTC seeds,
parking enabled and profiling disabled:

| Phase | Baseline `ff078fa` | CWP-enabled candidate |
| --- | ---: | ---: |
| Pre-command idle CPU | 23.99% | 24.28% |
| Post-command idle CPU | 38.68% | 38.28% |
| First command median | 27.54 ms | 31.74 ms |
| Post command median | 21.11 ms | 21.15 ms |

This initial sample did not establish feature acceptance. The build-composition
error, corrected measurements and bounded latency disposition are recorded
below and in the review. Aggregate profiles do not establish absolute virtual-
time phase alignment or the cause of the guest's CWP transition.

Validation: core **795 default / 806 parking-profile** successful executions;
frontend **114 / 117**; formatting, Clippy in both configurations, all four
core demos and the existing proof tests pass. No production commit was made.

Evidence: `local/issue5-cwp-admission-2026-11-18/` and
`local/issue5-cwp-proof-2026-11-18/`. Temporary builds/source copies were
removed.


## Completion evidence

The earlier eight-run comparison omitted the frontend patch, as established
by exact binary reconstruction. The table below is the subsequent corrected
comparison; preserve the old runs under their historical labels.

The independent review fixture was promoted as `tests/cwp_admission_review.rs`
(includes the submitted tests and the seven independent checks). Focused
results are **19/19 normal** and **20/20 parking-profile**.

A real diagnostic candidate run recorded per-CWP candidates/commits. CWP 7
was actually admitted and committed: idle window 0 had **1,326 candidates /
1,289 commits** at CWP 7; idle window 1 had **39 / 35**. Command windows also
recorded CWP 4/3/7 successes, with clean sync/halt.

The earlier eight unprofiled runs used the old frontend and are retained only
as **core-only/old-frontend** evidence; their feature conclusions are
retracted. The corrected four alternating unprofiled pairs used core
`ff078fa + core-production.patch` and frontend
`8e8d82a + frontend-production.patch` (candidate hash `60d9c383…`, baseline
hash `bd12d051…`, updated core plus original frontend) with instrumentation
disabled:

| Phase | Baseline | Candidate |
| --- | ---: | ---: |
| Pre-idle CPU median | 24.29% | **7.60%** |
| Post-idle CPU median | 42.88% | **8.40%** |
| First-command median / p95 | 27.83 / 34.53 ms | 33.77 / 36.23 ms |
| Later-command median / p95 | 22.32 / 33.52 ms | 33.36 / 35.56 ms |
| Fixed-work wall / CPU | 5.3656 / 5.335 s | 5.2166 / 5.180 s |

The corrected feature sharply reduces worker CPU in both idle phases, but
adds command latency: first-window median +5.94 ms and later window +11.04 ms
in this sample, with p95 increases around 1.7–2.0 ms. Input latency therefore
remains an explicit default-off tradeoff and open follow-up, not a regression
explained by the old build.
Fixed work is modestly lower. The diagnostic run separately proves real CWP-7
admission/commit (1,326/1,289 and 39/35 candidates/commits in idle windows).

Full core validation after promotion: **814 default / 826 parking-profile**
successful executions, formatting and Clippy pass; frontend is now 115/119
and all demos pass. Production admission is changed only for CWP 0–7 with
exact non-CWP PSR bits; full snapshots and stale-token checks remain.

The checkpoint is accepted with parking still disabled by default. The real
CWP transition trigger and added command-response latency are unresolved;
neither permits weakening proof or wakeup checks. Core human docs are updated;
frontend CWP tests and the scope/latency user-doc note are now promoted and
validated. The default-off checkpoint is committed; no new broad
benchmark is requested. Continue the separate bounded
[latency attribution handoff](performance-issue5-parking-latency-handoff.md).
No commit was made by review.
