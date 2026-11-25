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

The next experiment is to let instruction fetches and ordinary CPU loads
enter the existing scalar load implementation directly when the enclosing
CPU step has already installed its bus context. Keep
standalone MMU entry points scoped and check CPU 0/2 identity, supervisor
state, faults and context restoration across nested MXCC and DMA accesses.
Repeat the paired SS-20 512-byte read measurement before accepting the
change. Any wall-time gain needs a paired measurement.

The local harness, phase timings, final comparisons, profile, executable
hashes and instruction-range attribution are under
`local/sun4m-integration-2026-11-25-XvSmtPP4/`.
