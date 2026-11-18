# Issue #5: parking-profile isolation slice

Status: independently accepted as the diagnostic-isolation slice on
2026-11-18; ready to checkpoint, still uncommitted. No blocking findings remain
for this slice. The preceding committed checkpoint is `a88d769` (help follow-up
`2682872`) on core `e3d87e3`. Issue #5 remains open; no commit, merge or push was
made by this review. Normal `--idle-parking` behavior is unchanged.

## Implementation

Added the default-off Cargo feature `parking-profile`:

- `src/parking_profile.rs` and its import are compiled only with the feature.
- The ordinary worker calls the original `prepare_park`/`commit_park`/
  `try_park` path with no profile argument, no `Option<ParkingProfile>`, no
  timers, no marker polling and no profile branches.
- The feature build uses separate profiled admission/commit functions and
  feature-gated timing/counter statements in the worker loop.
- Dead-code allowances apply only when the feature build still compiles the
  regular path for tests.
- Both builds retain the same parking logic, 2 ms wait cap, proof checks,
  input/BREAK/network checks and default-off behavior.

Default and feature release builds have distinct hashes. The default binary
contains no `HELIACAL_PARKING_PROFILE` strings; the feature build does. This is
the isolation check, not a claim that profiling runtime has zero cost.

## Actual-worker comparison against 2682872

Both trees use the same current Helios core, the same `Cargo.lock`, separate
target directories and the review-only fixed worker workload (12,582,912
outcomes, 2,097,152 register/RAM iterations). Twenty alternating pairs, with
profiling environment variables absent:

| Arm | Median wall | Median worker CPU |
| --- | ---: | ---: |
| Accepted `2682872` | 0.743849 s | 0.740 s |
| Candidate default build | 0.742845 s | 0.740 s |
| Change | **-0.135%** | **0.00%** |

The paired wall-time delta median is **-0.316%**. All runs have matching
outcomes, register/RAM checksum, virtual time and status. This removes the
previous **+4.19%** absent-profiler slowdown; the residual is within host
variation. Evidence: `local/issue5-parking-profile-isolation-2026-11-18/`
(`summary-20.json`, paired logs, build logs, binary hashes).

## Validation

- Frontend tests: **114/114** with diagnostics disabled and **114/114** with
  `--features parking-profile`.
- Formatting and Clippy `-D warnings`: pass in both configurations.
- SDL smoke: pass in both configurations.
- Original SS-20 OBP/SX boot smoke with `--idle-parking`: pass in both
  configurations (default 55,562,240 steps, PC=ffef0008; profile 55,998,464
  steps, PC=ffef0018; neither establishes OS boot). A profile-enabled smoke
  with `HELIACAL_PARKING_PROFILE` also completed and wrote its counter file.
- Independent parking fixtures: **14/14** protocol cases on the default build
  and **17/17** cases on the feature build, including the three profile-on/off
  state/counter comparisons.

Validation logs are under the same evidence directory. No deeper token
profiling or semantic optimization was performed in this slice.

## Independent acceptance review

Source comparison confirms that the bodies of `host_work_pending`,
`prepare_park`, `commit_park`, `commit_park_with` and `try_park` are unchanged
from `2682872`. Diagnostic imports, state, functions and worker statements are
behind `cfg(feature = "parking-profile")`; the ordinary worker retains its
direct parking path. The feature build deliberately pays diagnostic costs,
even when its runtime profiler is inactive; this is not a zero-overhead claim
for diagnostic builds.

Recalculation of all 40 raw benchmark logs reproduces the reported medians:
0.743848907 -> 0.742845025 s, **-0.134958%**, with **-0.315800%** median paired
change and 0.740 s median worker CPU time in both arms. Paired elapsed deltas
range from -3.30% to +2.91%; there is no persistent +4.19% slowdown in this
control. Outcomes, register/RAM checksum and virtual time match throughout.
This establishes the isolation result for the fixed worker workload, not a
new Solaris performance improvement.

Fresh isolated builds independently pass:

- **114 tests in each configuration**, formatting, the prescribed Clippy
  command with `-D warnings`, SDL smoke and release build.
- **14 default and 17 feature-path review fixtures**. The three differential
  cases now compare the ordinary helpers directly with the profiled helpers
  after success, input cancellation and stale-token rejection.
- With the profiling environment variable present, the default worker writes
  no diagnostic file; the feature worker does, with nonzero outcome counters.
- Default release hash `880f0284...` and feature release hash `0e988b68...`
  exactly reproduce the submitted hashes. The default has no profiling
  environment-name strings; the feature binary includes all four names.

The supplied original-OBP smoke results therefore refer to the same reproduced
binaries. They were inspected, not rerun; this review does not claim a new OS
boot. No core code changed, so unrelated core tests were not repeated.

Evidence: `local/review-issue5-parking-profile-isolation-2026-11-18/`, including
`evidence-review.json`, `validation.json`, `isolation-check.json`, `fixtures.json`,
source hashes/diff, scripts and review-only fixture patches. Temporary source
copies and build directories are removed after validation. Production source,
the installed frontend, guest media and live configuration are unchanged.

## Next checkpoint and work order

Checkpoint this isolation before extending the profiler. Keep issue #5 open
and parking default-off. The next slice is feature-gated core timing only:
split token issuance and parked commit into proof/readiness, device digest,
arbitration, CPU/MMU/MXCC snapshot/equality, deadlines and device advancement.
Measure instrumentation cost with fixed work, preserve both pre-command and
later idle windows, and distinguish elapsed durations from CPU time.

Promote the three ordinary-versus-profiled state/counter comparisons when
extending the feature path: the existing 114 tracked tests still call the
ordinary helpers, whereas the feature-path comparisons currently live in
review fixtures. Also reconcile the diagnostic `instructions` and
`commit_validation` names with their actual outcome/whole-advance scopes.
Keep the latency cause unresolved until correlated worker receive/publication
timestamps exist. No proof caching, check removal, changed wait cap or SMP
parking is part of the next measurement slice.
