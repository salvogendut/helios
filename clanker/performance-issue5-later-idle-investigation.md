# Issue #5: later-idle CPU investigation assignment

Status: executed with aligned PC sampling and exact-frontend-predicate tracing;
uncommitted investigation results. Helios checkpoints:
typed digest is Helios `ff078fa` (source `939a7bc`), baseline is `316ea8a`, and
Heliacal is fixed at `8e8d82a`. Issue #5 stays open. No semantic optimization
was made.

## Question

Explain the later post-command idle CPU increase seen after the typed-digest
change. Determine whether it is caused by more normal guest work, fewer
parking opportunities, retained/rejected parking work, or host scheduling.
Keep pre-command idle, command, and post-command idle results separate; do not
merge or discard either idle regime.

## Required measurements

- Repeat matched pre-command idle, command, and post-command idle phases for
  both `316ea8a` and `ff078fa`, with Heliacal fixed at `8e8d82a`.
- Alternate baseline/candidate order across repeated pairs; run one guest at
  a time. Keep build settings, machine configuration, commands, networking,
  warm-up and phase durations identical, and record each run separately.
- Use disposable reflink disk/NVRAM/media clones per run and retain source
  hashes. End with guest `sync`/`halt` and record frontend exit status.
- Control the initial guest clock as well as the NVRAM bytes. Heliacal defaults
  to `rtc_host_time = true`, which seeds a different clock on each launch.
  Use `rtc_host_time = false` with identical disposable NVRAM seeds for the
  controlled comparison; record the guest starting date and clock mode.
  Keep the earlier host-time runs identifiable as a separate experiment.
- Record per phase:
  - successful execution outcomes and virtual-time advancement;
  - parking opportunities and admission/rejection reasons;
  - committed parked virtual time and actual host wait time;
  - worker CPU from `/proc` samples and actual command latency from host write
    to the second output token.
- Retain exact phase boundaries and fieldwise counter deltas. Report work per
  host second and per advanced virtual second: faster token construction can
  change how much guest time fits in a fixed host-time window. Separate
  execution, parked advancement and actual host waiting rather than assuming
  equal-length host windows contain equal guest work.
- Use diagnostics only behind the existing default-off `parking-profile`
  feature for attribution. Do not use profiling-enabled measurements for the
  actual CPU/latency comparison or as the end-to-end performance result.
- Retain raw phase logs and a compact summary; remove temporary source/build
  copies and media clones.

## Deliverable

Return a supported explanation of the later-idle difference, distinguishing
guest-work, parking-opportunity, rejection/flush, and host-scheduling
contributions. Include the smallest justified next change only if the evidence
supports one. Do not implement a semantic optimization in this slice.
If the increase does not reproduce consistently or attribution remains
ambiguous, report that limitation and the remaining hypotheses explicitly.

## Preserve

- Admission order, proof and readiness checks.
- BREAK/input/network wakeup handling and the 2 ms wait cap.
- Default-off parking and SMP exclusion.
- Both pre-command and post-command idle regimes and actual command latency.
- Existing ESP/SCSI digest behavior and typed-digest state coverage.

Keep all changes and the investigation notes uncommitted for review.


## Results

Four alternating actual-CPU runs (baseline, candidate, candidate, baseline)
and four alternating diagnostic runs used separate disposable disk/NVRAM
clones, `rtc_host_time = false`, identical seed NVRAM, and guest date
`Sat Jan 1 01:02:45 MET 2000` in all eight runs. Every run ended with `sync`,
`halt` and frontend exit 0. Diagnostic runs used the feature-gated profiler for
attribution only; actual CPU/latency came from the unprofiled runs.

Actual unprofiled phase medians:

| Phase | Baseline `316ea8a` | Typed digest `ff078fa` |
| --- | ---: | ---: |
| Pre-command idle CPU | 43.47% | 22.48% |
| Post-command idle CPU | 23.19% | **43.98%** |
| Command CPU | 66.14% | 60.83% |
| First command median / p95 | 29.22 / 39.66 ms | 29.57 / 38.61 ms |
| Post command median / p95 | 21.00 / 30.46 ms | 22.54 / 33.54 ms |

The later-idle CPU increase reproduces, but the two idle regimes swap: typed
digest is much lower before commands and much higher after. Command latency
is not clearly improved; the later median and p95 are somewhat higher. Two
unprofiled repeats per arm do not establish latency equivalence or its cause.

Diagnostic phase medians:

| Phase | Outcomes | Outcomes/host s | Outcomes/virtual s | Virtual advanced | Wait | Committed parked | Candidates / commits |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline pre | 19.71 M | 3.90 M | 4.22 M | 4.673 s | 2.799 s | 2.701 s | 1,397 / 1,366 |
| Typed pre | 13.25 M | 2.62 M | 2.72 M | 4.881 s | 3.849 s | 3.556 s | 1,841 / 1,798 |
| Baseline post | 2.44 M | 0.48 M | 0.61 M | 3.868 s | 3.764 s | 3.624 s | 1,868 / 1,830 |
| Typed post | 24.70 M | **4.90 M** | **4.64 M** | **5.302 s** | 3.106 s | 2.832 s | 1,460 / 1,431 |

The higher later idle CPU is accompanied by far more successful guest
execution per host second and per advanced virtual second, fewer parking
candidates/commits, less committed parked time and less host wait. First-command
phases are similar (about 5.5–6.0 M outcomes, 0.96 s virtual, 0.19–0.21 K
commits after host-work rejection). Rejections are overwhelmingly token
candidates (`prep_token` about 19–43 per phase); host-work/deadline/BREAK
rejections remain negligible.

Supported association: the diagnostic runs show more guest execution and
fewer recognized parking boundaries/commits in the later window; the separate
unprofiled runs show higher worker CPU. Outcome counts alone cannot distinguish
useful work, background work, and idle polling. The reason for the additional
execution remains **unresolved**.

Lower aggregate wait does not exclude host scheduling effects. There are
fewer waits, but the median of the per-run mean actual-minus-requested wait
rises from 76.4 to 189.5 microseconds in the later window. This neither proves
that scheduling caused the CPU increase nor supports excluding it.

The diagnostic post-command windows also start at different guest times:
median 117.250 s for baseline and 118.110 s for typed digest. Normalizing by
advanced virtual time does not align the guest events being observed. In all
16 diagnostic phases, `virtual_advanced_ns` is exactly
`100 * outcomes + committed_virtual_ns` for this fast-clock UP configuration.
The larger virtual-time delta is therefore partly a consequence of the extra
execution, not independent evidence of its cause. The earlier idle window
shows the opposite balance, so a single global parking explanation is
insufficient.

Smallest justified next step is measurement-only: capture guest PC/process
sample categories or align phase windows by virtual-time advancement to
identify the extra later-window guest work. No semantic optimization is
justified by this evidence yet.

Evidence: `local/issue5-later-idle-2026-11-18/` (`summary.json`, phase logs,
profile JSON, guest dates, input hashes; temporary source/build copies removed).

## Independent review and next measurement

Independent recalculation confirms the reported phase medians, matching media
hashes and guest dates, and clean shutdown records. Post-command admission
success is approximately 98% in both arms (97.992% baseline, 98.011% candidate).
The reduction in commits accompanies fewer recognized boundary visits, rather
than a materially worse rejection rate. The observed CPU increase is accepted;
its causal attribution remains open. Recalculation source and results are in
`local/review-issue5-later-idle-2026-11-18/{audit.py,audit.json}`.

The previously missing diagnostic source is now retained: the JSON includes
`virtual_start_ns`, `virtual_end_ns`, and `virtual_advanced_ns`, but pinned
Heliacal `8e8d82a` does not emit those fields and the retained `measure.py` does
not add them. The exact measurement patch and build recipe are now retained as
`local/issue5-later-idle-2026-11-18/diagnostic-virtual-time.patch` and
`diagnostic-build.md`. Diagnostic builds are labeled `8e8d82a + measurement
patch`. The patch applies cleanly to the clean source and is feature-gated;
the existing diagnostic logs came from this patch. Do not treat it as part of
the unmodified frontend source.

Review confirmed that the restored patch applies cleanly to `8e8d82a` and
records the virtual clock at the existing profile reset/dump boundaries. It
does not change execution or parking decisions. The build recipe identifies
both Helios revisions and requires separate source/target directories. No
rebuild was performed during this provenance check; original binary identity
has not been independently reproduced.

The next assignment remains measurement-only:

1. Preserve the controlled setup and separate unprofiled CPU/latency results.
   In diagnostic builds, collect bounded PC/outcome samples and counter deltas
   in common absolute `Machine::virtual_ns()` bins covering both idle windows
   (for example 110–126 s in 100 ms bins). Record actual endpoints and overshoot;
   do not change stepping, deadlines, or parking to force alignment. Retain
   host-time phase markers as well as the virtual-time bins.
2. Separate the proven idle loop/helper, other kernel execution, user execution,
   synchronous traps, and interrupt entries. Use returned execution metadata
   and read-only symbol inspection; never fetch through the live MMU for
   tracing. Retain raw top-PC counts if symbol attribution is unavailable.
   Record the sampling method and avoid a stride that aliases a short loop.
3. Record guest virtual time when command bytes are delivered to the guest,
   together with host timestamps. Identical host command sequences can reach
   different guest times; aligned bins alone do not prove identical guest work.
4. Per bin, retain outcomes, candidates, commits/rejection reasons, requested
   and actual wait, per-wait overrun, and committed parked time. Determine
   whether extra execution is shifted work, unrecognized idle polling, or
   identifiable foreground/background activity. Keep scheduling attribution
   unresolved unless the new evidence separates its contribution.
5. Keep added diagnostics behind the default-off feature and retain their
   exact patch, build commands, and measurement limits. Return results
   uncommitted. No semantic optimization, wait-cap change, proof relaxation,
   default change, or SMP parking is authorized by this assignment.

This review changed documentation only and created compact audit artifacts.
No guest run, production edit, commit, push, or merge was performed.


## Aligned virtual-time PC sampling

Four diagnostic runs (baseline/candidate/candidate/baseline) used the retained
`diagnostic-pc-sampling.patch` on Heliacal `8e8d82a`, `rtc_host_time = false`,
identical NVRAM seeds and media clones. Bins were 100 ms over absolute guest
virtual time 110–126 s; PC sampling used stride 251, with exact category counts
for every outcome and raw top-PC counts per bin. Both arms retained the same
guest date and clean sync/halt/exit status.

Independent review excludes partially covered boundary bins. All counts below
are **sums across two runs per arm**, not per-run medians; intervals are
half-open and fully covered by all four diagnostic runs:

| Common interval | Outcomes | Idle loop/helper | Candidates / commits |
| --- | ---: | ---: | ---: |
| Pre [111.3, 114.9) s baseline | 27,321,526 | 25,937,546 | 2,308 / 2,256 |
| Pre [111.3, 114.9) s typed | 27,321,526 | 25,937,546 | 2,308 / 2,256 |
| Post [118.2, 120.2) s baseline | 335,604 | 22,956 | 2,044 / 2,004 |
| Post [118.2, 120.2) s typed | 335,586 | 22,956 | 2,044 / 2,004 |

The two baseline post phases actually end at 120.3718612 and 120.2538686 s;
the typed phases end at 123.1774017 and 123.2699713 s. Over the candidate-only
**fully covered [120.4, 123.1) s** interval, the two typed runs together execute
**42,770,648 outcomes**, including **41,524,372 (97.09%)** in the idle-loop/helper
address ranges, with **578 candidates / 570 commits (98.62%)**. Top sampled PCs
match the 14-instruction path, including `0xf006fa24` and `0xf0071708/170c`.
The remaining counts are 1,243,232 other-kernel outcomes, 2,106 user outcomes,
394 traps and 544 interrupts. Baseline post phases do not cover this interval.

The submitted 45.27 M / 680 / 668 totals included bins labeled from 120.3 s,
including partial overlap with baseline and partial final candidate bins.
`pc-analysis.json` instead starts the extra region at the 120.4 s bin and
reports 45.244 M / 578 / 570, also including partial final bins. Both versions
are reproduced in the independent audit; use the fully covered intervals
above for matched comparisons rather than mixing endpoint conventions.

The additional diagnostic execution is predominantly **idle-loop/helper
execution outside the baseline post-phase coverage**. Ordinary foreground or
other-kernel execution is a small minority. This establishes the location of
the extra execution, not equivalence to the complete state required by the
parking proof, and not a direct host-CPU attribution from PC counts. The
unprofiled CPU/latency numbers above remain the earlier separate experiment.
The reason the typed post phase extends farther in virtual time and why most
loop entries fail the exact frontend candidate predicate remains unresolved.

The earlier 76 -> 190 microsecond wait overrun does **not** repeat in this
PC-sampling series: the median of per-run post-phase mean actual-minus-requested
wait is 185.03 -> 124.44 microseconds. This does not establish or exclude a
scheduling cause; do not combine the earlier wait measurements with the new
PC-bin counts as though they came from the same runs. Command-delivery guest
virtual timestamps remain missing, so aligned bins alone do not establish
identical guest state at command boundaries.

Evidence: `local/issue5-later-idle-2026-11-18/pc-analysis.json`, `pc*/`
profile JSON/logs, `diagnostic-pc-sampling.patch` and `diagnostic-pc-build.md`.
Temporary source/build copies were removed.

## PC-sampling review and focused next assignment

Review confirms the diagnostic patch applies to `8e8d82a`, outcome/category
counts reconcile in all eight idle phases, and fully covered common bins
support the findings above. The retained independent recalculation is
`local/review-issue5-later-idle-2026-11-18/{pc_audit.py,pc_audit.json}`.

Source inspection narrows the next trace. Heliacal tests the candidate after
**every completed step**, not just at a 2,048-step batch boundary. It requires
`idle_parking`, `PC=0xf006fa24`, `nPC=0xf006fa28`, and the entire
`PSR=0x404000e0`. Candidate counters therefore omit loop entries with a different
nPC or PSR before the core token/proof is ever called. High admission success
only describes visits that passed this earlier predicate.

Run the following bounded diagnostic continuation; no new broad benchmark
campaign or semantic optimization is needed first:

1. At the existing post-step point, before the candidate predicate, count
   **every state with current `PC=0xf006fa24`**, including those that fail the
   predicate. Per virtual-time bin, record a bounded histogram of nPC, raw PSR,
   and `PSR XOR 0x404000e0`; decode differing CWP, ICC, EF, PIL, ET and S/PS
   fields. Keep separate counters for PC-only, PC+nPC and full-predicate
   matches, then core admission/rejection and commits. This locates the
   exclusion instead of counting only successful candidates again.
2. Retain short transition records when the dominant mismatch appears or
   disappears, with virtual/host timestamps and returned step metadata. Use
   read-only register access and bounded storage; never fetch instructions
   through the live MMU. Only inspect additional proof dependencies if the
   full frontend predicate succeeds but token admission fails.
3. Complete the missing serial observation: at actual core SCC input acceptance,
   record guest virtual time, host time, byte count/order, and retries. Record
   the host PTY read separately. Preserve the commands and timing policy;
   observation must not insert input early or alter the wait path.
4. Compare the dominant mismatch in both core versions over the same fully
   covered bins, labeling each run and all partial endpoints. Retain the exact
   diagnostic patch/build recipe, default-off feature isolation and compact
   evidence, then clean disposable builds/media and return uncommitted.

Do not mask off PSR bits, broaden the accepted loop, relax proof/readiness,
change the 2 ms cap, or alter virtual time. Even a seemingly irrelevant PSR
bit needs a separate proof/state-preservation review before accepting more
states. The stopping point is identification of the rejected boundary state
and its transition, with any remaining causal limitation explicit.


## Exact frontend predicate trace

A second four-run diagnostic series used Heliacal `8e8d82a` plus
`diagnostic-loop-state.patch`, the same controlled RTC/NVRAM/media setup and
alternating order. Every post-step state with `PC=0xf006fa24` was counted
before candidate filtering. nPC matched `0xf006fa28` in **every** loop entry;
the exclusion is PSR-only.

Fully covered intervals, two runs per arm:

| Interval | Outcomes | Loop entries | PC+nPC matches | Full predicate | PSR `e7` / `e0` |
| --- | ---: | ---: | ---: | ---: | ---: |
| Pre [111.3,114.9) baseline | 27,321,526 | 1,853,188 | 1,853,188 | 2,308 | 1,850,880 / 2,308 |
| Pre [111.3,114.9) typed | 27,321,526 | 1,853,188 | 1,853,188 | 2,308 | 1,850,880 / 2,308 |
| Post [118.2,120.2) baseline | 335,586 | 2,044 | 2,044 | 2,044 | 0 / 2,044 |
| Post [118.2,120.2) typed | 335,586 | 2,044 | 2,044 | 2,044 | 0 / 2,044 |
| Typed [120.4,123.1), retained comparison interval | 42,770,648 | 2,966,198 | 2,966,198 | **578** | **2,965,620 / 578** |

PSR `0x404000e7` differs from the required `0x404000e0` only in CWP
(`xor 0x7`); PC and nPC are otherwise exact. In the extra typed interval,
**99.98% of loop entries fail only the CWP check**. Only 578 full-predicate
visits survive, yielding 570 commits. This explains the reduced candidate
count without a proof/readiness rejection increase.

Dominant mismatch transitions:

- Both arms enter pre-idle at PSR `e7` and switch to `e0` at virtual
  **112.647 s**.
- Baseline post-idle remains `e0` until its phase ends (120.6438491/120.6098478 s
  in `loop1`/`loop4`; the earlier 120.372/120.254 figures belong to the PC-only
  experiment).
- Typed post-idle starts at `e0` (118.168/118.182 s) and switches to `e7` at
  **120.967 s**. That CWP-7 state persists through the extra loop execution.
- Later command windows show CWP mixes (`e3/e4/e7`); no candidate is accepted
  outside the full predicate.
- Baseline also reaches CWP 7 during command-1, first at 120.9715873 s in
  `loop1` and 120.9667567 s in `loop4`. The state is not unique to the typed
  digest. These are first observations at the idle PC, not traces of the
  instruction that changed CWP.

In this series [120.4,123.1) s partly overlaps baseline post-idle coverage.
The fully covered candidate-only interval is now [120.7,123.1) s: 42,723,258
outcomes, 2,965,892 loop entries, 2,965,620 with CWP 7 (99.9908%), and
272 full-predicate visits / 268 commits, summed over two typed runs. This
endpoint correction leaves the immediate exclusion mechanism unchanged.

Actual SCC input-acceptance timestamps were recorded per byte with
phase-relative host time and the PTY buffer-length increase during the service
poll. Every command phase delivered the same ordered 360 bytes. First/last
accepted-byte guest times, per run (seconds, rounded):

| Run | Arm | Command-0 first / last | Command-1 first / last |
| --- | --- | ---: | ---: |
| loop1 | Baseline | 115.168 / 116.043 | 120.728 / 121.642 |
| loop4 | Baseline | 115.280 / 116.153 | 120.686 / 121.562 |
| loop2 | Typed | 116.282 / 117.193 | 123.450 / 124.322 |
| loop3 | Typed | 116.270 / 117.193 | 123.464 / 124.332 |

`pty_read_bytes` is a byte-count delta attached to an accepted-byte record,
not an independent PTY-read timestamp or latency. Zero means that this service
poll added no buffered bytes; previous polls may already have read them.

The extra CPU is therefore unparked idle-loop execution under **PSR CWP=7**,
which the exact frontend predicate correctly rejects. What causes the CWP
switch at 120.967 s and why the typed phase reaches it remains unresolved;
timing/background activity or host scheduling remains possible, and no PSR
relaxation is justified. Command delivery occurs at different guest times, so
host-identical sequences are not guest-identical at phase boundaries.

Evidence: `local/issue5-later-idle-2026-11-18/loop-analysis.json`, `loop*/`
profiles and logs, `diagnostic-loop-state.patch`, `diagnostic-loop-build.md`.
Temporary builds/source copies cleaned. No production changes or commits.

Independent review accepts the immediate exclusion finding. The restored patch
applies cleanly to `8e8d82a`; all 16 phase outcome/entry histograms reconcile,
transition buffers are not truncated, and all command byte sequences match.
Raw recalculation is retained in
`local/review-issue5-later-idle-2026-11-18/{loop_audit.py,loop_audit.json}`.
The next bounded assignment is the
[CWP state-invariance proof](performance-issue5-cwp-proof-handoff.md): tests and
captured-state replay before any production predicate change. More broad host
profiling is not a prerequisite for that proof.
