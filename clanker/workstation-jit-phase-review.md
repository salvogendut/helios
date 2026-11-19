# Workstation JIT reset and phase measurements

The submitted branches are Helios `29ee5fb` and Heliacal `d851b87`. The
candidate patches and measurement scripts are in
`local/review-workstation-jit-phase-2026-11-19/` in the shared Helios checkout.

## Reset changes the fallback policy on the submitted core

Heliacal sets both policies when it constructs a replacement machine. Within
the running machine, `Cpu::reset_sun4m` constructs a fresh decode cache. Each
of the three CPU-0 reset paths restored native execution and lost the selected
single-instruction policy. A one-outcome ADD called the native backend once
after a cold, software or module-local reset despite selecting interpreted
fallback. The three regressions fail on the submitted commit. Restoring both
settings in the same machine helper fixes all three, including their controls
with the native bridge selected.

## Apply the reviewed changes

From Helios, set `ev="$PWD/local/review-workstation-jit-phase-2026-11-19"`.
Apply `reset-policy.patch` and then `core-phase.patch` to Helios, and
`frontend-phase.patch` to Heliacal. All three were developed in disposable
copies of the branch revisions above.

The opt-in Cargo feature `jit-profile` instruments CPU execution in the
ordinary worker paths. Each CPU exports a snapshot with an epoch unique to
its construction or reset. Checked deltas reject incompatible epochs or
decreasing counters. The histogram counts native calls by length 1–16.
Separate counters cover interpreted retirements, fallback retirements with
JIT enabled, control-flow and memory subsets, synchronous traps, interrupt
entries and CPU error returns. The word and span caches each count hits
(including negative entries), misses/compile attempts, successful
compilations, capacity-evicted entries and host elapsed compilation time.
A guest power-off can return a CPU error at the core boundary and produce a
successful stop in Heliacal.

With this feature, point `HELIACAL_JIT_PHASE_DIR` at a private, fresh
directory. The harness atomically writes a sequence number into `request`.
The worker replies in `sample.toml` at a batch boundary. Match the PID,
request number, execution mode and per-CPU epochs before subtracting. The
worker serves requests during pause and after a guest fault. The timing
binary uses only `jit-native`, which compiles out the diagnostic counters
and request polling.

## What Solaris actually executes

One instrumented original-OBP SS-20 Solaris 2.6 boot reached login, returned
4950 from the fixed guest command, synced, halted to OBP and exited with
status 0. Each phase's worker outcome delta exactly equalled native plus
interpreted retirements, traps and interrupt entries.

| Measured quantity | OBP boot command to login | Fixed command |
| --- | ---: | ---: |
| Retired instructions | 950,598,791 | 1,813,650 |
| Retired natively | 51,724,816 (5.44%) | 86,872 (4.79%) |
| Native calls | 20,359,149 | 35,049 |
| Interpreted control-flow fallbacks | 241,509,790 | 426,062 |
| Interpreted memory fallbacks | 216,844,375 | 422,077 |
| Other interpreted fallbacks | 440,519,810 | 878,639 |
| Span compilations | 1,048,858 | 3,008 |
| Entries evicted at capacity | 1,048,832 | 3,072 |
| Elapsed time compiling spans | 6.203 s | 19.478 ms |

The fixed-command window made zero single-word native calls. Spans of two or
three instructions accounted for 91.1% of its native calls. The remaining
fallback category includes ALU instructions whose surroundings or device
deadlines prevent combining them. Its breakdown by opcode is unmeasured.
The diagnostic samples bracket command delivery and prompt receipt with
worker acknowledgements. Their counts include activity in those short margins.

The retained runner takes an explicit execution mode, checks the guest
answer, hashes the release binary and all seed inputs, and uses disposable
SCSI-3 clones. `measure_solaris.py --diagnostic` captures acknowledged
phase samples. `match_pairs.py` alternates the arms across three pairs and
records boot and command elapsed time and worker CPU time.

## Three matched release pairs

All six arms used the same uninstrumented `jit-native` release binary
(`58b0e784…`), the reset correction, original Sun OBP, one SS-20 CPU,
128 MiB, a headless serial console and disabled parking. The sole execution
switch was `--execution interpreter|jit`. Each arm started with the same
seeded NVRAM, fixed RTC setting and a fresh clone of the same disk. Pair 2
ran JIT first. Every guest returned 4950 from the shell command, synced,
halted to the OBP prompt and exited Heliacal with status 0.

| Median across three arms per mode | Interpreter | JIT |
| --- | ---: | ---: |
| OBP boot command to login prompt, wall | 79.211 s | 126.596 s |
| Boot worker CPU | 77.42 s | 124.14 s |
| Fixed shell command to prompt, wall | 155.00 ms | 264.90 ms |
| Command worker CPU | 0.16 s | 0.27 s |

JIT took 1.59–1.62 times as long to boot and 1.63–1.77 times as long for
the command across the three pairs. Command time includes serial delivery,
guest shell activity, execution of awk and receipt of the prompt. Worker CPU
time comes from host scheduler ticks. The diagnostic run above used a
separate build. The raw paired timings, arguments and input hashes are in
`matched/rows.json` and the medians are in `matched/summary.json`.

## Worker regressions

Five frontend tests drive the actual worker with synthetic guest firmware.
They exercise a translated loop across the 2,048-outcome batch boundary,
pause, keyboard backpressure, BREAK delivery and release, the parking gate,
both CPUs under SMP, and guest fault, power-off and reset during a batch.
The guest-reset test checks the phase epoch and selected fallback policy.
The fault and power-off cases preserve the preceding successful outcomes.
For a negative control, forcing a fixed request of 16 regardless of the
remaining budget made the boundary test fail. Restoring the budget made it
pass. See `worker-budget-control.log` and `worker-budget-restored.log`.

Core suites passed with 814 default, 904 native and 909 profiling test
executions. Heliacal passed 121 default, 121 native and 126 profiling tests. Formatting,
Clippy and the four core demos passed. The three patches pass
`git apply --check` against the submitted branches. The reviewed changes
were exercised in disposable checkouts. The production release and source
media were preserved.

To repeat the comparison, build Heliacal with
`cargo build --release --locked --offline --features jit-native` and run
`python3 "$ev/match_pairs.py" --binary /path/to/heliacal --out /path/to/new-results`
on this host with Xvfb and xdotool available. For counters, build with
`--features jit-profile` and use
`python3 "$ev/measure_solaris.py" --diagnostic --mode jit --binary /path/to/profiled-heliacal --out /path/to/new-samples`.
The two output directories must be new. Patch and measurement hashes are in
`review-files.sha256`.

## Next assignment

1. Carry the reset fix, phase counters and worker tests onto the two feature
   branches. The patches apply in the order above.
2. Extend native execution to integer branches and their delay slots. Test
   taken, untaken and annulled paths, including a fault or interrupt at the
   boundary, against interpreter PC/nPC, effects and outcome counts.
3. Add common integer RAM loads/stores through the existing translation and
   fault paths. Exercise changed mappings, protection, alignment and code
   modified by a store. Keep MMIO and unsupported accesses on their existing
   paths. Preserve the single-CPU scope.
4. Reuse the paired release runner after the execution changes. Report boot
   and command times, plus native share, span lengths and recompilations in
   a separate diagnostic run. Compare with the same-build interpreter.
   Keep both branches unmerged until correctness holds and the useful-work
   median clears the agreed 10% improvement. The current candidate misses
   that target in every pair.
