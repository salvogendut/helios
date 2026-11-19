# Workstation JIT single-instruction fallback comparison

Heliacal `6bd8e80` adopts the reviewed `outcome.len()` change exactly. That
source with Helios `dd95120` passed the native build check and 121 frontend
tests in the preceding review.

**Carry interpreted fallback into the Solaris comparison.** In a release build
of Helios, interpreting the already-fetched word when a span could not run
reduced time in every repeat on four synthetic SS-20 workloads. Longer
eligible spans continued to execute natively.

| Guest loop | Ordinary interpreter | Spans with one-word bridge | Spans with interpreted fallback |
| --- | ---: | ---: | ---: |
| ADD, branch, delay NOP | 53.46 | 161.14 | 137.56 |
| Alternating ADD and RAM load, branch, delay NOP | 66.03 | 183.27 | 164.16 |
| Fourteen ADDs, branch, delay NOP | 56.22 | 60.74 | 56.75 |
| Same fourteen ADDs with SCSI DMA reset held | 51.32 | 168.96 | 144.06 |

Entries are median host nanoseconds per retired guest instruction. Each arm
ran 24,000 warm-up outcomes and 2.4 million timed outcomes in a fresh process.
All six orders of the three modes used the same binary, a translated warm
4 KiB page, one CPU, a full SS-20 board and no display. The checksum, end
PC/nPC and virtual clock were asserted on every trial. The probe sets its
policy once before warm-up and leaves the ordinary `Machine::step` bridge
available. The timing binary has `perf-counters` off.

**Span admission also has a measurable cost.** The branch checks readiness for
candidate lengths from 16 down to 2 before testing the first opcode. A second
experimental patch checks that opcode first, computes the maximum permitted
elapsed time once, and bounds the suffix by the budget and that time. It keeps
the existing readiness check before committing a candidate span and keeps the
first live fetch. Both changes were measured together.

| Guest loop | Interpreted fallback | With revised admission |
| --- | ---: | ---: |
| Isolated ADD | 142.13 | 109.33 |
| Alternating ADD and load | 174.13 | 125.16 |
| Fourteen ADDs | 57.74 | 53.48 |
| SCSI DMA reset held | 147.43 | 91.45 |

This second experiment used six alternating pairs of release binaries.
Instruction, native call and span totals, and guest checksums matched between
arms on every trial. Each revised arm was faster in all six pairs. The
short loops still cost substantially more than ordinary stepping. These
measurements cover fixed guest loops. Solaris command timing remains open.

The combined prototype passed the 23 existing pair/span fixtures. A separate
test compared the new elapsed-time limit to the original readiness predicate
at timer deadlines, deferred-clock boundaries and near `u64::MAX`. Eight
differential replays checked both fallback policies on the four loops. Each
matched every returned outcome against ordinary stepping for 2,400 instructions,
then matched the machine snapshot, MMU counters and a fault after overwriting
the next instruction. The experiment used disposable sources. Production
adoption and full validation belong to the implementation pass.

The evidence directory contains both experimental patches, the guest probe,
raw timings, differential fixtures and a replay script. Run the script with
the Rust toolchain on PATH and the offline Cargo cache selected.

`local/review-workstation-jit-fallback-2026-11-19/`

## Work order for the implementation agent

1. Start from Helios `dd95120` and Heliacal `6bd8e80`. Use the two experimental
   patches as a reference for a candidate with interpreted single-word fallback
   and bounded readiness checks. Keep the bridge as the comparison arm. The
   prototype's policy setter is a measurement hook. Carry the policy across
   reset/replacement wherever Heliacal selects it for a trial.
2. Add the missing counters at guest phase boundaries and worker tests in the
   same delivery. Record reset epochs, the 1–16 native-call histogram,
   interpreted fallbacks, compilations, cache misses and evictions. Exercise
   the actual 2,048-outcome loop with a short final budget and progress before
   fault, power-off or reset. Check keyboard/BREAK, parking and SMP paths.
3. Run the three matched original-OBP Solaris useful-work pairs with explicit
   interpreter/JIT selection on one release binary, parking off and fresh disk
   clones. Keep per-phase wall time, worker CPU time, counters, argv and binary
   hash. If useful work still falls short of the 10% target, use the observed
   fallback distribution to prioritize branch/delay-slot and integer-memory
   work. Keep both branches unmerged pending that measurement and correctness
   coverage.

The subsequent [reset and phase review](workstation-jit-phase-review.md)
supplies the reset fix, counters, worker regressions and all three Solaris
pairs. Branch and memory execution coverage are the next implementation work.
