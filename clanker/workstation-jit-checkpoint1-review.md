# Workstation JIT checkpoint 1: review and continuation

2026-11-19. Reviewed `c22e670` on `feat/workstation-jit`.

Checkpoint 1 needs four corrections before acceptance. The reported
checkpoint-2 blocker is the implementation assigned by the original handoff,
not an external dependency or a missing user decision. Fix these bounded
regressions, then continue into the translated/device execution path.

## Findings

1. **P1: doubleword and atomic writes leave stale native code.**
   `SystemBus::write_doubleword`, `swap32` and `ldstub8` advance the old
   mutation epoch but never notify the new code-write epoch. Native cache hits
   no longer re-read the words. After warming `add %g1, 1, %g1`, replacing it
   through a doubleword store or SWAP with `add %g1, 9, %g1` still executes
   the old add-one instruction. LDSTUB on its last byte likewise produces 1
   instead of the new immediate 255. These paths are available now, without
   enabling workstation JIT. See `src/system_bus.rs:2409`, `:2498`, `:2529`.

2. **P1: watch-table rollover loses protection for surviving blocks.**
   `watch_code` clears its 512 ranges and advances the epoch. An existing
   engine entry can then validate unchanged raw words under the new epoch,
   but registration occurs only when compiling a new entry. The surviving
   block becomes current without a watch. Keeping PC 0 hot while visiting
   512 other blocks, then changing its instruction with an ordinary word
   store, reproduces execution of add-one instead of add-nine. Re-register
   the inspected code span during validation too, and use the epoch *after*
   registration, since registration itself can cause rollover.
   See `src/system_bus.rs:1202`, `src/machine.rs:2204`.

3. **P2: the retained `jit`-only feature no longer builds.**
   `impl NativeEngine` lacks the `jit-native` cfg used by its type and
   dependencies. `cargo check --locked --offline --features jit` produces
   unresolved-type/dependency errors. Gate the implementation consistently.
   See `src/jit.rs:166`.

4. **P2: run results mix cumulative and per-call counters.**
   `executed`, `native_instructions` and compilation values describe one call,
   but `cache_hits`/`evictions` are assigned engine lifetime totals. An empty
   run after warm-up reports cache activity despite doing no work. Return
   fieldwise per-call deltas, or expose lifetime snapshots separately with
   explicit units. See `src/machine.rs:2289`.

The selective-register fixture, including a read-after-write dependency,
attempted `%g0` write and ICC update, passes. The ordinary word-store
invalidation control also passes. The candidate passes 2/7 independent
fixtures; the other five reproduce findings 1, 2 and 4. Finding 3 is a
separate compile check.

## Tested correction and scope

The isolated correction is retained at:

`local/review-workstation-jit-cp1-2026-11-19/checkpoint1-corrections.patch`

It adds the missing three write notifications, restores watches on validation,
refreshes the epoch after registration, gates the native implementation, and
returns hit/eviction deltas. No production file was patched by this review.

With that patch, an isolated source copy passes:

- All seven new fixtures.
- All 37 previous focused JIT/native/engine fixtures, including the 900-case
  ALU/ICC matrix: **44 focused tests total**.
- `jit`-only compilation, formatting and native-feature all-target Clippy
  with `-D warnings`.

The retained submitted logs contain 814 default / 853 native passing tests;
this review did not rerun the full suites or performance benchmark. The fixed
ALU log records 41.61 versus 26.62 ns/outcome, approximately 1.56x, from one
pair. It supports the reported synthetic improvement, not a real-guest result
or a post-correction performance claim.

Promote `native_checkpoint1_review.rs` into tracked tests, apply the patch or
equivalent, run final checkpoint validation in default, `jit`, and `jit-native`
configurations, then commit the corrections. No new performance study is
needed to resolve these correctness findings.

## Continue checkpoint 2 immediately afterward

Use the existing interpreter boundary to build the first working path:

1. Share the existing `Machine::step` / `Cpu::step_with_interrupts` sequence
   between interpreter and native dispatch. Keep reset/error handling, device
   service, IRQ sampling, initiator scope, actual SRMMU fetch, wait accounting
   and virtual-time advancement in their established order. Do not simply
   delete the old device/MMU admission guards from the bare-metal block runner.
2. Initially substitute native execution for a supported **already-fetched
   current instruction**. A one-instruction cached native operation inside
   that ordinary boundary is an acceptable implementation bridge. Cache keys
   must validate the current raw word and execution assumptions. Compilation
   must not perform live speculative fetches of subsequent guest instructions.
   Keep all other operations on the existing interpreter. This bridge is for
   correctness and real-guest coverage; it is not expected to meet the speed
   target before batching is introduced.
3. Return bounded run results with exact outcome counts and partial progress
   even when execution stops on a fault. The current `Result<NativeRunStats,
   SchedulerError>` loses all local stats on `Err`, and `fallback_steps`
   includes non-retiring events. Complete the outcome/error contract before
   connecting it to frontend step counters; distinguish instructions, traps,
   interrupt entry and resets rather than counting them all as retirement.
4. Add the Heliacal feature forwarding and execution selector, with the worker
   using the persistent engine and bounded batches. Preserve host input/output,
   key delivery, lifecycle and parking observation boundaries. SMP uses its
   existing scheduler. Do not change parking policy or SS-5 WP semantics.
5. Boot the original-OBP, single-CPU SS-20 Solaris guest through this path.
   Demonstrate actual native execution **after kernel entry**, run a shell
   command, and halt cleanly. Compare the focused instruction-level snapshots
   and event/fault fixtures before broadening native blocks.
6. Continue into checkpoint 3: amortize the bridge overhead using proved block
   boundaries, then branch/delay-slot and integer memory coverage. Measure
   real boot/useful work with parking disabled. Follow the original performance
   target and stop rule; do not stop merely because the bridge reached login.

Keep these as steps within the existing three-checkpoint assignment, rather
than starting another sequence of measurement-only handoffs. The next report
should contain the corrected checkpoint and implementation progress on the
shared execution boundary, not repeat that the boundary is absent.

## Reproduction and cleanup

Run from the Helios root:

```sh
python3 local/review-workstation-jit-cp1-2026-11-19/run_review.py
python3 local/review-workstation-jit-cp1-2026-11-19/run_review.py --patch
```

The runner copies tracked source into an owned temporary directory, applies
the patch only there when requested, adds the fixtures, records logs/source
hashes/toolchain/results, and removes the copy/build in `finally`. Original
and corrected logs plus cleanup records are retained in that evidence
directory. No guest disks, live catalog, NVRAM, Heliacal source, or production
core files were modified. Only this review note and the index are uncommitted.

Original assignment: [workstation JIT handoff](workstation-jit-handoff.md).
