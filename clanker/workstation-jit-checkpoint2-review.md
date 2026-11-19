# Workstation JIT: bridge review and checkpoint 3 continuation

2026-11-19. Helios `b241146`, Heliacal `dcca16d`.

Checkpoint 1's four corrections are accepted. The UP bridge is a useful
functional implementation of the assigned execution boundary, with the
SMP exclusion and frontend/evidence completion items below. It is not an
acceleration candidate. Continue the existing checkpoint 3 assignment; do
not merge or stop at another synthetic result.

## Independent checks

The seven promoted checkpoint-1 regressions pass and `jit`-only compilation
works. New bridge checks exercise a real SS-20 device configuration with
translation enabled, compare full snapshots after each supported instruction,
and verify precise permission and next-page faults. The eight earlier
fetch/fault/reset/branch-delay fixtures also pass when adapted to ordinary
steps with the bridge enabled, rather than `run_jit_native`.

Of 22 focused tests, 21 pass. The failure is the documented SMP exclusion:

- `Machine::set_native_execution_enabled(true)` enables CPU 0 even when a
  secondary CPU is installed. `step_scheduled()` then executes CPU 0's ALU
  instruction natively while CPU 2 remains interpreted. The fixture observes
  native count 1 where the agreed interpreter-only SMP scope requires zero.
- This is a scope/dispatch regression, not evidence that the one-instruction
  ALU result itself is incorrect. Keep SMP native execution out of this task
  rather than silently enlarging the acceptance scope.

An isolated tested fix at
`local/review-workstation-jit-cp2-2026-11-19/smp-exclusion.patch` makes effective
native enablement conditional on no secondary CPU and explicitly disables
the secondary bridge. All 22 tests then pass, as does `jit` compilation.
The reset paths retain this effective setting. Apply that correction or an
equivalent, promote the 13 bridge review/adapted fixtures, and add the
frontend's visible SMP fallback explanation. Production code is unchanged
by this review.

Submitted core logs contain 814 default / 831 `jit` / 862 `jit-native` passed
tests. The full suites, demos, frontend build and guest boot were not rerun
by this review; the independent scope is the focused check above.

## Complete the frontend selector while continuing

The feature forwarding and CLI/config fields exist, but the planned selector
integration is incomplete:

- A feature-off build accepts a native request and only prints a fallback
  warning when constructing the worker. Reject the explicit CLI request before
  startup, and give saved settings a clear visible unsupported-mode error or
  disabled selector. Do not silently present a requested mode as active.
- Add `--execution interpreter|jit` to help and the Tinker-gated Advanced row.
  No such row currently exists. Make actual execution mode/fallback visible.
- Include the execution setting in the existing apply/reset decision, or
  implement and test a safe explicit mode switch. Currently `hardware_eq`
  ignores it and the worker reads it only during machine construction; a
  saved setting must not imply a change to an already-running engine.
- Preserve CLI precedence, named-machine settings, default interpreter mode,
  reset/replacement behavior and the feature-off path. Keep SMP on the
  scheduled interpreter.

These are completion items within the current assignment, not a new GUI
project or a reason to pause checkpoint 3.

## Evidence qualification

Both retained Solaris logs show original OBP, login, `uname`, a correct 4950
result from the 100-iteration awk command, filesystem sync, return to `ok`
and host exit 0. The native log reports 601,669,633 instructions at shutdown;
the interpreter log reports zero. This supports a native-enabled guest run.
The single cumulative native total does not locate execution within boot,
kernel, user work or the final OBP session. On the next required guest run,
record native-count deltas at login and around useful work, with reset epoch
and virtual time, instead of inferring phase coverage from a shutdown total.

The observed 0.564991828 / 0.148830684 seconds is about 3.80x. Treat it as
one smoke comparison, not a controlled median or a component profile:

- Interpreter and native run binary hashes differ (`c7158d...` / `d164fb...`).
- The native log includes diagnostic strings absent from the committed source.
  The retained directory does not identify the corresponding diagnostic patch
  and exact build recipe. The final release hash is `c7158d...`.
- The retained runner hardcodes `--execution jit`, so it does not directly
  reproduce both runtime arms without edits. Add an execution-mode argument
  and retain the actual argv and source/build manifest for each run.
- A short awk command includes process startup and console round-trip work;
  it is not a pure arithmetic benchmark. Keep it as a responsiveness/control
  probe and add a longer fixed useful-work phase with a verified checksum.

Do not spend another full boot merely repairing these labels. Incorporate
the missing phase counters/provenance into the checkpoint-3 guest measurement.
Do not call a specific overhead the measured cause without a bounded count
or profile that distinguishes cache misses/compilation from steady dispatch.

## Next implementation pass: eliminate per-instruction native entry

The bridge still pays the ordinary interpreter fetch/device path and then
adds word-cache lookup, `Rc` handling, state initialization, register-mask
loops and a native call for each supported instruction. Heliacal still calls
`step_machine` once per outcome. The new 4096-word bridge cache is separate
from checkpoint 1's persistent multi-instruction engine. These are concrete
extra operations visible in the source; their individual cost shares have
not been measured.

Proceed in this order:

1. **Keep the bridge as a correctness/fallback path.** Build the workstation
   block path around the persistent engine instead of extending the word-at-
   a-time path with every remaining opcode. A tiny optional counter sample
   can record native entries, instructions per entry, cache misses/clears and
   compilations during the existing guest probe; no broad profiling project.
2. **Batch proved straight-line spans first.** Start with short 2–16 instruction
   spans in ordinary translated RAM, using one register/ICC transfer and one
   native entry. Reuse the existing physical-code validation and exact outcome
   accounting contract. The bare-metal engine's old admission guard cannot
   simply be removed: block construction must not touch future live MMU state,
   and execution must preserve fetch/permission/diagnostic/fault effects and
   device/interrupt observation boundaries. Use conservative fallback whenever
   this cannot be proved. Never use the absence of a timer deadline as the
   sole proof that pending device/DMA work can be skipped.
3. **Connect bounded block runs to Heliacal.** Return before budget/event/input
   observation boundaries and preserve partial progress on faults and resets.
   Keep keys, PTY, network, output publication, pause/stop and parking policy
   unchanged. Larger host batches alone are not native block acceleration.
4. **Then add branch/delay and integer memory coverage.** Retain exact PC/nPC,
   annul and split-budget behavior. Begin memory operations with safe SRMMU/bus
   helpers and precise exits on faults, MMIO and code writes. Do not bypass
   helpers or change MMU semantics to manufacture a speedup.
5. **Measure one final candidate properly.** Use the same feature-enabled
   release binary with runtime interpreter/JIT selection, plus a feature-off
   baseline for ordinary-build overhead. Start with three alternating pairs,
   parking off, identical cloned media/RTC/NVRAM and matching phases. Report
   both boot and useful work, native-executed deltas, average native block
   length, compilation cost and worker CPU/wall time. Keep the original 10%
   real-workload target and the bounded follow-up rule in the main handoff.

The next report should contain native blocks executing inside the translated
guest and the frontend batch path, or a specific failing invariant with its
minimal reproducer. Another one-instruction opcode expansion or another
statement that batching is absent does not complete this implementation pass.

## Reproduction and cleanup

```sh
python3 local/review-workstation-jit-cp2-2026-11-19/run_review.py
python3 local/review-workstation-jit-cp2-2026-11-19/run_review.py --patch
```

The runner retains fixtures, toolchain/source hashes, results and cleanup
records. It copies tracked source, changes only that owned temporary copy,
and removes its source/build directory in `finally`. Both review builds were
removed. Source media, the live catalog, NVRAM, production core and frontend
code were untouched. This review and index entry are uncommitted.

Continue under the [original implementation handoff](workstation-jit-handoff.md).

## Concrete first-block contract after fa6475c

The SMP patch and selector follow-ups have landed as Helios `345ecdb` /
Heliacal `219278b`. The latest core commit `fa6475c` adds an invariant note,
not block execution. Review of that note found the missing TLB invalidation
step described in the corrected progress report.

`local/review-workstation-jit-fetch-2026-11-19/native_fetch_contract_review.rs`
has three passing regressions, including SS-5 and SS-20 with the bridge both
off and on:

- Warm resident instruction translation + physical PTE permission edit:
  execution continues using the cached permission, with no fault.
- The same sequence plus ASI-3 page flush: the next fetch faults before native
  execution, with SFSR `0x368` and no duplicate capture. In this ET=0 fixture
  the CPU latches the precise error; instruction protection leaves SFAR
  unchanged because FAV is not set.
- Permission removed before the first fetch: immediate protection fault.

This matches the existing `tlb_retains_translation_until_mmu_flush_and_cache_flush_is_separate`
test in `tests/mmu.rs`. If the other reproduction includes a translated data
access that evicts the instruction entry (especially SS-5 WP=0), retain that
access in its test. Do not replace the MMU's cached-permission behavior with
a blanket PTE-memory-write invalidation rule.

Implement this deliberately narrow first block path. The API names below
are proposed; none is claimed to exist or be proved by these three tests.

1. **Keep the first real fetch.** Enter through the ordinary reset/error,
   device-service, interrupt and access-context boundary, then fetch the
   first instruction through `Srmmu::load`. That handles a cold mapping,
   an actual permission loss and its side effects once. A successful fetch
   is already a useful starting point for inspecting a warm translation.
   If block admission fails, execute this already-fetched word through the
   bridge/interpreter; do not fetch it again and double-record effects.
2. **Prove a small cached-hit span inside the MMU module.** Start with exactly
   two straight-line ALU/SETHI instructions, later up to 16, all within one
   4 KiB virtual page backed by ordinary installed DRAM. Require a normal
   level-3 leaf, the exact fast-instruction-TLB selection, valid permissions
   for the current supervisor/user ASI, R already set, no boot mapping,
   no relevant PTP2 override, no unsupported diagnostic state and no pending
   I/O invalidation. Reject other cases to the bridge. Read the same state
   `translate()` would use, not a fresh page-table interpretation. The
   parking lookup is a reference, not a ready-made JIT API: it assumes
   supervisor access and does not perform instruction-fetch effects.
3. **Make the proof local to this execution boundary.** Validate PC/nPC,
   effective context/privilege and relevant control/configuration fields,
   selected TLB slot and its generation, global TLB generation, physical
   backing and code identity. `execution_generation()` alone is insufficient:
   ordinary register writes do not all advance it and 4 KiB replacements use
   per-slot generations. Renew the proof after every exit/fallback. This
   does not require changing guest-visible MMU replacement or flush policy.
4. **Inspect code without executing future fetches.** Read candidate words
   through an ordinary-DRAM, side-effect-free peek and compare the compiled
   code identity. Initially validate those words on every entry rather than
   assuming the bare-metal watch scheme covers DSIMM aliases and every DRAM
   write route. In particular audit the early DRAM `load_image` path and
   mirrored storage before using epochs alone. Reject page crossings, PROM,
   MMIO, empty banks, branches, loads/stores and potentially trapping opcodes.
5. **Guard a no-event interval in Machine.** First reuse the pre-step service
   order; then require no work that another ordinary pre-step would consume,
   no pending IRQ/reset, and no PROM waits. Choose a count fitting the exact
   remaining budget and clock phase, strictly before the next device deadline.
   For the first version also keep the *whole span* below the existing
   deferred-device-time 2,000 ns application boundary, accounting for the
   already deferred amount. Thus no intermediate device tick or external
   owner can change the proof. Reject held DMA reset and uncertain service
   states. Preserve frontend input/parking observation requirements as well.
6. **Implement the cached-fetch effects, do not simply omit them.** For this
   restricted fast-hit/level-3/R-set instruction case, `touch` needs no PTE
   write, `record_diag_view` is a no-op for instructions and `fill_ptp2` only
   operates on level-1 entries. These are inspectable simplifications of the
   current code, not assumptions about arbitrary MMU accesses. A small safe
   helper must still preserve memory-request metadata/access context, actual
   word identity, feature-enabled translation counters and the resulting bus
   state. Use the full reference sequence if any additional effect is needed.
   Compare the proposed helper's resulting state with ordinary fetches before
   relying on it for admission. Do not weaken snapshot equality to hide an
   omitted latch, counter or fast-cache update.
7. **Execute once and commit the exact span.** Copy the union of required
   registers, execute one native block, commit only its outputs/ICC and the
   exact sequential PC/nPC. Advance the existing clock phase and logical/
   deferred time for exactly the retired instructions. Code/data-memory,
   privilege and mapping writes are excluded from this first span. If the
   proof cannot establish nonfaulting fetches and no intervening event,
   reject it *before* native execution and use the established path.

First completion test: on an SS-20 device machine with a warm level-3
translation, one native entry retires two instructions and matches two
ordinary steps in CPU/MMU/bus/device/time state. Extend it with the three
permission cases above, slot replacement/context/control changes, code edits,
one-instruction budgets, pending interrupts and a deferred-time boundary.
Then connect this path to the bounded frontend runner and increase block
length. Permission-generation bookkeeping alone, or another prose-only
invariant report, is not that deliverable.

This is a concrete implementation proposal requiring the stated equivalence
tests; the review has not implemented or benchmarked it. The invariant tests
can be reproduced with:

```sh
python3 local/review-workstation-jit-fetch-2026-11-19/run_review.py
```

Their runner removed its owned temporary source/build. The retained fixture,
logs, source/toolchain hashes and cleanup record are the complete new evidence;
no production behavior or source media was changed.
