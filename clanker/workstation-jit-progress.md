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

The bridge is roughly 3.8x slower and therefore misses the 10% target. Native
dispatch costs and guest coverage are candidates; their individual runtime
contributions were not measured in that run. The next focused pass is the
assigned block-batching step: execute
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

## Translated two-instruction prototype

Adopted the tested translated-pair path on `feat/workstation-jit`: the first
live fetch remains; a proved warm level-3 4 KiB same-page second instruction is
executed in one native call. Raw words, permissions, contexts, slot/global
generations, device deadlines, deferred time, reset/error and interrupt gates
are checked. Fourteen focused fixtures pass on SS-5, SS-20 and Axil.

The pair is a correctness starting point, not acceleration: it is about 15%
cheaper than the one-instruction bridge but still ~28% slower than the
interpreter synthetically. Span extension to 16, Heliacal batch integration,
branch/delay and memory coverage, real-guest phase counters and the 10% gate
remain open.

### Adoption review and next integrated checkpoint

Independent hash comparison at `83939df` confirms that all six adopted
source/test files exactly match the previously tested patch. Heliacal remains
at `219278b`. The next assignment combines longer proved spans, frontend
outcome-budget integration and real-guest phase measurements; see
[the block integration work order](workstation-jit-block-integration-handoff.md).

## Bounded translated spans and frontend integration

The translated path now generalizes the accepted pair to 2–16 straight-line
supported ALUs/SETHI on the same warm 4 KiB level-3 page. Admission preserves
the first live fetch and validates the warmed fast-ITLB slot, permissions,
context, slot/global generations, raw DRAM words, clock phase, deferred time
and next device deadline. One register/ICC transfer and one native call execute
the admitted span.

Heliacal selects spans only for an enabled UP machine with parking off and no
pending keyboard input. Its worker budget is 2,048 outcomes; spans subtract
actual consumed outcomes and keep processor totals. Both SMP CPUs and
interpreter mode remain on the existing paths.

Focused span tests cover lengths 2/3/15/16, one-instruction bridge fallback and
unsupported suffixes. A real original-OBP SS-20 Solaris run reached login,
executed the useful-work probe, synced and halted cleanly with **579,432,680**
native instructions, **545,483,922** native calls, and **22,115,529** spans of
length >=2. This demonstrates multi-instruction native execution inside the
guest, but not acceleration: the bounded useful-work probe was 0.617 s versus
0.154 s interpreter on the same feature-enabled binary. Branch/delay and
memory coverage plus the 10% gate remain open; branches stay unmerged.

### Independent span-integration review

At `787b5ca`/`60ecaad`, 23 focused tests pass, including six new native-disabled
oracle cases. A warmed allocation probe measures four heap allocations per
16-instruction span and per branch/held-reset fallback. The shutdown totals
imply 95.95% single-instruction calls and a mean multi-span length of 2.535;
these are cumulative run totals, not useful-work deltas. New frontend worker
regressions and phase counters are still missing. See the
[review and bounded follow-up](workstation-jit-span-integration-review.md)
before extending opcode coverage. Production behavior was not changed by
this review.


## Span overhead corrections

The span path now uses fixed-capacity arrays for cycle times, words and fetch
proofs, and an inline `NativeSpanStep` outcome record; warm spans, one-word
fallback, and budget-one calls no longer allocate heap storage. Span proof
lookahead is capped by the largest device/deferred-time-fitting length before
examining words, and an unsupported first opcode falls directly to the
already-fetched interpreter/bridge path. Six independent warmed-span fixtures
cover budgets 1/2/3/15/16/17, unsupported interior suffixes, page-edge faults,
late code writes, deferred-time shortening, exact outcomes and MMU counters.

Independent review at `dd95120` confirms 23 focused checks pass and all six
warmed allocator cases report zero allocations. Heliacal `60ecaad` still uses
the removed public `steps` field and fails its native build. The isolated fix
to `outcome.len()` passes the frontend build and 121 native-feature tests;
see the [review, tested patch and next delivery](workstation-jit-span-overhead-review.md).
No new guest timing or acceleration acceptance is claimed.

## Phase counters and matched Solaris result

Applied the reviewed reset-policy, core-phase and frontend-phase patches.
CPU-0 reset paths now retain both native and interpreted-fallback policies.
The opt-in `jit-profile` feature exports reset-epoch snapshots, checked deltas,
1–16 native-call histograms, interpreted/fallback retirement categories,
control-flow/memory subsets, trap/interrupt/error counts, cache hits/misses/
compiles/evictions and compilation time. The frontend worker answers phase
requests at batch boundaries; five worker-budget/lifecycle regressions cover
the 2,048-outcome boundary, pause, keyboard backpressure, BREAK, parking, SMP,
fault, power-off and reset.

Three matched original-OBP SS-20 Solaris pairs on one release binary measured
79.21 s versus 126.60 s boot-to-login and 155 ms versus 265 ms for the fixed
command (interpreter versus JIT). JIT missed the 10% target in every pair.
Instrumentation showed only 4.79% of command instructions retired natively;
91.1% of native calls covered just two or three instructions. Of the
interpreted command fallbacks, 24.7% were control flow, 24.4% were memory
instructions, and 50.9% fell outside those categories. A subsequent
[opcode breakdown and implementation handoff](workstation-jit-branch-memory.md)
identifies the most frequent forms.

Integer branch/delay-slot and RAM load/store native coverage remain
unimplemented. Both branches stay unmerged; no acceleration claim.

## Integer branch exit prototype

The translated span may now end at an integer Bicc. The ALU/CMP prefix and the
branch are admitted in one native call; the host applies the branch condition
to the post-span ICC and leaves the exact taken/untaken/annulled PC/nPC for the
next delay-slot outcome. The supplied three-fixture branch exit case passes,
including fresh ICC, side-effecting delay slots and an unmapped page-edge slot.
Word LD/ST native execution, delay-slot fusion and the matched Solaris rerun
remain outstanding.

### Branch exit implementation

The supplied `bicc_exit.rs` fixture now passes: a CMP/ALU prefix and its terminal
integer Bicc are admitted in one native call, and the host applies the
post-span ICC to leave the exact taken/untaken/annulled PC/nPC for the delay
slot. Fresh ICC, side-effecting slot and unmapped page-edge slot cases pass.
The bare-metal one-instruction native callers still reject branches. Integer
LD/ST native coverage is not implemented; the matched Solaris target remains
unmet.

### Native branch lowering correction

The reviewed native branch exit patch is applied: Bicc is lowered into
generated code, returns relative PC/nPC deltas from the executing branch
address, and the span compiler accepts only a terminal branch after a
supported prefix. The seven supplied branch fixtures pass, including interrupt,
trap, annul, relocation, ICC-changing slot and page-edge cases. The proposed
three-outcome CMP/BNE/delay-slot fusion is not implemented yet: generated
branch code still returns at the branch, so the fixture remains a failing
next step. Word LD/ST and matched Solaris reruns also remain open.

## Delay-slot fusion

Adopted the reviewed fusion patch. An ALU/SETHI prefix, terminal integer Bicc
and one eligible same-page ALU/SETHI delay slot execute in one native call;
the generated code saves the branch exit before the slot, preserves conditional
writeback for annulled slots, and keeps split behavior at budget/deadline/page
edges. The supplied three-outcome, annul, trap/interrupt, relocation and
cache-reuse tests pass. Word LD/ST native execution remains unimplemented.

## Native word RAM access and gate result

Adopted the reviewed native word LD/ST patch. An ALU/SETHI prefix followed by
an aligned ordinary word load/store executes in one native call using the
post-prefix effective address and a warm permitted DRAM mapping. The eight
focused fixtures pass, including exact MMU counters, aliasing store
invalidation, alignment/fault atomicity, dirty-bit transition, protected store
and MXCC fallback.

The three-pair same-binary Solaris comparison missed the acceleration gate:
boot median 77.54 s versus 163.20 s and fixed command 156.9 ms versus 358.2 ms
(threshold 141.2 ms). All guests returned 4950 and halted cleanly. Branches
remain unmerged; no accelerated-workstation claim is made.

## Lazy clock projection experiment

The span path now computes guest-cycle projections only after the live fetch,
opcode eligibility and post-fetch device/deferred-time limit are known, caching
only the requested cumulative cycle times. Benchmarked on the four existing
SS-20 loops, three alternating pairs, 2.4M outcomes per arm:

| Loop | Interpreter | JIT |
| --- | ---: | ---: |
| Isolated ADD/branch | 54.15 | 80.70 |
| ADD+word LD | 65.78 | 112.10 |
| Fourteen ADDs | 55.36 | 42.37 |
| Long loop, DMA reset held | 49.88 | 92.33 |

Compared with the prior probe, rejected-DMA improved 94.6→92.3 and
ADD+load 113.1→112.1; isolated and long loops were unchanged/slightly worse.
Only the long loop remains faster than the interpreter. The change is retained
for its bounded-rejection benefit; the 10% Solaris gate remains missed and no
new guest boot was run.
