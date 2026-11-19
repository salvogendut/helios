# Workstation JIT span overhead review

2026-11-19. Helios `dd95120`, Heliacal `60ecaad`.

The core allocation correction passes independent checks. The current pair of
commits does not build with Heliacal's `jit-native` feature because its span
consumer still uses the old outcome API. This review supplies a tested minimal
patch. The acceleration gate remains open; no new Solaris timing is claimed.

Heliacal subsequently adopted the tested change at `6bd8e80`. The
[single-instruction fallback experiment](workstation-jit-fallback-comparison.md)
records the measured follow-up and current assignment. The build failure below
applies to `60ecaad`.

## Required integration correction

`../heliacal/src/emulator.rs:573` accesses `outcome.steps.len()`, but Helios now
makes `NativeSpanStep.steps` private. A full frontend check with
`--locked --offline --all-targets --features jit-native` fails with E0616 for
both the application and test targets.

Use `outcome.len()` instead. The patch is retained at:

`local/review-workstation-jit-span-corrections-2026-11-19/heliacal-outcome-api.patch`

Applied only in a disposable copy, this patch makes the same check pass, and
all **121 Heliacal tests with `jit-native` pass**. Apply it to the frontend
branch and rebuild before testing the new core. The user's existing frontend
binary was not replaced by this review.

## Core verification

The retained independent fixtures were adapted only to the new collection
API. Their assertions are unchanged, and their promoted versions differ only
in formatting. All **23 focused tests** pass with `jit-native,perf-counters`:
14 pair fixtures, three submitted span tests, and six independent span tests.
These compare exact outcomes, architectural state and explicit MMU counters,
including shorter budgets, clock phase, page-edge faults and code writes.

After 32 warm-up calls, the allocator probe executed 2,000 calls per case:

| Path | Outcomes | Allocations | Allocated bytes |
| --- | ---: | ---: | ---: |
| Interpreter | 2,000 | 0 | 0 |
| One-word native bridge | 2,000 | 0 | 0 |
| Sixteen-word spans | 32,000 | 0 | 0 |
| Budget-one span calls | 2,000 | 0 | 0 |
| Branch fallback | 2,000 | 0 | 0 |
| Held SCSI-DMA-reset fallback | 2,000 | 0 | 0 |

The former four allocations per warmed span/common fallback and one allocation
per budget-one call are gone. This measures warmed dispatch, not compilation
misses or real-guest throughput. Source inspection also confirms that the
device/time-fitting limit is found before suffix proofs are collected.

## Next delivery for the implementation agent

Complete the pending dispatch/evidence work as one integrated delivery:

1. Apply the frontend API patch. Add tests exercising the actual worker batch
   path: exactly 2,048 outcomes, final short budget, retained progress before
   faults/reset/power-off, keyboard/BREAK boundaries, parking exclusion and
   interpreter-only SMP. Existing frontend tests passing does not supply this
   new coverage. Correct or rename the old core suffix test, which still puts
   its branch outside the requested span; its old reference also enables the
   native bridge. The promoted independent tests already cover these cases
   with a native-disabled oracle.
2. Add reset-aware phase snapshots/deltas, a native-call length histogram,
   compile/cache-miss/eviction counts and interpreted-fallback counts. Distinguish
   retired instructions from traps, interrupts and resets. Record the reset
   epoch so a phase cannot silently subtract counters from different epochs.
   Retain mode, actual argv, binary/source hashes and phase boundaries in the
   benchmark output. Shutdown totals are not useful-work deltas.
3. Measure an interpreted fallback for isolated instructions against the
   one-word bridge, keeping native multi-instruction spans. Use the instruction
   already fetched; do not fetch twice, toggle global enablement, clear caches
   per step or relax the mapped-fetch contract. Prove equivalent outcomes,
   state, MMU effects and timing. Choose the policy from measured costs.
   The earlier cumulative run had 95.95% single-instruction native calls;
   measure the current useful-work distribution rather than assuming it.
4. First run warmed fixed-work controls, including ordinary interpreter mode.
   Then run three alternating matched original-OBP Solaris pairs using the
   same feature-enabled binary, explicit execution mode, parking off and
   disposable disk clones. Report useful-work wall/CPU time with phase-native
   deltas and histograms. Preserve source media and remove scratch builds/clones.

Branch/delay-slot and integer-memory coverage remain required checkpoint-3 work.
Use the measured stopping/fallback distribution to choose the next execution
paths. These branches remain unmerged until correctness and the original
10% real-useful-work improvement gate pass. Default-off operation, SMP exclusion
and parking policy are unchanged.

## Evidence and cleanup

`local/review-workstation-jit-span-corrections-2026-11-19/` retains exact adapted
fixtures, allocator source/manifest/lockfile, build commands, commit/toolchain
provenance, the minimal frontend patch, logs and a reproduction runner.

- `run-20261119T141210-2/`: 23 core checks and all six allocation cases pass;
  the first frontend attempt was blocked by sandboxed Podman runtime metadata.
- `run-20261119T141342-2335217/`: existing distrobox build reproduces E0616;
  the isolated one-line patch passes compilation and 121 frontend tests.
- Both temporary source/build trees were removed; cleanup manifests confirm it.

No production source, guest media or live configuration was changed. Full core
suites, Clippy, demos and a guest boot were not rerun for this narrow review.
