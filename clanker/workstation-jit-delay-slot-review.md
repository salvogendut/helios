# Native Bicc delay-slot handoff

Start at Helios `67aed18` and Heliacal `b33eb55`. The four core/backend
and regression files adopted in `84ad13d` and `67aed18` match the
reviewed terminal-exit patch byte for byte. The previous three-outcome
fixture still returned two outcomes on that checkout.

## Apply the fusion patch

From the Helios feature checkout, run

```sh
ev="$PWD/local/review-workstation-jit-slot-fusion-2026-11-19"
git apply --check "$ev/delay-slot-fusion.patch"
git apply "$ev/delay-slot-fusion.patch"
cargo test --locked --offline --features jit-profile,perf-counters --test bicc_exit --test native_bicc_backend --test native_span_review
```

The isolated implementation executes an ALU/SETHI prefix, its Bicc and
one eligible same-page delay instruction in one native call. The generated
code saves the branch exit before executing the slot. An ICC-writing slot
therefore sees the original incoming nPC and leaves its own ICC for the
next guest instruction. The ordinary block compiler and terminal-only span
compiler retain their existing contracts.

Conditional annul has a second effect beyond selecting PC/nPC. A skipped
slot must preserve any register it would have overwritten. The backend
loads that register into the native state when its preservation can be
required by conditional writeback. Only an executed slot contributes a
fetch completion, an outcome, a clock step and a native histogram entry
at the longer length. BA,a and BN,a end the span at the branch. Budgets,
deadlines, an unsupported slot or the end of the proved code page can
also end it there.

The focused tests compare guest outcomes, full snapshots and, when
enabled, MMU counters. They exercise pre-slot flags, conditional writeback,
branch targets coincident with the slot, changed slot code, cache reuse with
a smaller budget, the 16-word limit, a deferred device-service boundary,
page edges, traps and interrupt entry. Removing conditional destination
preservation in a disposable copy made the CPU-state comparison fail.

## Solaris and validation

One diagnostic release booted original Sun OBP and Solaris 2.6 on a
single-CPU SS-20. The fixed shell command printed 4950, the guest synced
and halted to OBP, and Heliacal exited with status 0. Firmware, NVRAM
seed and source-disk hashes were preserved. The runner removed its SCSI-3
clone.

| Diagnostic phase | Retired instructions | Retired natively | Native share |
| --- | ---: | ---: | ---: |
| OBP boot command to login | 949,507,350 | 121,938,857 | 12.84% |
| Fixed guest command | 1,856,650 | 218,658 | 11.78% |

Both phase totals reconcile with worker outcomes, including synchronous
traps and interrupt entries. These are aggregate native counts across
the enabled execution paths. The three-pair performance comparison is
still due after RAM instruction coverage.

The isolated patch passed 814 default and 933 combined
`jit-profile,perf-counters` core test executions. The 27 focused cases,
native crate tests, formatting, Clippy with warnings denied and all four
demos passed. Heliacal `b33eb55` passed all 126 profiling tests linked
against the candidate core. Raw results, patches, serial transcript and
binary hashes are in the evidence directory above.

## Next work for the other AI

1. Adopt the tested fusion patch. It already includes the earlier
   three-outcome regression. Then apply `$ev/next-word-ram-tests.patch`
   and run `cargo test --locked --offline --features jit-profile,perf-counters --test native_word_ram`.
   The architectural reference and precise misaligned-load control pass.
   The two native coverage cases currently return one interpreted outcome
   when asked for the address calculation and memory operation.
2. Implement ordinary aligned word LD through a proved RAM path, then ST.
   The load must use the effective address computed by the preceding ALU
   operation. The store fixture writes through a virtual alias to an
   already cached next instruction and checks the replacement executes.
   Start with a terminal data access. Preserve translation permissions,
   R/M bits, byte order, bus request effects and precise faults. Keep
   device accesses on the existing MMIO path. Add direct backend coverage
   showing the actual data access, alongside snapshot and native-count
   assertions. Extend regressions to cold/remapped pages, protected stores
   and MMIO fallback.
3. Run the three alternating same-binary Solaris pairs when both native
   RAM cases pass. Use the uninstrumented release for timing and a separate
   diagnostic run for coverage. Keep the interpreter default and both
   branches unmerged pending the agreed 10% useful-work speedup.
   [Matched runner and baseline](workstation-jit-phase-review.md)
