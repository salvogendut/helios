# Workstation JIT: longer blocks and frontend integration

2026-11-19. Continue `feat/workstation-jit` in both repositories.
Starting points: Helios `83939df`, Heliacal `219278b`.

Implementation update: `787b5ca`/`60ecaad` adds the span and worker paths.
The [integration review](workstation-jit-span-integration-review.md) records
the measured warmed allocations and the still-missing phase/worker coverage.
Use that follow-up before extending opcode coverage; the requirements below
remain the original acceptance contract.

## Review result and next deliverable

The adopted core/test files match the tested translated-pair patch byte for
byte: `cpu.rs`, `mmu.rs`, `system_bus.rs`, `machine.rs`, `jit.rs` and
`tests/native_pair_prototype.rs`. This check uses the retained
`local/review-workstation-jit-pair-2026-11-19/source-hashes.json`. No new
behavioral test rerun was needed to establish identity with that validation.

The pair checkpoint is accepted as a correctness foundation. It is not an
accelerated-workstation merge candidate. The next deliverable is **one
integrated core/frontend checkpoint**: variable-length translated ALU spans,
Heliacal actually using them, and per-phase evidence from the original-OBP
SS-20 Solaris workload. Do not return only a larger core API or another plan.

## 1. Generalize the proved span to 2–16 instructions

- Preserve the initial live fetch and the adopted cached-fetch proof. Admit
  only an ordinary-DRAM, same-page, warm level-3 straight-line prefix using
  the already-supported nontrapping SETHI/basic ALUs. Stop the prefix before
  branches, memory, windows, ASIs, traps, FPU or any failed proof. Do not skip
  unsupported instructions to join separate ALU runs.
- Bound length by the remaining outcome budget, page end, supported prefix,
  actual clock phase, next device deadline and the deferred-service quantum.
  If sixteen do not fit but a shorter prefix does, use the shorter prefix.
  If no multi-instruction prefix fits, retain ordinary stepping. Do not
  flush deferred time early or change the established event schedule.
- Make one register/ICC transfer and one native call per admitted span.
  Preserve each fetch's request/counter effects and exact PC/nPC. Build and
  validate future code only through side-effect-free reads; an unsupported
  suffix must not produce a speculative architectural fetch or fault.
- Keep raw-word revalidation and architectural cached-permission semantics.
  Do not substitute the global MMU epoch or the incomplete DRAM code-write
  epoch for the proof. Keep the shared fetch/execute helpers inlined; the
  initial out-of-line refactor measurably regressed ordinary execution.
- Use a bounded cache with reusable compiled spans/prefixes and no allocation
  on a warmed hit. Consolidate with the persistent engine where practical;
  do not accumulate a separate cache for every block length or create a
  compile storm when event budgets vary. Never run a cached block past the
  remaining budget merely because its full version is already compiled.
- Return exact consumed outcomes and native work. A multi-block wrapper must
  retain completed progress when a later call faults or resets. Synchronous
  traps, interrupt entries, resets and retired instructions are distinct.

## 2. Connect the bounded runner to Heliacal

`src/emulator.rs::step_machine` currently returns only a processor ID and
discards `Step`. Its worker uses `for _ in 0..2048`, adding one to `state.steps`
and the selected processor count after every successful call. That is correct
for the old API, not for a call returning multiple outcomes.

- Select bounded native spans only for an eligible UP machine in JIT mode.
  Keep interpreter mode and both SMP CPUs on their existing paths, including
  reset/replacement. Preserve feature-off rejection and the F5 setting flow.
- Budget **2,048 outcomes**, not 2,048 block calls. Subtract the actual number
  consumed, keep processor totals accurate, and preserve power-off success,
  fault attribution, completed progress and reset-epoch accounting.
- Preserve keyboard delivery/backpressure, mouse/paste handling, serial
  BREAK, PTY/network servicing, output draining and command/abort processing.
  The current keyboard queue is retried before every outcome, and idle
  parking is checked after every outcome. Use a single-outcome fallback when
  those boundaries require it unless tests prove an equivalent batched path.
  Do not silently skip parking candidates or change its proof, 2 ms cap,
  input grace or timing policy. Measure acceleration with parking off.
- Expose phase-readable native instruction/call/span counts and fallbacks.
  Distinguish the one-word bridge from actual spans of length >=2. A bounded
  block-length histogram or equivalent aggregate makes this directly visible.
  Compilation counts/time should be separate from steady execution; avoid
  per-instruction host timers or diagnostics in ordinary builds.

## 3. Validate and measure the integrated result

Keep the fourteen pair fixtures. Add full-snapshot plus explicit MMU-counter
comparisons for lengths/budgets 1, 2, 3, 15, 16 and 17; cached-short-budget
execution; code/page edges; an unsupported suffix; shortened deadline spans;
accurate-clock fractional phase; interrupts/reset; and invalidation between
calls. Check the complete outcome sequence, not only final general registers.

Frontend tests must cover a 2,048-outcome batch ending exactly on budget,
partial progress before a fault/reset, power-off, pending keyboard/BREAK,
pause/stop, and interpreter-only SMP. Run the established core default/feature
tests, formatting, Clippy and four demos; frontend default/native tests,
formatting, Clippy, release build and relevant smoke checks.

Then use a disposable SCSI-3 disk clone and temporary config/NVRAM for original
OBP SS-20 UP Solaris. Verify login, the accepted checksummed useful-work probe,
sync and clean halt. Preserve source media and the live catalog.

- Compare runtime interpreter and JIT modes in the **same feature-enabled
  release binary**, with a feature-off control for ordinary-path overhead.
  Record actual argv and binary hashes; do not reuse a harness that hardcodes
  JIT while labeling the run interpreter.
- Use at least three alternating matched pairs with identical media/NVRAM,
  clock/display settings, workload and settled phase boundaries. Keep parking
  off. If the old probe is too short/noisy, lengthen both arms identically and
  retain the exact command and expected output.
- Report per-phase outcome/native-instruction/native-call/multi-instruction-
  span/compile/fallback deltas, block lengths, virtual time, wall time and
  host CPU time. Separate boot, useful work and idle, and reset counter epochs
  explicitly. A shutdown total from the old bridge is insufficient evidence.
- Record whether real useful work actually executes blocks longer than two.
  Keep the >=10% real-workload improvement gate and the existing ordinary-path
  regression control. The synthetic pair result is not that acceptance test.

## Return point and remaining scope

Return the integrated core/frontend implementation, promoted regressions,
exact revisions/build hashes and matched real-guest results. If the speed gate
is still missed, report actual block lengths/coverage/fallback evidence rather
than declaring the fetch contract blocked again.

Branch/delay-slot and safe integer-memory coverage remain the following part
of checkpoint 3; this bounded assignment does not remove them or close the
milestone. Their helpers must preserve precise exits, PC/nPC, annul, MMIO,
code writes and partial progress. No merge or acceleration claim until the
overall correctness and real-workload gates pass.

Retain compact scripts/logs/hashes, reconcile public docs with the selected
behavior, and remove only the temporary builds/clones created for this work.
