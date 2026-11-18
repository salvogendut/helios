# Issue #5: core token sub-profile review

**Accepted 2026-11-18; no blocking findings remain.** The profiling checkpoint
is committed as Helios `21c9d2e` and Heliacal `8e8d82a`, before starting the
separate digest optimization. Guard order is restored, all nine
fixtures and the strengthened frontend comparisons are promoted, reporting is
corrected, and the exact benchmark source/runner is retained. No digest
optimization, commit, push or merge was performed by this review; #5 stays open.

The final review verified that both source corrections exactly match the
previously tested patches and reran formatting and the nine core fixtures.
The restored benchmark was independently rebuilt in separate default/profile
targets and completed 20,000 cycles/20,000 ns in each. This one-run-per-build
reproducibility check does not replace the retained eight-pair overhead result.
Corrected validation logs confirm 782/791/794 core executions and 114/117
frontend tests, with no failures. No new guest boot or repeated full validation
was needed for corrections identical to those already independently validated.

Final-check evidence is under
`local/review-issue5-token-profile-corrected-2026-11-18/`. Only review notes and
the index were changed in the live repositories; temporary builds were removed.
The resolved findings below retain the original rationale and test evidence.

## Admission order and comparison coverage (resolved)

The initial `src/machine.rs::parking_token()` evaluated `idle_boundary_proof()` before
checking CPU error/reset/PC/nPC/PSR. The accepted implementation short-circuits
those cheap checks first. This change also affects builds **without**
`parking-profile`: a wrong boundary or reset request unnecessarily traverses
the mapped instruction/data proof. The profiling work order explicitly
required preserving evaluation order. No guest-state corruption is established
by this finding; it is an unnecessary-work and measurement-order regression.

The applied correction moves the existing cheap rejection condition before the
timed proof, then rejects `!proof` separately without weakening it. The tested patch is
[`restore-guard-order.patch`](../local/review-issue5-token-profile-2026-11-18/restore-guard-order.patch).

Nine independent fixtures are retained in
[`token_profile_review.rs`](../local/review-issue5-token-profile-2026-11-18/token_profile_review.rs).
The initial candidate passed 6/9; wrong-PC, wrong-nPC and pending-reset fixtures each
observe one proof call where the original ordering requires zero. With the
patch, 9/9 pass. Other fixtures cover exact nested operation counts, stale
memory and CPU state, proof/readiness rejection, thread-local isolation and
profiling reset without token or machine mutation. They are now tracked in
`tests/parking_profile_review.rs` and pass 9/9.

The three Heliacal comparisons initially called
`prepare_park_profiled` and `commit_park_with_profiled` on **both** sides.
Setting the frontend profiler to `None` neither selects the ordinary helpers
nor disables core timers in a feature build. They verify optional frontend
recording, but not the claimed ordinary-versus-profiled helper comparison.
[`frontend-comparison.patch`](../local/review-issue5-token-profile-2026-11-18/frontend-comparison.patch)
makes the reference side use `prepare_park`/`commit_park_with`; this patch is now
applied and all three pass.

A separate cross-build fixture,
[`token_profile_state.rs`](../local/review-issue5-token-profile-2026-11-18/token_profile_state.rs),
also compares successful admission/validation/advance, CPU staleness and
interrupt-pending rejection in default and feature builds. Snapshot contents
and outcomes match. The comparison sorts only the complete key/value pairs
of the two MMU diagnostic HashMaps, whose Debug order varies across processes;
no state fields are omitted. Its driver and normalized hashes are retained.

## Evidence and reporting corrections

Independent arithmetic from the individual phase JSON files confirms:

| Phase | Token + advance | Parking-device digest | Share |
| --- | ---: | ---: | ---: |
| Pre-command idle | 1,668.826 ms | 1,563.955 ms | 93.716% |
| Post-command idle | 1,294.629 ms | 1,209.692 ms | 93.439% |
| First command windows, as reported | 181.988 ms | 170.774 ms | 93.838% |
| Second command windows | 66.052 ms | 61.619 ms | 93.288% |
| All command windows | 248.041 ms | 232.393 ms | 93.691% |

Both guest runs retain sync/halt and exit status 0. No new guest boot was
needed for this review.

The following reporting findings are now corrected in the report/summary:

- The timer surrounds **`parking_device_digest()`**, including `devices_digest`,
  bounded video state and MXCC formatting. The profile establishes the cost of
  that whole path. It does not separately measure Debug formatting, allocation,
  hashing, individual devices, or just the inner `devices_digest` function.
  Identifying typed encoding as the next candidate is a source-based inference.
- The published command column sums only `profile-command-0.json`. Label that
  selection explicitly and retain the second windows, or report all four.
  Run 1's second command window has no parking calls and about 98.97% worker
  CPU; run 2's has 156 successful tokens. This matters when estimating the
  eventual end-to-end benefit of faster token construction.
- The component totals include rejected calls. Rename “successful calls” to
  “all calls”; retain the separately recorded success/rejection totals.
  Validation inside advance is nested, so `token_ns + advance_ns` is the
  correct outer denominator for these guest runs. Do not add `validate_ns`.
- Proof/readiness/arbitration/deadline are approximately 0.36%/0.39% of the two
  idle totals, not below 0.3% combined; adding advancement gives about
  0.55%/0.59%. This does not change the ranking.
- The summary's `command.idle_cpu_percent` repeats the first idle samples.
  Remove or replace it with explicitly sourced command CPU data. Keep worker
  `Instant` intervals separate from external thread CPU measurements.

The 16 retained fixed-work logs reproduce the reported medians: 6.915726 s
default versus 6.857174 s profiling (-0.847%), and -0.799% CPU ticks. This is
consistent with no detected overhead for that fixture, not a general zero-cost
claim. The initially missing `parking_profile_bench.rs` and `run_token_bench.py`
are now retained alongside the original logs. Their source was inspected and
the benchmark independently rebuilt and run once in each configuration. An
explicit validate followed by advance includes **three** digests
per benchmark cycle, whereas the normal successful frontend cycle has two.

## Next slice: typed device digest

Proceed with a narrowly scoped allocation-free typed encoding of the device
digest. Further whole-worker profiling is not needed to choose this candidate.
Preserve the same state coverage and operations; no caching, proof relaxation,
generation-only substitutions, deadline changes or sleeping changes.

1. Inventory the current Debug/diagnostic coverage before replacing it. Include
   private FIFO contents, pending flags, timer/baud phases, control/status and
   latched state, NVRAM bytes and clock/latches, interrupt banks, and buffered
   network/SCSI data wherever the existing fingerprint includes them. Preserve
   the existing ESP/SCSI payload digests and documented media exclusions.
2. Use per-device typed digest methods where fields are private. Encode option
   and enum discriminants, sequence lengths and element order unambiguously.
   No host pointers, HashMap iteration order or external media reads. The
   resulting hash numbers need not equal the old Debug encoding.
3. Keep the old encoding as a test/reference path while checking coverage.
   Independent field mutations must still invalidate tokens and distinguish
   differential snapshots, including staged payload bytes and internal phase
   state. Comparing two machines through the same new incomplete digest alone
   would not detect a field accidentally omitted from both.
4. Measure fixed token work and matched frontend pre-command idle, later idle
   and command latency separately. Retain source/configuration/binary hashes
   and raw results. Run without profiling for the end-to-end performance
   comparison, using the opt-in profiler only for attribution. Keep the 2 ms
   cap, BREAK/input/network handling, default-off parking and SMP exclusion.

Commit the accepted profiling checkpoint first, keeping the core and frontend
changes in their respective repositories. The digest optimization should remain
a separate reviewable change; do not claim an achieved speedup from the
component percentage alone. Keep #5 open through this incremental work.

## Independent validation and cleanup

With the proposed guard-order fix in an isolated core:

- Full core tests: 782 default, 791 with `parking-profile`, 794 with both
  `parking-profile,perf-counters` successful executions. Each count includes
  the existing isolated MXCC child rerun; feature builds include the nine new
  review fixtures. The cross-build trace fixture ran separately afterward.
- Formatting, Clippy `-D warnings` in all three configurations, and all four
  demos pass.
- Isolated Heliacal with the strengthened helper comparisons: 114 default /
  117 profiling tests; formatting, both Clippy configurations and both SDL
  dummy/software smokes pass. No frontend production behavior was changed.

Evidence is under `local/review-issue5-token-profile-2026-11-18/`: source hashes,
candidate diffs, failure/passing logs, tested patches, fixtures, recalculation
script/results and validation manifests. Temporary source copies and build
directories are removed after review. Original media, the live catalog and
the submitted evidence are preserved.
