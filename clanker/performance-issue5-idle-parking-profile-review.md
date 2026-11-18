# Issue #5: idle-parking profile review and next-slice handoff

Token-profile update 2026-11-18: core sub-profiling is committed as Helios
`21c9d2e` / Heliacal `8e8d82a` and the three state-comparison fixtures are
promoted; see
[token sub-profile](performance-issue5-token-profile.md). Device digest is
93.4–93.8% of token+advance elapsed, with timer overhead undetected. No
optimization is implemented; digest encoding is the narrow proposal.

Isolation accepted 2026-11-18: the default-off `parking-profile` feature resolves
the ordinary-build slowdown. Independent source, binary, test and measurement
checks pass; see [profile isolation](performance-issue5-idle-parking-profile-isolation.md).
The isolation is ready to checkpoint and remains uncommitted. The next slice
is feature-gated core timing with all safeguards unchanged. No merge or push
was made and issue #5 remains open.

The following records the earlier review before isolation. Phase separation and withdrawal of the
latency-cause claim are accepted. The fixed-work medians reproduce, but a new
independent control finds a repeatable slowdown when the profiler is absent.
Resolve that default-path cost before merging these diagnostics. No production
commit, merge or push was made. The accepted default-off checkpoint remains
`a88d769`/`2682872` on core `e3d87e3`; issue #5 stays open.

## Accepted evidence

The independently verified matched result remains **74.2% lower worker CPU**
(99.560% -> 25.687%), with median PTY latency 22.305 -> 22.145 ms and p95
29.768 -> 38.854 ms. The phase-separated idle rerun measured 28.4%/17.2% with
parking on versus 99.4%/99.6% off and is consistent, but does not replace the
accepted matched pair.

The corrected seven runs retain matching media hashes, expected AWK output
`4999950000`, clean guest sync/halt and zero frontend exit status. The four
phase runs use binary `c243b5c0...`; the three toggle runs use `4b37d5fe...`.
They must remain separate comparisons. Independent arithmetic and all retained
phase results are in
`local/review-issue5-parking-profile-corrected-2026-11-18/corrected-evidence-review.json`.

## Corrections applied

- Fixed 100,000-iteration guest work compares inactive/active instrumentation
  inside the same process, with parsed `perf stat` instructions/cycles.
  Medians reproduce: +0.764% worker CPU time, +0.0673% host instructions and
  +0.4795% host cycles. The earlier +1.30 point utilization-based overhead claim
  is withdrawn. The first active repeat is slower in all three runs;
  background guest work is a possible explanation, not an established cause.
- Idle and command phases now reset and dump separately. Idle elapsed is
  5.054/5.063 s with 3.613/4.174 s of host wait; command elapsed is
  1.006/1.007 s with 0.426/0.453 s wait. All elapsed fields are labeled worker
  `Instant` intervals, not CPU time.
- Latency causality remains unresolved. The harness has host-visible write,
  first-token, second-token and prompt timestamps but no worker input-observation
  or output-publication timestamps. The previous echo/output-tail explanation
  is withdrawn.

## Historical finding: ordinary-build slowdown (resolved by isolation)

`active=false` retains a `Some(ParkingProfile)`, marker polling, timer dispatch
and the new branches. It does not exercise the production `None` path, and
neither state compares with the accepted frontend before the profiling patch.
The AWK results therefore do not establish the default-path cost.

The independent review uses Heliacal's actual worker loop, built in release
mode with identical review-only setup/stop hooks. A synthetic SS-20/one CPU,
16 MiB, no display/network/disk workload completes **12,582,912 outcomes**:
2,097,152 six-instruction load/add/store/register-add/branch/delay iterations.
Both checked registers and the RAM word finish at 2,097,152; PC/nPC and virtual
time also match. Machine setup, final checks and NVRAM cleanup are outside the
measured interval. Both builds link the same current Helios core.

An initial valid five-run-per-arm control found +4.57% elapsed time with no
profiler. A separate eight-pair repeat, alternating which executable runs
first, confirms:

| Actual-worker control | Median elapsed | Median worker CPU time |
| --- | ---: | ---: |
| Accepted frontend `2682872` | 0.739572 s | 0.735 s |
| Profiling candidate, all profiler variables absent | 0.770552 s | 0.765 s |
| Change | **+4.19%** | **+4.08%** |

All eight candidate runs are slower than their paired accepted runs. CPU
seconds come from thread ticks and have 10 ms resolution here; elapsed times
are the stronger comparison. This is a repeatable synthetic worker control,
not a claimed Solaris boot regression or proof of one particular compiler/
branch cost. It nevertheless contradicts treating the +0.76% active/inactive
AWK delta as the complete diagnostic overhead.

Sources and commands are retained as `worker_bench.rs`, `prepare_bench.py`,
`run_bench.py` and `repeat_control.py`; results are `bench-summary.json` and
`control-repeat.json` in the review directory. Each build uses its own target
directory; executable hashes differ, and profiler output/counters verify the
active/inactive arms. An earlier shared-target attempt reused one executable
for both source trees and was explicitly discarded under
`discarded-shared-target/`; none of its numbers is used above.

## Preserve both idle regimes and use precise metric names

The 62.5% combined token/commit elapsed share applies to the first, pre-command
idle windows only. The retained second windows show a different regime:

| Window | Run 1 worker CPU | Run 2 worker CPU | Token/commit share outside waits | Normal-execution share outside waits |
| --- | ---: | ---: | ---: | ---: |
| First idle, before commands | 28.391% | 17.194% | 62.54% combined | 35.12% combined |
| Later idle, after commands | 53.556% | 51.586% | 17.84% combined | 80.58% combined |

The current evidence does not identify why the guest does more normal work
later. Do not discard that regime or infer that reducing token cost alone
will solve it. Preserve both and classify the work/opportunities in the next
measurement. The new profile labels correctly identify `Instant` intervals;
neither denominator is a CPU-time attribution.

Two earlier naming corrections remain: `commit_validation` times all of
`Machine::advance_parked`, including readiness, deadlines and device-time
advancement, and `instructions` counts successful execution outcomes, including
trap/interrupt entry. Call them parked commit/advance and execution outcomes,
respectively, or document legacy JSON names explicitly. The corrected report
now states the former scope; production field names have not been changed by
this review.

## Next work order

1. **Completed and independently accepted:** isolate diagnostics from the normal worker before adding more timers.
   Prefer an explicit profiling build feature, or equivalent dispatch that
   demonstrably removes profiling checks from the ordinary instruction loop.
   Preserve guest behavior and the accepted parking protocol. Rerun the
   supplied actual-worker comparison against `2682872`, with both diagnostics
   absent and enabled; retain distinct build outputs and final checks.
2. Under that opt-in mechanism, sub-profile core token issuance and commit:
   boundary proof/mapped reads, readiness, device digest, arbitration,
   CPU/MMU/MXCC snapshot, snapshot equality, deadline queries and device-time
   advancement. Separate success/rejection costs, outer totals and remainder;
   do not double-count nested intervals. Core diagnostics must remain outside
   architectural equality, token validity and ordinary builds, with no unsafe
   core code or changes to evaluation order.
3. Add fixed-work token/candidate controls for timer overhead. Measure both
   pre-command and later idle, recording guest outcomes and admission/
   rejection counts. A longer settle alone is not evidence that background
   work has stopped. Retain all phase results and exact raw `perf stat` output,
   exit status and measurement intervals for future PMU comparisons.
4. Before attributing or optimizing the latency tail, add bounded opt-in worker
   input-observation and output-publication timestamps correlated with the
   host transaction timestamps. Keep its cause unresolved until then.
5. Promote the three profile-on/off state/counter comparisons retained in the
   review fixtures, and cover the inactive/toggle path consistently. Run the
   prescribed checks for affected repositories with diagnostics absent and
   enabled; retain existing token-proof and host-wakeup coverage.

Return the default-path control, component costs and a small evidence-based
proposal. No proof caching, check removal or guest semantic optimization in
this slice. Preserve the **2 ms cap**, proof/validation, BREAK/input/network
checks, default-off parking and SMP exclusion. Keep the changes uncommitted
for review and issue #5 open.

## Independent validation and cleanup

The updated candidate passes 114 frontend tests, formatting, Clippy with
`-D warnings`, SDL dummy/software smoke and release build. All 17 independent
parking/profile-state fixtures pass again with profiling enabled. Only
`src/emulator.rs` and `src/parking_profile.rs` changed since the preceding
source review; no core behavior changed and no unrelated core suite or new
Solaris boot was repeated.

The original submitted notes are retained beside the review evidence.
Production code and the installed executable were not modified. Review source
copies, builds and benchmark executables are removed after checks; compact
logs, source hashes/diffs, fixtures and reproduction scripts are retained.
Source media and live settings were untouched. No commit or push was made.

The current profiling code and these notes are uncommitted for review. Raw
corrected evidence is under `local/issue5-parking-profile-corrected-2026-11-17/`.
