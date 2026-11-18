# Issue #5: CWP admission implementation review

Status: **accepted as an experimental, default-off checkpoint** after the
correctly composed unprofiled rerun. Idle CPU savings are established, with
an explicit command-response latency cost. Default-on/general latency
acceptance remains open under issue #5. Earlier build-mismatch findings below
are retained as history; the final section records the corrected evidence.
This review changed documentation and local fixtures only, with no production
code edits, commits, pushes or merges.

## Source and independent checks

Reviewed the uncommitted change against Helios `ff078fa` and Heliacal
`8e8d82a`. The shared predicate accepts only CWP 0–7, checks every other PSR
bit exactly, and retains exact PC/nPC. Core issuance/validation, ordinary and
profiled frontend preparation, and the per-instruction frontend candidate
check use it. Full CPU/MMU/FPU/MXCC equality, mapped proof, readiness, deep
snapshot ownership, deadline handling and host-wait protocol are unchanged.

Independent fixture source and logs are retained under
`local/review-issue5-cwp-admission-2026-11-18/`. An isolated copy of the
candidate passes **19/19 normal** and **20/20 parking-profile** tests: the
12 submitted tests plus seven independent tests and one feature-only test.
Coverage includes:

- All 32 CWP encodings, each non-CWP bit flipped independently, and wrong
  PC/nPC. Eight warmed window states admit, validate and advance while
  preserving full CPU state and RAM.
- A CWP-only change between two fully eligible windows invalidates an old
  token. Bus state/time/epochs are untouched. Profiling confirms the failure
  reaches the exact CPU snapshot comparison; switching back restores the
  complete original CPU.
- A truly inactive physical local register changes without changing any of
  the 32 visible registers or live bus state; the old token still becomes
  stale. WIM-only changes are checked separately.
- CWP-7 wrong/current-window operands, unsteady o7/l0, changed helper/code/data,
  execute permission removal and boot override reject admission. Pending
  interrupts and coincident timer deadlines retain their service boundary.
- A CWP-7 PIL mismatch rejects before mapped proof or digest construction.

Two submitted fixtures need strengthening before promotion is called complete:
`cwp_change_stales_an_existing_token` invokes `set_cwp`, which writes live
RAM and changes g1/MMU/bus observation state. It does not isolate CWP equality.
`inactive_windows_and_wim_changes_stale_tokens` writes register 24, which is
the **currently visible i0** and a mapped-proof dependency, even though its
storage aliases the next physical window's outputs. Use the independent
fixtures rather than claiming these establish inactive-window isolation.
The wrong-operand/code counterexamples should assert token rejection as well
as non-invariant replay. Existing proof fixtures can remain.

Independent formatting checks pass in both repositories. Submitted logs have
795/806 successful core executions and 114/117 frontend tests, no failures;
their Clippy and demo results are clean. This review reran the focused tests
and formatting, not the complete guest boot/full test matrix.

## Measurement audit

Recomputed CPU utilization from worker `/proc` tick deltas, and command
latencies from the retained per-trial timestamps. The four unprofiled boots
ran in baseline/candidate/candidate/baseline order with stable per-arm binary
hashes, matching media/NVRAM/ROM input hashes and the same reported guest date.
All six submitted runs, including the two profiling runs, retain the Solaris
halt message, completed filesystem sync, OBP prompt and host exit 0. Disposable
disk clones are gone. No guest failure was found in these transcripts.

Medians below are medians of the two per-run phase statistics, not a claim of
many independent boot replications:

| Measurement | Baseline | Candidate |
| --- | ---: | ---: |
| Pre-command idle worker CPU | 23.988% | 24.285% |
| Post-command idle worker CPU | 38.682% | 38.284% |
| First command-window median | 27.536 ms | 31.742 ms |
| First command-window p95 | 37.846 ms | 40.522 ms |
| Later command-window median | 21.113 ms | 21.154 ms |
| Later command-window p95 | 34.103 ms | 28.453 ms |
| Fixed 100k-iteration guest work, wall | 5.157761 s | 5.185549 s |
| Fixed work, worker CPU | 5.120 s | 5.150 s |

The first-window median is **+4.205 ms / +15.27%**; both candidate runs are
higher than both baseline runs. First-window p95 rises 7.07%. This warrants a
bounded repeat; two boots per arm do not establish the cause or a general
regression. Later latency is essentially unchanged. Fixed-work wall/CPU
changes are +0.54%/+0.59%, with identical `4999950000` guest results. No broad
CPU improvement is established, as the implementation report correctly says.

The diagnostic runs cannot yet explain that outcome. Their profiles aggregate
all CWPs and omit absolute guest-time phase endpoints. They contain no record
of a successful **real CWP-7 token and commit** in the new frontend. Synthetic
admission and the earlier parking-disabled loop captures do not substitute
for that observation. In the later diagnostic phase, baseline/candidate have
28,594,970/35,203,612 outcomes, 1,368/1,269 candidates and 1,341/1,244 commits;
the respective aggregate admission rates are still about 98%. These counts
alone do not say whether CWP-7 loops were admitted, absent, or excluded before
the aggregate counter.

Binary hashes are retained, but the current evidence directory lacks the
exact source-overlay/lockfile/toolchain/build recipe for the unprofiled arms.
The two profile build logs identify separate build directories, but do not
prove which uncommitted frontend overlay went into either measured candidate.
This is a provenance gap, not evidence that the wrong binary was used. Rule
it out before attributing the unchanged CPU use to guest behavior.

The original CWP transition cause remains explicitly out of scope. No new
proof relaxation, guest-window manipulation or timing change is justified.

## Bounded completion assignment

1. Promote/adapt the independent fixtures, fixing the two misleading tests.
   Parameterize the existing frontend ordinary/profile preparation and commit
   tests at least over CWP 0 and 7, including queued input/BREAK rejection.
   Update human parking docs with CWP 0–7 eligibility, exact other PSR bits,
   full snapshot stale-token semantics and the existing default-off SS-20/UP
   scope. Do not restart the accepted loop proof.
2. Rebuild from explicitly recorded source overlays in separate directories.
   Retain commits, the two production patches, lockfile hashes, toolchain,
   feature flags, commands and output binary hashes. Verify the candidate
   includes **both** the core predicate and all frontend call sites. Keep
   instrumentation default-off and absent from the unprofiled measurements.
3. In a bounded diagnostic guest run, record exact loop-boundary entries,
   candidate decisions, token successes/rejections and commits **by CWP**,
   together with absolute guest virtual phase endpoints. Retain a successful
   real CWP-7 issue/commit with preserved CPU state, and serial input followed
   by clean halt. If the relevant CWP-7 interval is not reached, extend that
   diagnostic window explicitly; do not label an unexercised run acceptance.
   If candidates fail, report the first existing guard that rejects them;
   do not weaken it. Reuse the previous bounded loop-state instrumentation,
   updating its old exact-PSR counter to distinguish old and new predicates.
4. Run four new alternating **unprofiled** pairs with the same fixed RTC,
   NVRAM and disposable media. Retain both idle phases, the 30-command phase
   samples and fixed-work result. Report per-run and aggregate median/p95,
   command-window ordering and host load. Use a separate diagnostic repeat
   for guest-time/input-observation/output-publication timestamps if the
   first-window latency increase persists. Do not infer its cause from a
   profile-only latency or treat 30 commands as 30 independent boots.
5. Report once with the resulting evidence and limits. If CWP-7 parking works
   but gives no broad CPU gain, say so; that is not permission to broaden the
   recognizer. If the latency signal persists, keep performance acceptance
   open with the bounded result rather than repeatedly altering semantics.
   Run affected validation, rebuild the normal frontend, preserve source
   media/catalog and compact evidence, clean disposable builds/clones, and
   return uncommitted for review. Issue #5 remains open.

Review artifacts: `build.json`, `cwp_admission_review.rs`, `default.log`,
`profiling.log`, `measurement-audit.json` and `cleanup.json` under the review
directory above. No production source or submitted test was changed here.

## Follow-up: exact binary identifies the missing frontend patch

The independent fixture is now promoted in `tests/cwp_admission_review.rs`,
including CPU-only CWP invalidation and a truly inactive-window mutation.
Submitted full-suite logs contain 814 normal / 826 profiling successful
executions, no failures. These are execution counts: including
`parking_windows.rs` also repeats its 12 existing tests. Reviewed production
source hashes are unchanged.

Diagnostic per-CWP sums reconcile with aggregate candidates/commits. Idle
phases record CWP-7 candidates/commits of 1,326/1,289 and 39/35. Successful
core snapshot comparisons equal total commits. The transcript reaches
completed sync, OBP and host exit 0. Diagnostic idle worker CPU is about
7.80%/8.19%; this is not an unprofiled performance result.

The eight new unprofiled runs retain the reported arithmetic, stable binary
hashes, matching media inputs and clean halt. **Their candidate binary omits
the frontend call-site patch.** Independent release builds with the same
source/target paths established the following exact hashes:

| Rebuilt sources | Release Heliacal SHA-256 |
| --- | --- |
| Helios `ff078fa` + core patch; **unchanged Heliacal `8e8d82a`** | `bd12d0511ab5803c829683975d05fffa100cd1cd4ed896969c11b8114461a099` |
| Same core + **current Heliacal `src/emulator.rs` patch** | `60d9c3831fc29546d38163df6c5b9b617a7615d80b8a94284cf6dc92f5da3116` |

The first hash matches **all four reported unprofiled candidate runs** and
the candidate listed in `provenance.txt`. That old frontend still checks the
entire PSR at the worker candidate, ordinary preparation and profiled
preparation, excluding CWP 7 before reaching the new core. Therefore the
22.19%/43.18% idle figures and 27.47/21.03 ms command medians do not measure
the completed CWP feature. The repeat does not resolve its latency gate.

This is a demonstrated build-composition error, not a new theory about guest
CWP transitions or host scheduling. It explains why the unprofiled results
cannot be paired causally with the CWP-enabled diagnostic run. No additional
proof relaxation or guest-state investigation is justified by these results.

Correct `provenance.txt`: its claim that the measured frontend contains the
uncommitted call-site patch is false. Its single Cargo.lock hash belongs to
the core; the frontend build uses Heliacal's lockfile, SHA-256
`7a488fbf6fb5ccf8160e5e206f72bec759f1d3aa41fd25870501a004f2c66343`.
Retain both where relevant; do not silently relabel the historical runs.

### Finish using the correctly composed build

1. Preserve the eight runs as **core-only / old-frontend** evidence and
   retract their feature-level conclusions. Export both production patches,
   verify their source hashes before building default/profile candidates,
   and check all shared-predicate call sites. The review's source manifest
   and patches below provide exact inputs.
2. Rebuild both-patch candidates in isolated sibling source trees with
   separate targets and the frontend lockfile. The correct release build
   succeeded during review; no production fix is indicated. Different build
   paths/toolchains may change the binary hash: verify source composition
   rather than blindly demanding the review hash. Reject the known
   old-frontend candidate hash above.
3. Repeat the already bounded four-pair **unprofiled** comparison, retaining
   both idle phases, command median/p95, fixed-work result and clean halt.
   Add the requested CWP-0/7 ordinary/profile frontend fixture. Keep
   diagnostics separate and retain their patch/build recipe, still absent
   from this submission. Do not reopen the accepted loop proof.
4. Reconcile human docs with CWP 0–7/exact other PSR bits, retain all default-
   off/UP/proof/wakeup limits, rebuild normal Heliacal and return uncommitted.
   Preserve source media/catalog and compact evidence; remove scratch builds
   and clones. Issue #5 stays open.

Evidence: `local/review-issue5-cwp-admission-followup-2026-11-18/` contains
`reproduced-binaries.json`, the source/lockfile manifest, both production
patches, compiler versions, build logs, `reproduce.py`, the build recipe,
recomputed phase statistics and cleanup record. The first host-only build
failed because the host lacks `c++`; both successful release builds used
`my-distrobox` with the recorded Rust/C++ toolchains. This review ran no new
guests and wrote no source media or production files.

## Corrected-build acceptance

The new `corr-baseline-*` / `corr-candidate-*` input hashes match the two
independently reconstructed releases: baseline `bd12d051...` has the updated
core and original frontend; candidate `60d9c383...` has **both** patches.
Thus this comparison measures the completed frontend's use of the broader
core eligibility, against CWP-0-only frontend admission with the same core.
`provenance-corrected.txt`'s `core baseline: ff078fa` line should be read as
the base revision plus the core patch, not pristine `ff078fa`.

Independent arithmetic from raw worker tick deltas and command timestamps
matches the report. Ordering is B/C/C/B/B/C/C/B. Media/NVRAM/ROM hashes match;
all eight runs return the expected `4999950000` fixed-work result, complete
filesystem sync and halt to OBP, and exit 0. Scratch disks were removed.

| Measurement | CWP-0 frontend | CWP-0–7 frontend |
| --- | ---: | ---: |
| Pre-command idle worker CPU | 24.289% | 7.596% |
| Later idle worker CPU | 42.882% | 8.395% |
| First command median / p95 | 27.827 / 34.532 ms | 33.768 / 36.227 ms |
| Later command median / p95 | 22.321 / 33.523 ms | 33.361 / 35.555 ms |
| Fixed guest work wall / worker CPU | 5.366 / 5.335 s | 5.217 / 5.180 s |

Idle CPU reductions are **68.7% and 80.4%** relative to this baseline. The
first/later median command-response increases are **5.94 / 11.04 ms** and
p95 increases **1.70 / 2.03 ms**. These are medians of four per-run statistics,
not pooled percentiles or a universal responsiveness guarantee. Fixed work
shows no regression in these runs; its small improvement does not establish
faster CPU instruction execution.

The measured interval runs from immediately after a host PTY write to receipt
of the command's second marker. It includes serial delivery and guest
processing; it is not isolated keystroke latency or the duration of one host
wait. A 2 ms individual wait cap therefore does not, by itself, explain or
bound the whole command-response difference.

I independently added focused frontend fixtures in an isolated source copy:
**1/1 default and 2/2 profiling tests pass**, each exercising CWP 0 and 7.
They verify ordinary/profile preparation and commit agree, the 2 ms cap and
wait-before-advance ordering remain, full CPU state is preserved, queued
input prevents advancement without being consumed, and pending/active BREAK
rejects parking. The fixtures are retained for promotion; no frontend
production source was changed. Existing core/fixture source hashes match
the accepted earlier review. Full 814/826 core and 114/117 frontend validation
remains the submitted validation, not a newly rerun full suite here.

### Acceptance disposition

The correctness and idle-efficiency milestone is ready to checkpoint. Keep
parking **off by default**, confined to the proved single-CPU SS-20 Solaris
boundary, and document the measured response-time tradeoff. This is not a
claim that the latency increase is harmless, explained, or acceptable for a
default-on feature. No default switch, SMP expansion or weaker proof is
approved by this checkpoint; issue #5 stays open.

Core human docs describe CWP eligibility and exact stale-token equality.
Frontend housekeeping is now complete: the CWP-0/7 fixtures are promoted
without a separate duplicate target, and README/configuration docs describe
default-off SS-20/UP scope and the measured latency tradeoff. Submitted
frontend validation is **115 default / 119 profiling**, with clean Clippy.
Production core/frontend code hashes still match the reviewed candidate;
independent formatting checks pass. The checkpoint is ready to commit.
No additional broad benchmark or proof round is required for this checkpoint.
The next bounded task is [latency attribution](performance-issue5-parking-latency-handoff.md),
with no timing-policy changes before the delay is located.

Final audit and fixtures:
`local/review-issue5-cwp-admission-corrected-2026-11-18/` contains
`audit.py/json`, source hashes, `frontend_cwp_review.rs`, the isolated test
runner/logs and cleanup record. Temporary source/build files are removed;
no guest run, media write, live catalog edit, or production code edit was
performed during this final audit.

The subsequent host first-byte probe does not change checkpoint acceptance.
Its medians and clean halts are independently checked, with the reporting
limits recorded in the latency handoff. Worker PTY/SCC/guest-output/drain
attribution is still pending and belongs in the next diagnostic slice.
