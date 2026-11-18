# Issue #5: core parking-token sub-profile

Status: accepted profiling checkpoint, committed as core `21c9d2e` and
frontend `8e8d82a`; the preceding baseline was core `bcf1c09` / frontend
`9b21916`. Issue #5 remains open. The review guard-order correction is applied: cheap CPU/reset/PC/nPC/PSR rejection again precedes the
mapped proof, and nine review fixtures are promoted. No caching or semantic
optimization was performed. Parking still requires the exact proof, readiness, wakeup and
2 ms wait rules; default-off and SMP exclusion are unchanged.

## Instrumentation

Added the default-off Helios feature `parking-profile`; Heliacal's existing
`parking-profile` feature enables it. The core records one elapsed interval
per operation and aggregates by operation, with separate token/validation/
advance success and rejection counts, split elapsed totals, proof/readiness
outcomes, digest/arbitration/snapshot call counts, snapshot-equality outcomes
and deadline/device-advancement counts. The state lives in a thread-local
aggregate and is reset with the frontend profiling phase. Normal builds do not
compile the module or any timing statements.

The three profiling state-comparison fixtures are now feature-gated tests in
`src/emulator/parking_tests.rs`; they compare diagnostics-off and diagnostics-on
snapshots for successful parking, input cancellation and stale-token rejection.

## Measurement method

Two release runs on SS-20/one CPU, 128 MiB, Solaris 2.6, headless PTY,
networking disabled. Each run had two 5-second idle windows, 30 command trials
per command phase, and guest `sync`/`halt`. The first idle window is before
commands; the second is after commands. Each phase resets the frontend and core
aggregates. Core values below are worker elapsed intervals, not CPU time.

Timer overhead used fixed work: 20,000 token-issue + validation + 1 ns advance
cycles through the real core, eight alternating pairs of default and
`parking-profile` release test binaries, each in a separate target directory.

## Component costs, all calls

| Component | Pre-command idle | Post-command idle | All command windows |
| --- | ---: | ---: | ---: |
| Token + advance elapsed | 1,668.83 ms | 1,294.63 ms | 248.04 ms |
| Device digest | 1,563.96 ms (**93.72%**) | 1,209.69 ms (**93.44%**) | 232.39 ms (**93.69%**) |
| CPU/MMU/MXCC snapshot | 36.42 ms (2.18%) | 29.75 ms (2.30%) | 5.61 ms (2.26%) |
| Snapshot equality | 45.32 ms (2.72%) | 36.30 ms (2.80%) | 6.64 ms (2.68%) |
| Proof | 2.87 ms (0.17%) | 2.41 ms (0.19%) | 0.45 ms (0.18%) |
| Readiness | 1.21 ms (0.07%) | 1.03 ms (0.08%) | 0.17 ms (0.07%) |
| Arbitration | 0.22 ms (0.01%) | 0.17 ms (0.01%) | 0.03 ms (0.01%) |
| Deadline query | 1.76 ms (0.11%) | 1.49 ms (0.11%) | 0.22 ms (0.09%) |
| Device advancement | 3.04 ms (0.18%) | 2.54 ms (0.20%) | 0.41 ms (0.17%) |
| Unaccounted | 14.03 ms (0.84%) | 11.25 ms (0.87%) | 2.12 ms (0.85%) |

The component measurements were captured before the guard-order correction.
That correction only moves cheap rejection ahead of the proof; the successful
path and these component costs are unchanged. Validation, fixtures and
validation counts were rerun after the correction.

Each successful frontend cycle performs two device digests, one during token
construction and one during the validation inside advance. Per digest:
**218.4 us** pre-command, **223.8 us** post-command, **202.8 us** across all
command windows. The command column includes all four command windows; run 1's
second window had no parking calls, while run 2's second window had 156
successful tokens. The first-window-only subtotal is 181.99 ms / 93.84%.
CPU/MMU/MXCC snapshot plus equality is about 4.9% of the pair.

## Successful and rejected calls

| Phase | Tokens success / rejected | Successful token elapsed | Rejected token elapsed | Validation success / rejected | Advance success / rejected |
| --- | ---: | ---: | ---: | ---: | ---: |
| Pre-command idle | 3,580 / 81 | 803.86 ms | 0.05 ms | 3,580 / 0 | 3,580 / 0 |
| Post-command idle | 2,703 / 76 | 614.24 ms | 0.05 ms | 2,703 / 0 | 2,703 / 0 |
| All command windows | 574 / 61 | 119.97 ms | 0.045 ms | 572 / 0 | 572 / 0 |

Rejected token attempts are cheap because proof/readiness rejects occur before
the expensive snapshot and digest path. There were no rejected validation or
advance calls in these runs.

The idle CPU samples were 33.78%/18.99% before commands and 41.39%/45.38%
after commands; the latter window does more normal guest work and is retained
separately rather than merged. First command-window CPU was 60.1%/62.3%;
second command-window CPU was 98.97%/63.70%. These are external `/proc` CPU
observations, not denominators for the core elapsed components above.

## Timer overhead, fixed work

| Arm | Median wall | Median worker CPU |
| --- | ---: | ---: |
| Default build | 6.915726 s | 6.885 s |
| `parking-profile` build | 6.857174 s | 6.830 s |
| Change | **-0.85%** | **-0.80%** |

All runs produced 20,000 advances of 1 ns. The fixed-work comparison does not
show detectable timer overhead; the small negative difference is host noise.
This measures the timers, not their component shares. The retained benchmark
does an explicit validation plus advance, so it includes three digests per
cycle, while a normal successful frontend cycle has two; the overhead result is
not a general zero-cost claim. Exact source and runner are retained beside the
logs.

## Narrowly justified proposal

Target **device-digest construction only**. It is 93.4–93.8% of token+advance
elapsed and is executed twice per successful frontend cycle. Proof, readiness,
arbitration and deadline queries are about 0.36%/0.39% of the two idle totals
(about 0.55%/0.59% including advancement), so they are not optimization
candidates. The timer covers the whole `parking_device_digest()` path,
including `devices_digest`, bounded video state and MXCC formatting; it does
not separately isolate Debug formatting, allocations, hashing or individual
devices. The typed-encoding proposal is therefore a source-based inference
from that whole-path cost, not a measured formatting breakdown. Preserve the
same state coverage and comparisons; no caching, proof relaxation,
generation-only substitution, deadline change or sleeping change.

## Validation and evidence

- Core successful test executions: 782 default, 791 with `parking-profile`
  (including the nine promoted profile review fixtures), 794 with
  `parking-profile,perf-counters`; formatting, Clippy and all four demos pass
  in both relevant configurations.
- Default frontend: 114 tests; `parking-profile`: 117 tests (114 plus the
  strengthened helper state comparisons); formatting, both Clippy
  configurations, SDL smoke and original SS-20 OBP boot smoke pass. Default
  binary has no `HELIACAL_PARKING_PROFILE` strings; profile build does.
- Evidence: `local/issue5-parking-token-profile-2026-11-18/`
  (`token-profile-summary.json`, `profile-*.json`, `token-bench.json`, logs,
  binary hashes).
