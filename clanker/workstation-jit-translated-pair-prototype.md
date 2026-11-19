# Workstation JIT: tested translated-pair starting point

2026-11-19. Based on Helios `715c428`; Heliacal remains `219278b`.

The fetch-contract corrections are sound. The outstanding implementation is
no longer just a prose assignment: an isolated prototype now executes two
translated ALU instructions in one native invocation on the full SS-5,
SS-20 and Axil device machines. It preserves the complete diagnostic snapshot
and the separate MMU performance counters against two interpreter steps.

This prototype has not been applied to either production worktree, committed,
or merged. The ordinary Heliacal path still uses the one-instruction bridge.
This is a correctness starting point for checkpoint 3, not a Solaris speedup
or an accepted acceleration release.

## Patch and reproduction

Retained under `local/review-workstation-jit-pair-2026-11-19/`:

- `translated-pair.patch`: core prototype plus the fourteen focused fixtures.
- `native_pair_prototype.rs`: standalone copy of those fixtures.
- `run_prototype.py`: reconstructs the base in a disposable directory, applies
  the patch, runs validation, and removes its scratch source/build output.
- `native_pair_probe.rs`: bounded, checksummed translated SS-20 ALU workload.
- Validation logs, build commands, source/binary hashes and measurements.

For adoption on the existing feature branch, first inspect and apply the patch:

```sh
git apply --check local/review-workstation-jit-pair-2026-11-19/translated-pair.patch
git apply local/review-workstation-jit-pair-2026-11-19/translated-pair.patch
cargo test --locked --offline --features jit-native,perf-counters --test native_pair_prototype
```

The default build does not expose the new API. With `jit-native`,
`Machine::step_native_pair(NonZeroUsize)` returns one normal outcome, or two
exact `Step::Executed` records. A budget of one cannot execute the second
instruction, even with a compiled pair already cached. The result includes
the native-instruction delta; `native_execution_calls()` counts actual calls
through the ordinary bridge/pair path. Both native instructions and calls
reset with the CPU, so measurements must respect reset epochs.

## Narrow proof and effect contract

1. The ordinary pre-step service, interrupt check and **first live fetch**
   remain. A fault, reset, interrupt or unsupported first instruction retains
   its normal outcome and side effects. Fallback executes the already-fetched
   word; it does not fetch it again.
2. Both PCs must be sequential and in the same 4 KiB virtual page. The proof
   follows the current fast-ITLB slot, global/slot generations, context and
   privilege. It requires enabled translation, disabled boot mode, a normal
   level-3 entry, execute permission, an already-set referenced bit, and
   installed ordinary DRAM. Diagnostic-origin entries, relevant PTP2 state,
   large mappings, empty banks, MMIO, video and firmware fail admission.
3. Code words are checked through side-effect-free DRAM peeks **on every
   entry**. Compiled pairs are keyed by both raw words, not a stale virtual
   address or an incomplete DRAM code-write epoch. This also covers host
   `load_image` writes and backing-store mirrors without adding invalidation
   semantics. There is no blanket PTE-write invalidation: cached permission
   remains authoritative until the existing MMU rules invalidate it.
4. The first live load drains integrated I/O flushes. On the proved second
   fast instruction hit, `touch`, `record_diag_view` and `fill_ptp2` have no
   effects. `complete_native_fetch` still reproduces the memory-request latch
   and instruction/fast-hit counters. Its proof is ephemeral and consumed
   within the same call; no CPU, MMU, device, memory or time mutation may
   intervene. Native compilation touches only host code/cache storage.
5. Only existing nontrapping SETHI/basic integer ALUs may form the pair. There
   is one register/ICC transfer and one native call. Those operations cannot
   alter any second-fetch dependency, so its proved request effects commute
   with the first ALU. The bus is single-owner and SMP remains excluded.
6. Skipped pre-step work must be a no-op. Guarded service is required; pending
   work, interrupts, held DMA reset, reset requests or PROM waits reject the
   pair. Both clock cycles must fit strictly before the next known device
   deadline **and** the 2 us deferred-device-service boundary. Clock phase,
   logical time and deferred time match ordinary stepping. No parking policy
   or host wait is changed.

The proof follows existing `Srmmu::translate/load`, `SystemBus` DRAM/device
dispatch and the normal instruction boundary. The hardware/MMU source basis
is unchanged from `docs/references.md` and `docs/memory.md`; this is not a new
interpretation of PTE coherence or the SS-5 replacement policy.

## Coverage

Fourteen focused tests exercise cold and warm pairs, dependent registers/ICC,
all three profiles and both clock policies, full snapshots and MMU counters,
actual native-call counts, cached budget truncation, warmed PTE permission
edits, precise post-flush and cold protection faults, latched-error replay,
next-page faults, remapping, host image and byte code writes, unsupported
second instructions, branch-delay fallback, deferred time, earlier timer
deadlines, serial interrupt priority, held DMA reset, software reset and
unconditional-device-service fallback.

Final inlined-patch validation passes: **814 default / 892 native / 895
native-plus-perf-counter test executions**, zero failures; formatting,
Clippy with `-D warnings` for default, `jit`-only and native-plus-counters,
and all four demos. The retained reconstruction script independently
reapplied the final patch to clean `715c428`, verified source hashes and
passed all fourteen focused tests. Its scratch tree was removed.

The first full-suite attempt hit the sandbox's loopback-socket restriction;
the complete runs passed with that restriction lifted. A later native link
attempt exceeded the temporary build quota; its owned output was removed
and the final suites used debug symbols off and two build/test jobs. These
environment failures and the successful reruns are retained in the logs.
`summary.json`, `validation-final.json` and `clippy-jit-final.log` record
the final result. Both production source worktrees remain unchanged.

## Bounded performance result

The final probe uses one `jit-native` release binary with runtime interpreter,
bridge and pair modes. Six alternating-order samples per mode each retire
10 million instructions in a mapped SS-20 loop: fourteen ADDs, a branch and
its delay NOP, full board devices, 16 MiB RAM, no display, fast clock. Every
sample asserts the result checksum, outcome count and virtual time; any
non-retirement event fails the probe.

| Mode | Median ns/instruction |
| --- | ---: |
| Interpreter | 55.690 |
| One-instruction native bridge | 83.479 |
| Two-instruction native prototype | 71.050 |

Pair execution is **14.9% cheaper than the bridge but 27.6% more expensive
than the interpreter** in this synthetic workload. It is not an acceleration
claim. The pair run retires 8 million instructions in 4 million pair calls;
remaining branches/delay slots use the existing single-step path. Larger
proved blocks are still needed to amortize entry, state transfer and proof
work; their performance is not established by this result.

Five alternating baseline/prototype control pairs run the existing bare-metal
fixed-work harness with runtime native execution disabled. Both binaries have
`jit-native` compiled in and otherwise use identical release settings:

| Ordinary workload | `715c428` | Prototype | Difference |
| --- | ---: | ---: | ---: |
| ALU | 42.37 ns/step | 40.81 ns/step | -3.7% |
| Memory | 55.86 ns/step | 54.25 ns/step | -2.9% |

No ordinary-mode regression is observed in this control. This is not a
feature-off performance control or a guest boot result. The first refactor
version had a 7.8% ALU slowdown; forcing inlining of the private shared
fetch/execute helpers removed that observed regression. Initial results and
patch are retained separately rather than overwritten. Final raw output,
binary hashes and medians are in `measurements-inlined/`; the initial
refactor measurements are in `measurements/`.

No original-OBP/Solaris boot or Heliacal run was performed for this prototype.
The previous guest's native count does not validate the new pair path. The
frontend integration and original real-workload acceptance gate remain open.

## Next implementation assignment

Keep working on `feat/workstation-jit`; do not make another documentation-only
checkpoint or turn the corrected PTE contract back into an implementation
blocker.

1. Review and adopt this bounded working path and fixtures. It deliberately
   uses a small separate pair cache to isolate the correctness question;
   consolidate that into the persistent bounded engine when generalizing.
2. Extend proved spans from two to at most sixteen instructions, retaining
   all per-fetch effects, remaining-budget limits and event boundaries. A
   failed proof must leave future instruction state untouched. Do not replace
   the live first fetch or rely solely on the MMU global epoch.
3. Add the Heliacal bounded UP runner. Account for every returned outcome and
   native call, retain partial progress on later faults/resets, and return to
   the existing command/input/network/PTY/presentation boundary. Keep SMP
   interpreted and interpreter selection/default-off behavior intact.
4. Then complete the already-assigned branch/delay and safe integer-memory
   paths. Use precise helper exits; do not bypass SRMMU or device semantics.
5. Measure the original-OBP SS-20 Solaris useful-work phase with per-phase
   native instruction/call/compile/fallback deltas. Use the same release
   binary with runtime mode selection, three alternating matched pairs,
   disposable disks and parking off. The real-workload 10% gate is unchanged.

Do not claim checkpoint 3 is complete from this two-instruction synthetic
probe. The frontend connection, wider/branch/memory execution and real-guest
acceleration are still outstanding. Update public execution documentation
when the path becomes selectable, retain compact evidence and remove only
the scratch builds/clones created for that work.
