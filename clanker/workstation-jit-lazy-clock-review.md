# Lazy clock projection review

Helios `e24bec0` introduces a fallback-policy and accounting regression when a caller sets `set_device_service_guards(false)`. This switch is used for reference execution. The standard Heliacal benchmark leaves the guards enabled.

## Reproduction and correction

On a single-CPU SS-20, run one translated ADD through `step_native_span` with unconditional device servicing. With interpreted fallback selected, the new early return calls `Machine::step`, which runs the native single-word bridge. With the bridge selected explicitly, the CPU runs it but the returned span reports zero native instructions. The register result and guest clock are correct in both cases.

| Single-word policy | Expected native calls / instructions / reported instructions | Observed |
|---|---|---|
| Interpret | 0 / 0 / 0 | 1 / 1 / 0 |
| Native bridge | 1 / 1 / 1 | 1 / 1 / 0 |

The two fixtures pass on `5281937` and fail on `e24bec0`. The reviewed correction limits the effective span budget to one when service guards are disabled and keeps the usual span execution and accounting path. No prospective cycles are needed at that budget. The fixtures cover budgets 1 and 16, compare the returned outcome, architectural snapshot, MMU counters and hardware clock against the interpreter, and check the native and phase counters.

From Helios at `e24bec0`, the patch and regression fixtures can be applied together.

```sh
git apply --check local/review-workstation-jit-lazy-clock-2026-11-19/reference-service-policy.patch
git apply local/review-workstation-jit-lazy-clock-2026-11-19/reference-service-policy.patch
cargo test --locked --offline --features jit-profile,perf-counters --test native_pair_prototype
```

## Timing and decision

Each workload used three alternating baseline/candidate pairs, 24,000 warm-up outcomes and 2.4 million measured outcomes per arm. Both revisions used the same release profile, toolchain and probe. Checksums and execution counts matched. These results time the committed implementations with service guards enabled.

| JIT loop | `5281937` ns/outcome | `e24bec0` ns/outcome | Pairs faster on candidate |
|---|---:|---:|---:|
| ADD, branch, delay NOP | 81.34 | 86.09 | 0/3 |
| ADD and word LD | 113.99 | 115.56 | 1/3 |
| Fourteen ADDs, branch, delay NOP | 43.77 | 44.25 | 1/3 |
| Same long loop, SCSI DMA reset held | 93.68 | 89.26 | 2/3 |

The held-reset loop had a lower median and zero native calls. The small ADD/load improvement in the submission did not reproduce. Correct the reference path and end the clock experiment here. The existing Solaris command result remains 358.2 ms with JIT versus 156.9 ms interpreted on the word-RAM checkpoint, with a 141.2 ms acceptance threshold. No Solaris run was made on the clock revision.

The raw samples, runner, original failures, corrected test logs and patch are under `local/review-workstation-jit-lazy-clock-2026-11-19/`.
