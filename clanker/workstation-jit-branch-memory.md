# Workstation JIT branch and memory handoff

Start from Helios `9c75e88` and Heliacal `b33eb55`. All 14 files from
the three reviewed patches match their committed Git blobs. The five phase
tests and three reset tests pass on the committed core. The installed
Heliacal release hashes to `7600f48595b98357fbf0d3e2a9e21f4c8412e06b6b685747cadeb05e8e876f9d`.

## Which instructions to target

One fresh diagnostic run on the original-OBP, single-CPU SS-20 reached Solaris
2.6 login, returned 4950 from the fixed shell command, synced, halted to OBP
and exited Heliacal with status 0. The diagnostic counts every retired
interpreted instruction in JIT mode by SPARC encoding. The buckets reconcile
with the committed phase counters in both windows.

| Interpreted instruction group | Boot to login | Fixed command | Share of command fallbacks |
| --- | ---: | ---: | ---: |
| SETHI and ALU forms the emitter already supports | 357,336,964 | 733,186 | 40.82% |
| Integer conditional branches (Bicc family including BA/BN) | 155,029,083 | 294,470 | 16.39% |
| Ordinary word LD | 117,910,345 | 252,086 | 14.03% |
| Ordinary word ST | 29,922,876 | 74,565 | 4.15% |
| All interpreted fallbacks | 897,911,642 | 1,796,173 | 100% |

The first row accounts for 80.7% of the command's previously unclassified
fallbacks. The counts identify encodings. They cannot distinguish short
blocks from deadline or MMU proof restrictions, identify branch/slot pairs,
or distinguish RAM from MMIO for a particular load. A separate uninstrumented
three-pair comparison remains the timing baseline. [Matched results](workstation-jit-phase-review.md)

## Implementation order

1. **End a translated span with integer Bicc.** Start with an ALU prefix
   that computes ICC and a terminal branch that reads those flags within
   the same native call. A two-outcome budget must be able to stop after
   the branch. Return the actual PC/nPC and the executed instruction
   addresses. The current span runner reconstructs outcomes by subtracting
   from a linear final PC. The code cache is keyed by words and can reuse
   a block at a different virtual address, so calculate the relative
   target using the executing guest PC. Keep the straight-line native
   callers on their existing opcode subset until they handle branch exits.
2. **Execute an eligible delay slot.** First make a split at the slot
   work for taken, untaken, annulled, backwards and page-edge branches.
   Then include supported slots when fetch effects, remaining budget and
   device/IRQ timing allow. Keep an annulled slot out of the execution
   count and its fetch out of guest-visible state. Cover an interrupt
   between the branch and slot, a slot fault with the saved target nPC,
   and a condition-code-writing slot. Measure native branch retirements
   explicitly so success cannot come from silently interpreting the branch.
3. **Begin memory coverage with ordinary word LD, then ST.** Use existing
   MMU/bus behavior for effective addresses, permissions, byte order and
   referenced/modified bits. Preserve prior retired work on a fault. Test
   the destination and memory state, MMIO fallback, alignment, mapping
   changes and stores to following code through a physical alias. A store
   which changes a later instruction must be visible before executing it.
4. **Measure once there is execution coverage.** Use the same release
   binary in both modes, the same fresh clones/seeds and the prior three
   alternating pairs. Use a separate diagnostic run for coverage, span
   lengths, cache churn and compilation time. Keep the interpreter default
   and both branches unmerged until the useful command meets the agreed
   10% improvement with matching guest results.

## Ready-to-run branch fixture

Copy `local/review-workstation-jit-opcodes-2026-11-19/bicc_exit.rs` to
`tests/bicc_exit.rs` in the feature checkout and run
`cargo test --locked --offline --features jit-native,perf-counters --test bicc_exit`.
Two cases check the interpreter-visible outcomes, all snapshot fields and
MMU counters for fresh ICC, delay-slot effects and an unmapped slot.
The third requires one native call containing CMP and BNE at a budget of
two. It fails on the submitted checkpoint, which returns only CMP.
Use the existing decoded-branch condition matrix for the remaining ICC
combinations and displacement cases.

## Reproduction

Local patches, the decoded histogram and per-phase raw samples are under
`local/review-workstation-jit-opcodes-2026-11-19/`. In disposable sibling
checkouts of the two commits, apply the corresponding `helios.patch` and
`heliacal.patch`, build Heliacal with
`cargo build --release --locked --offline --features jit-profile`, then use
the existing `measure_solaris.py --diagnostic --mode jit` with the binary
and a new output directory. Run `report.py` beside the resulting `run/`
directory to check both phase totals. This measurement used a separate
diagnostic binary, a fixed RTC/NVRAM seed and a disposable SCSI-3 clone.
