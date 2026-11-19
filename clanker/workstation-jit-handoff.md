# Workstation JIT: implementation handoff

2026-11-19. Execute this work order; it is not another research assignment.

## Objective and starting point

Make the native JIT selectable in Heliacal and demonstrate native execution
and a measurable benefit while running a real Solaris guest. Deliver three
reviewable implementation checkpoints, then a final integration candidate.
Do not describe a synthetic ALU result or an all-interpreter boot as completion.

Start from the merged local `main` in both repositories: Helios `fb49784`
(accepted native implementation `9ac1ed5`) and Heliacal `682d85b`, or their
descendants. Verify these commits are present before branching; do not reset
either checkout to a remote-tracking ref that may be older. Create
`feat/workstation-jit` in each repository and preserve unrelated work.

The existing native emitter is useful and already reviewed. Extend it rather
than replace it with another compiler framework. Current limitations are:

- `Machine::run_jit_native` in `src/machine.rs` creates its cache on every call,
  copies all active registers in/out, and admits only device-free,
  untranslated, single-CPU RAM execution.
- `jit-native/src/lib.rs` emits SETHI and basic integer arithmetic/logic;
  unsupported instructions fall back.
- `../heliacal/src/emulator.rs::step_machine` always selects the interpreter
  or SMP scheduler. The frontend does not enable the native Cargo feature.
- `NativeRunStats::native_instructions` currently counts instructions compiled,
  not instructions actually executed natively. It cannot establish OS coverage.

Primary target: original-OBP SS-20, one CPU, installed Solaris 2.6, first on
serial and then native SX in Heliacal. SS-5 and Axil remain required regression
targets; measure SS-5 separately. Do not reinterpret its WP=0 replacement
behavior to obtain a speedup. SMP continues through `step_scheduled`.

## Execution discipline

Reuse the retained boot, guest-work, snapshot and benchmark harnesses. Take
one fresh interpreter baseline, then implement. No open-ended profiling,
parking investigation, new ISA survey, or unrelated cleanup campaign.

At each checkpoint run focused tests, keep the result reviewable, and commit
the passing implementation with its evidence. Continue to the next checkpoint
within this scope without waiting for permission after each progress report.
Return the final candidate for review before merging. If a correctness failure
blocks progress, report its smallest reproducer and next concrete fix.

Keep native execution default-off throughout. Preserve core
`unsafe_code = "forbid"`; executable memory and native calling-boundary unsafe
code stay in the optional `helios-jit-native` crate. Keep W^X, bounded code
storage, and cleanup. Do not add raw pointers into movable Rust CPU/bus state
as a shortcut around the safe core interface.

## Checkpoint 1: reusable engine and trustworthy accounting

1. Introduce an execution-engine object that survives frontend batch calls.
   Keep it outside architectural snapshots/equality. Reset/replacement and
   mapping changes must revoke affected cached assumptions. Bound both code
   bytes and metadata; use eviction, not permanent JIT disablement after the
   first 64 compilation attempts of a real boot.
2. Preserve the interpreter path and its APIs. Define a bounded run result
   that distinguishes retired instructions, total processor outcomes, native
   executions, interpreted executions, traps, interrupt entries, resets and
   stops/errors. Partial progress before an error must remain reportable.
   Separate compiled instructions, cache hits, compilation time and fallback
   reasons. Expensive attribution belongs behind a diagnostic feature.
3. Reduce register copying with per-block input/output/ICC masks if the small
   paired benchmark confirms a benefit. Retain `Cpu::read_reg`/`write_reg`
   window semantics; registers written earlier in a block are not external
   inputs. Do not start a general register allocator in this checkpoint.
4. Stop unrelated data writes from forcing recompilation of every hot block.
   Use bounded physical-code identity plus coherent write/mapping generations,
   or validated raw instruction words where write coverage is not yet proved.
   Cover CPU stores, DMA, MXCC, host loads and mutable memory access paths,
   DSIMM mirrors, reset and mapping replacement. A new VA alias must not hide
   a write to the same physical code. Do not remove existing conservative
   invalidation until the replacement is tested.

Acceptance: all existing native review fixtures pass; repeated small run calls
reuse native blocks, exact budget suffixes remain correct, ordinary data stores
do not cause a compile storm, and code changes through each supported writer
are observed. Run the existing fixed ALU/memory pair once after correctness;
do not spend another checkpoint tuning this synthetic workload.

## Checkpoint 2: real workstation execution and early Heliacal integration

This is the main engineering task. Removing the `devices().is_none()` or
`untranslated_execution()` guards by itself is not an implementation.

1. Introduce a shared execution-boundary contract derived from `Machine::step`:
   preexisting reset/error handling, device service, interrupt sampling,
   initiator context, instruction fetch, execution, wait accounting, clock
   advancement and reset-after-store. Keep the existing interpreter as the
   reference. Native execution must stop before an event the reference would
   observe; pending DMA/SCC/IRQ work is not represented solely by a timer
   deadline. Preserve the held-SCSI-reset interrupt boundary regression.
2. Admit translated ordinary-RAM instruction execution using the existing
   SRMMU semantics. A block lookup must include or validate physical code
   identity and the relevant execution state: PC/nPC, context, privilege,
   MMU/boot configuration, permissions, and translation validity. Recheck
   after every interpreter helper that can alter those assumptions.
3. Compilation/discovery must not speculatively mutate the live MMU or bus.
   Do not fetch future instructions through `Srmmu::load` just to decode a
   block. Start with cached instructions observed during ordinary execution,
   or a proved side-effect-free inspection interface; validate before reuse.
   At execution, preserve architectural fetch effects in their original order:
   permissions, R/M changes where applicable, TLB fills/replacements,
   diagnostics, fault capture, access metadata and wait states. Read-only
   translation inspection is not a substitute for those execution effects.
4. Begin with short ALU blocks on warm translations. On a miss, changed word,
   uncertain translation, event boundary or fault, synchronize state and use
   the interpreter exactly once. A per-instruction helper implementation is
   acceptable for this correctness checkpoint; label its cost honestly.
   Coalesce fetch/service work only where equivalence is demonstrated. Preserve
   all PC/nPC and device effects before a later instruction faults.
5. Add frontend Cargo feature forwarding and a consistent runtime selector,
   proposed `--execution interpreter|jit` in Helios and Heliacal. Add the saved
   named-machine setting and a Tinker-gated Advanced execution row; default to
   interpreter. A build without the feature must clearly reject/hide native
   selection. A dual-CPU machine remains scheduled/interpreted and reports
   that fallback. Selecting JIT must never silently imply native coverage.
6. Integrate bounded runs into the worker rather than calling a fresh engine
   per instruction. Preserve pending-key delivery, PTY/network processing,
   pause/quit/reset, power-off handling, counters, and output publication.
   Do not mix UP and SMP step APIs. Preserve existing parking boundaries and
   policy; conservative interpreter fallback when their observation is needed
   is acceptable. Run JIT throughput measurements with parking disabled.

Acceptance: Solaris reaches login on the SS-20 with devices and translation
enabled and a nonzero, independently checked count of instructions executed
by native code after kernel entry. Run a real command and clean halt through
Heliacal. This is a functionality checkpoint, not yet a performance claim.
Do not stop the assignment here just because firmware rendered or login worked.

## Checkpoint 3: useful coverage, then measured acceleration

Use one bounded real-guest fallback/count sample to choose the next work. The
expected priority is control flow and integer memory access; implement these
incrementally, retaining fallback for everything not yet covered.

- Add Bicc and CALL, then the common JMPL forms if the sample justifies them.
  Preserve branch decisions, PC/nPC, delay-slot execution and annul rules,
  including budget or interrupt boundaries between branch and delay slot.
  Carry pending control flow explicitly across run calls. SAVE/RESTORE,
  RETT, privileged state writes and FPU can remain interpreted exits.
- Add ordinary integer loads/stores through audited safe SRMMU/bus helpers
  first. Prove alignment, sign extension, word-transparent lane handling,
  permissions, fault atomicity and MMIO side effects. Helpers must not expose
  raw host RAM pointers to generated code. Keep ASIs, atomics and uncertain
  targets on interpreter fallback initially. A faulting helper must exit
  immediately with prior native state committed and the fault attributed once.
- End/revalidate a block on code writes and mapping/control changes. On a
  store that can overwrite upcoming translated instructions, exit before the
  next instruction; invalidating only the next cache lookup is insufficient.
- Reduce helper/dispatch cost only after the above works. Start with register
  masks and persistent hot blocks; direct chaining, inline TLBs, new host
  backends and SMP native execution are follow-ups unless indispensable to
  the measured target. Keep any extra optimization bounded to one diagnosed
  bottleneck rather than restarting a general performance investigation.

Acceptance target: a reproducible improvement of at least 10% in either a
real Solaris useful-work phase or boot-to-login, with actual native coverage.
This is a target, not an assumed result. Always report both boot and useful
work; a useful-work improvement does not imply a faster boot. Seek no repeatable
regression above 3% in the checked interpreter/fallback workloads. If these
targets are missed after one focused optimization pass, return the working
candidate with the exact limitation and one measured follow-up; do not claim
the acceleration objective is complete or keep investigating indefinitely.

## Correctness and measurement gates

Reuse `Machine::diagnostic_snapshot()` and its deliberate-mismatch tests.
Synthetic differential runs compare identical outcome counts at every block
exit and fault/interrupt boundary, including all windows, WIM/CWP, MMU/FPU,
device state, clocks, deferred time and memory. Do not weaken the oracle to
exclude newly inconvenient differences. SCSI media contents are outside the
snapshot: validate disk data independently with checksums and clean reboot.

Required focused cases, in addition to the existing 900-case ALU matrix:

- Remaining budgets 0/1 and short cached suffixes; branch/delay/annul splits.
- Warm-cache self-modification, DMA and MXCC code writes, VA/physical aliases,
  code-page boundaries, permission loss, context changes and MMU enable/disable.
- No future-fetch PTE/fault mutation; one precise fault capture; committed
  earlier instructions and unchanged destination on a failing load.
- Interrupts, timer deadlines, wait states, reset/power-off, latched errors,
  held SCSI DMA reset, and both existing clock modes.
- Cache reset/replacement/eviction, fallback after window changes, and
  feature-off execution remaining on the accepted interpreter path.

For performance use identical release settings, fixed RTC/NVRAM/ROM seeds,
cloned source disks and matching commands. Compare the same feature-enabled
binary in interpreter/native modes, plus a feature-off baseline for added
ordinary-build overhead. Retain exact source revisions, patches if any,
binary hashes, features, compiler, runner and raw results. Use three alternating
pairs initially; repeat only for a concrete discrepancy. Separate boot,
fixed guest useful work, disk I/O, and idle. Report wall and worker CPU time,
outcomes, native-executed share, fallback reasons, cache reuse and compile cost.
Do not compare different virtual-time phases or count compilation as execution.

Run focused checks after each edit. At checkpoint completion run core default
and `jit-native` tests/Clippy, formatting, native-crate checks, and all four
demos as required by AGENTS.md. At final integration run Heliacal default and
native-feature tests/Clippy/formatting, release build and SDL smoke. Exercise
the existing performance/parking feature combinations where shared code changed.

Final real-machine checks: primary SS-20 original-OBP Solaris serial boot,
useful work and clean halt; SX Heliacal typing/capture, disk checksum, reset and
reboot persistence; SS-5 and Axil UP boot-to-shell regression; one dual-CPU
fallback smoke. A NetBSD smoke on the preserved installation is desirable
after these gates, not permission to expand into a new OS-debugging project.

## Finish and hand back

Update human docs with the actual native/fallback coverage and the runtime
selector. Put concise checkpoint evidence in one progress report linked from
`clanker/INDEX.md`; do not create a chain of measurement-only assignments.
Rebuild Heliacal against the final core and give the exact launch command.
Leave the reviewed source commits on the feature branches for final merge.

Preserve source disks, the user's NetBSD CHD, live catalog and NVRAM. New
scratch disks must include their SCSI ID in the filename. Remove only this
assignment's disposable clones, temporary worktrees/builds and tracing after
retaining compact logs, reproduction sources and hashes. Never delete another
agent's active work. Do not reopen the completed prototype milestone as an
indefinite umbrella issue; track workstation acceleration separately.

References already inspected: [SPARC JIT comparison](jit-reference-comparison.md),
[accepted native results](jit-prototype-results.md),
[architecture references](../docs/references.md),
[memory model](../docs/memory.md), and
[interrupt/timer model](../docs/interrupts-timers.md).
Use the pinned QEMU block/state/budget model and UltraViolet cache/register
organization as design references. InfiniteSPARCjit's address-width and
diagnostic lessons remain relevant. Do not copy third-party implementation
or adopt sun4u/V9 semantics for these V8/sun4m machines.
