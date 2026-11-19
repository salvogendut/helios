# Workstation JIT progress

Branch: `feat/workstation-jit` in Helios and Heliacal.

## Checkpoint 1: reusable engine and accounting

Implemented:

- A persistent, bounded `NativeEngine` owned by `Machine`, excluded from
  architectural snapshots and cleared on reset.
- Cache entries store validated raw instruction words, a code generation and
  bound metadata (256 entries, 16 instructions/block); no permanent compile
  stop. Hit path avoids refetching/compiling.
- Code-write watch ranges and a separate code epoch; ordinary data stores no
  longer invalidate hot native code. Host image loads and mapping changes
  advance the code epoch.
- Register input/output masks and ICC-only writeback replace the full 32-register
  copy. ICC/window semantics remain through `Cpu::read_reg`/`write_reg`.
- `NativeRunStats` distinguishes retired instructions, native executions,
  interpreted fallback instructions, compiled instructions, cache hits,
  evictions and compilation cost. Reset/error/partial boundaries retain the
  existing fallback behavior.

Focused native tests include persistent-cache reuse, correct native counters,
ordinary data writes not forcing recompilation and code writes forcing a new
translation. Fixed-work pair retained ~1.5x ALU and no memory compile storm.
The full native review matrix still passes.

## Historical checkpoint 1 stopping point (superseded by the bridge below)

The native path still admits only device-free, untranslated RAM. Enabling it
for the SS-20 Solaris guest requires the shared translated/device execution
boundary (architectural fetch effects, interrupts/device deadlines, SRMMU
identity) and a frontend batch driver; that work is not implemented here.
The existing frontend remains interpreter-only. Therefore there is no nonzero
native count during a real Solaris boot and Checkpoint 3's real-workload
10% target is not met. Synthetic fixed ALU is approximately 1.5x with the
persistent engine; the memory compile storm is resolved. Do not merge as
workstation acceleration.

## Checkpoint 2: native execution inside Solaris

The frontend now forwards `jit-native` and exposes `execution_jit = true` /
`--execution jit` (default interpreter). The core native bridge substitutes a
cached one-instruction native SETHI/ALU operation for the **already-fetched**
instruction inside the normal `Machine::step` / `Cpu::step_with_interrupts`
boundary. Device service, IRQ sampling, SRMMU fetch/faults, wait accounting,
reset/error handling and clock advancement stay in their established order.
All unsupported instructions, branches, memory, windows, traps, FPU and SMP
remain interpreted.

An original-OBP single-CPU SS-20 Solaris 2.6 boot through Heliacal with
`--execution jit` reached login, ran the bounded command/fixed-work and echo
probes, synced and halted with exit 0. The shutdown report counted
**601,669,633** native instructions, so execution continued natively after
firmware and kernel entry. This is functionality evidence, not acceleration:
the one-instruction bridge has high per-step overhead, and the fixed 100-loop
probe took 0.565 s. Checkpoint 3 block batching/branch/memory coverage and the
10% real-workload target remain unimplemented. Parking measurements were run
with parking off.

### Real-execution measurement and stop rule

Matching bounded 100-iteration fixed-work probes with parking off:

- interpreter bridge disabled: 0.1488 s
- `--execution jit` one-instruction bridge: 0.5650 s

The bridge is roughly 3.8x slower and therefore misses the 10% target. The
measured bottleneck is per-instruction engine/cache/call overhead, not guest
coverage. The next focused pass is the assigned block-batching step: execute
proven straight-line spans through the shared fetch/device boundary, then add
branch/delay and integer memory coverage. Checkpoint 3 remains open; do not
merge this branch as accelerated workstation execution.

## Checkpoint 2 review corrections and checkpoint 3 status

- Native enablement is now interpreter-only on any dual-CPU machine; CPU 0 and
  CPU 2 both remain interpreted and reset/replacement preserve that scope.
- Heliacal now rejects explicit native selection in feature-off builds, exposes
  the execution row when Tinker is enabled in Advanced, and retains interpreter
  as the default. Help documents `--execution interpreter|jit`.
- The promoted bridge fixtures compare translated SS-20 state, permission and
  next-page faults, resets, dual-CPU scope and the prior fetch/fault cases.

Block batching is still absent. Heliacal continues calling one ordinary step at
a time, so checkpoint 3's branch/delay-slot and integer-memory coverage, native
block length/counter evidence, and the 10% real-workload target are not met.
The measured one-instruction bridge remains 3.8x slower on the bounded useful
work probe. The branch must not be merged as accelerated workstation JIT.

### Block fetch contract: corrected reproduction and next implementation

Independent review reproduced the proposed PTE edit on SS-5 and SS-20, in
interpreter and native-bridge modes. After warming VA `0x1234_5000` with PTE
`0x088e`, writing `0x0882` to the physical leaf **without invalidating or
replacing the resident translation still executes the ADD**. An ASI-3 page
flush makes the next fetch raise one InstructionMmu/Protection fault. A cold
first fetch with `0x0882` also faults. The earlier statement that the warmed
PTE edit alone faults was incomplete; a different result needs its actual
flush, replacement, diagnostic-edit or other access sequence retained.

The JIT must preserve cached translation permissions as well as subsequent
invalidation. A PTE-memory edit is not permission to change architectural TLB
semantics. Three independent regressions cover these cases on both profiles
and both execution modes; see
`local/review-workstation-jit-fetch-2026-11-19/`.

The next implementation is the narrow warm, same-page fetch contract in the
[checkpoint 2 review continuation](workstation-jit-checkpoint2-review.md#concrete-first-block-contract-after-fa6475c).
It retains the initial architectural fetch, proves the remaining cached-hit
effects, and admits a short native span only under explicit memory and event
guards. This is proposed work, not an implemented block path or a speed claim.
