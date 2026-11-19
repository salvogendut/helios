# Where the short ADD/branch loop spends time

At Helios `eb3d0bd`, span admission was the largest measured group of extra host work on the warm SS-20 ADD/branch/delay loop. An independent instruction-pointer sampler put its JIT-minus-interpreter cost at **28.1, 31.1 and 28.4 ns per guest outcome** in three runs. This is a synthetic-loop finding. The previous Solaris acceptance decision stands.

## Submitted timings

The retained file has three groups in interpreter/JIT/JIT/interpreter order, giving six observations per mode. The two observations per group have the following midpoints. The pooled row uses the conventional median of all six observations per mode.

| Group | Interpreter ns/outcome | JIT ns/outcome |
|---|---:|---:|
| 1 | 54.04 | 82.89 |
| 2 | 53.08 | 83.12 |
| 3 | 52.89 | 83.14 |
| Pooled median | 53.14 | 83.12 |

Each timed JIT arm retired 2.12 million of 2.4 million outcomes natively in 720,000 calls. The timed loop made 1 million span API calls, including 280,000 interpreted fallbacks. The warm-up was 24,000 guest outcomes.

The section-timed JIT run took 147.06 ns/outcome, about 77% more than the pooled uninstrumented JIT median. Its outer timed loop took 352.95 ms. The section accumulator reported 284.62 ms and included warm-up. Subtracting its four top-level fields leaves 120.17 ms inside that accumulator. Its nested generated-code field is 13.00 ms in the raw line, which leaves 36.67 ms inside the execution wrapper. The retained directory contains the timing output only. The timer placement and the cause of the residual cannot be reconstructed from those two files.

## Independent measurement

The independent probe used the same committed emulator and fixed workload, with line tables in a release build. It sampled only between warm-up and final checksum validation. Three pairs of each mode ran with sampling enabled and three with sampling disabled, in alternating order, with 2.4 million outcomes per arm. All arms produced checksum 808000 and the expected PC/nPC, virtual time and native counts. The no-sampling medians were 54.21 ns/outcome interpreted and 84.35 with JIT. The sampled medians were 53.77 and 83.15 respectively. One sampled interpreter arm took 65.51 ns/outcome.

A periodic wall-clock signal sampled the single benchmark thread at intervals of 223, 251 and 269 microseconds in the three pairs. The sampler retained 1,666 interpreter and 2,437 JIT interrupted PCs, symbolized with DWARF inline frames. Each estimate weights the sampled fraction by that arm's elapsed time per outcome, then subtracts the matching interpreter estimate. Figures below are medians of the three differences. Component medians need not sum to the group median.

| Sampled work | Extra ns/outcome |
|---|---:|
| Admission, all four components | **28.4** |
| ↳ Device readiness, deadlines and projected time | 12.2 |
| ↳ Fetch proof and RAM word peek | 8.2 |
| ↳ Span cache lookup | 5.7 |
| ↳ Span eligibility and decode | 2.4 |
| Constructing, returning and checking outcomes | 13.1 |
| Native execution wrapper, including register transfers | 6.0 |
| Ordinary fetch, MMU and RAM reads | −19.1 |

The device/clock group includes both readiness checks and their shared out-of-line helpers. This experiment cannot apportion those shared samples to individual callers. Library and anonymous executable PCs were left unresolved (about +7.0 ns/outcome). The table omits smaller categories. Positive costs are partly offset by fewer ordinary fetches and less interpreted execution. An isolated implementation comparison would be needed to estimate any achievable speedup.

The original report is under `local/workstation-jit-2026-11-19/profile-short-loop/`. The independent source, build and analysis scripts, sampled addresses, symbol tables, raw timings and all category deltas are under `local/review-workstation-jit-short-profile-2026-11-19/`. The disposable build was removed. No guest media or production source was involved.

## Implementation assignment for the other AI

Start from Helios `eb3d0bd` on `feat/workstation-jit`. Implement and test one change to register-only ALU/SETHI and Bicc spans, including eligible delay slots.

1. In `Cpu::step_native_span_with_interrupts`, retain the `native_span_time_limit()` value computed after the first live fetch. At the final register-span admission point, compare the candidate's projected elapsed time with that limit. Today `native_pair_ready()` queries the devices and deadlines again. Reuse the limit only within this call, while the intervening lookahead consists of read-only proofs and decode. Keep the word LD/ST path's own readiness check. Preserve the current clock projection, all fetch proofs, strict deadline and deferred-time boundaries, and one-outcome fallback policy.
2. Promote the retained `local/review-workstation-jit-fallback-2026-11-19/readiness_fixture.rs` predicate-equivalence test. Exercise the existing span, pair, branch-slot and word-RAM fixtures with `jit-profile,perf-counters`. Require agreement with the interpreter on outcomes, PC/nPC, time and MMU counters. A correctness failure ends the experiment before timing.
3. Compare uninstrumented release builds of `eb3d0bd` and the candidate using the existing four SS-20 loops. The short ADD/branch loop is the primary result. Run exactly three alternating baseline/candidate pairs per loop, 24,000 warm-up and 2.4 million timed outcomes per arm, eight seconds maximum per arm. Use identical compiler options and the same probe; record both binary hashes. Report per-pair ns/outcome, checksums and native call/instruction counts. Return a tested patch if the short loop improves consistently and the controls hold, or a negative result. Stop there. Heliacal changes, Solaris boots and merging are outside this experiment.

## Decision on readiness reuse

Archive the candidate and keep `eb3d0bd`. The short-loop candidate-minus-baseline differences were +5.17, −1.26 and +1.63 ns/outcome. The medians were 81.26 and 83.43 ns/outcome respectively for baseline and candidate. The mixed load and long ALU loop medians moved by less than 0.4%. The held-reset median increased by 5.0%. That control executed zero native calls in either build, so its shift cannot be attributed to removing a successful span's final check.

The retained 48 timing rows agree on steps, checksums and native execution counts within each matched workload and mode. The timing JSON omits the binary hashes requested in the assignment. The patch and rows are in `local/workstation-jit-2026-11-19/readiness-reuse-{candidate.patch,timing.json}`. The existing Solaris gate still applies.

## Next bounded assignment

Prototype a count-only result for the same single-CPU span executor at `eb3d0bd`. Heliacal's `step_machine_span` consumes only `outcome.len()`; the detailed API builds a per-instruction `Step` array. Share execution between the detailed and count-only callers. Keep enough internal information for clock advancement, first-outcome reset attribution, phase counters, and exact trap, interrupt and error behavior.

Test the prototype in Helios before changing Heliacal. In paired machines compare the count-only result with the detailed result length or exact fault, then compare machine snapshots, MMU and native counters. Cover short budgets, branch/annul, word LD/ST, deadlines, both fallback policies, resets and power-off. If these pass, measure the count-only caller against the detailed caller on the same four loops in three alternating release pairs (24,000 warm-up, 2.4 million timed outcomes, eight seconds per arm). Capture hashes and raw rows. Return one patch or a negative result and stop. Defer frontend integration and Solaris testing until that comparison shows a consistent short-loop improvement with the controls intact.
