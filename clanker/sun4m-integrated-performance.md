# Combined sun4m performance on Solaris 2.6

The comparison runs the previous main (`603766a`) against the merged
interpreter (`218356c`). The four commits index DSIMM windows, borrow the
standalone IOMMU in place for DMA, skip empty SS-5 TLB slots, and index
interrupt and timer banks by processor number.

| Workload | SS-5 before → merged | Time saved | SS-20 before → merged | Time saved |
| --- | ---: | ---: | ---: | ---: |
| Kernel banner to root shell | 124.95 → 85.53 s | 31.6% | 76.21 → 66.21 s | 13.1% |
| 2 MiB raw read, 512 B requests | 12.163 → 8.192 s | 32.6% | 4.938 → 4.324 s | 12.4% |
| 2 MiB raw read, 8 KiB requests | 1.256 → 0.819 s | 34.8% | 0.740 → 0.627 s | 15.2% |

Each build booted Solaris once per model with original OBP, one CPU, 128 MiB,
the same model-specific NVRAM seed and a writable clone of the SCSI ID 3 disk.
The benchmark ran one guest at a time on the same host CPU during the measured
intervals. It alternated first runner across 33 segments of each boot and
across four trials of each read size. Boot values sum the timed segments.
Read values are medians from issuing `dd` through the return of the root
prompt. Every read reported the expected record counts and took less time
on the merged build in its paired trial.

At every checkpoint the two builds agreed on PC/nPC, virtual time, retired
instructions, traps, interrupt entries and resets. Both models synced and
halted at OBP. Serial transcripts and NVRAM matched byte for byte. The final
diagnostic snapshot comparisons included RAM and device fingerprints. They
normalized diagnostic-map print order and checked the merged build's derived
TLB search bound against all 64 architectural tags.

## SS-20 small-read profile

A fresh SS-20 clone repeated the sequence with the same merged release
executable. Sampling at 997 Hz on the worker's user-mode CPU clock was
enabled only around the four 512-byte `dd` commands. All 18,721 samples
fell in those windows. Perf recorded zero lost events. The guest matched
the unprofiled merged run at each checkpoint and at halt.

| Sampled instruction ranges | Samples | Share of the four reads |
| --- | ---: | ---: |
| MMU access-context setup and restore | 1,038 | 5.54% |
| Of those, the scalar load path (includes instruction fetch) | 977 | 5.22% |
| Outer CPU step's context setup and restore, counted separately | 234 | 1.25% |

The MMU context ranges accounted for 5.42–5.64% in each read. Ranges
were identified from the disassembly of the measured executable. Its MMU
wrapper symbols also contain inlined memory operations. Counting the whole
wrappers would attribute 14.32% to context management.

The local harness, phase timings, final comparisons, profile, executable
hashes and instruction-range attribution are under
`local/sun4m-integration-2026-11-25-XvSmtPP4/`.

## Reusing the CPU step's bus context

The V8 interpreter already installs the issuing CPU's identity and supervisor
pin for the entire instruction. Instruction fetches and scalar operand loads
can enter the existing MMU load implementation under that context. Direct
calls to `Srmmu::load` still establish their own scope. Tests execute a
guest load of the MID register on CPUs 0 and 2, check restoration of an
enclosing DMA context on success and misalignment, and use SS-5 memory
capture registers to verify the supervisor pin on a user fetch and load.

Two fresh release builds compared `02bc759` with the context change on
the same single-CPU SS-20/SX and Solaris 2.6 workload described above.

| SS-20 workload | Main | Reused context | Wall time saved |
| --- | ---: | ---: | ---: |
| Kernel banner to root shell, one boot per build | 64.645 s | 54.308 s | 16.0% |
| 2 MiB raw read, 512 B requests, four-run median | 4.298 s | 3.743 s | 12.9% |
| 2 MiB raw read, 8 KiB requests, four-run median | 0.614 s | 0.531 s | 13.4% |

The small reads used 4.285 s and 3.725 s median worker CPU time. All four
paired wall times improved at each read size, including both execution
orders. Both guests reached the root shell and synced and halted at OBP
after 1,463,350,000 outcomes. All checkpoint counters, virtual time and
PC/nPC matched. The serial transcripts, NVRAM and final diagnostic snapshots
also matched after sorting the two diagnostic maps for comparison.
The measurements cover the SS-20. The change also applies to the SS-5
and Axil interpreter paths. Validation passed 1,039 default and 1,042
`perf-counters` test executions, including the isolated MXCC child run.
Each configuration had 23 ignored tests. Formatting, both all-target Clippy
checks with warnings denied, and all four demos passed. Build hashes, phase
records and validation logs are in
`local/sun4m-load-context-2026-11-25-jkXNFb0c/`.
