# sun4m interrupt and timer bank lookup

The Solaris UFS profile attributed about 9.4% of boot samples inclusively to
`Sun4mDevices::sync_interrupts`. That profile predates the DRAM and TLB
optimizations. The baseline for this experiment is `9c4b0e1`, which includes
both changes. The benchmark uses default-feature Cargo release builds.

The machine samples interrupt inputs on CPU service boundaries and during
device access, including SCSI DMA. The interrupt controller also supplies the
pending levels for the selected CPU. Both the timer and interrupt controller
used ordered maps for their processor banks. The hardware selector has only
four possible values. The replacement stores four optional banks and indexes
them by hardware processor number. The board still enforces the allowed CPU
population for each model. Device service, sampling boundaries and processor
selection use the same paths as the baseline.

The bank fingerprint still encodes the number of populated entries followed
by hardware number and contents in ascending order. The regression checks
all 16 population masks against the original ordered-map encoding, installs
banks out of order, repeats installations after writing values, and checks
all 256 possible byte-sized processor IDs.

## SCSI controller replay

The replay reads 2 MiB through ESP/DMA and checks transfer residuals, DMA
addresses and every destination word. Each process runs three trials per
request size. The table reports the median of the per-process medians across
four alternating baseline/candidate pairs on the same logical host CPU.
The controller interval excludes the separate backing-file measurement and
the destination-word checks.

| Model | Request size | Baseline | Indexed banks |
| --- | ---: | ---: | ---: |
| SS-5 | 512 B | 220.20 ms | 205.48 ms |
| SS-5 | 8 KiB | 190.33 ms | 188.30 ms |
| SS-20 | 512 B | 150.56 ms | 117.59 ms |
| SS-20 | 8 KiB | 133.45 ms | 102.23 ms |
| Axil 320 | 512 B | 130.93 ms | 112.69 ms |
| Axil 320 | 8 KiB | 132.03 ms | 100.32 ms |

All four SS-20 and Axil pairs improved for each request size. The host ran
other workloads and used a frequency-scaling governor. The absolute times
are specific to this run.

## Solaris 2.6

Two single-CPU SS-20/SX machines booted original OBP with 128 MiB and
identical NVRAM, each with a writable clone of the installed SCSI ID 3
disk. Both executables ran on logical host CPU 3. The coordinator released
one guest at a time and reversed the order between trials of each read size.
The shell commands verified all input and output records. The reported
interval includes serial command delivery and return to the prompt.

| SS-20 guest workload | Baseline | Indexed banks | Elapsed change |
| --- | ---: | ---: | ---: |
| Kernel banner to root shell, one boot per build | 79.10 s | 76.56 s | −3.2% |
| Read 2 MiB, 512-byte requests, four-run median | 4.764 s | 4.742 s | −0.5% |
| Read 2 MiB, 8 KiB requests, four-run median | 0.705 s | 0.680 s | −3.5% |

Two of four 512-byte reads improved, as did all four 8 KiB reads. The
512-byte result is effectively flat in Solaris. The two guests synced and
halted at OBP after 1,463,350,000 outcomes with identical PC/nPC, virtual
time, instruction, trap, interrupt and reset counts at every checkpoint.
Serial output, NVRAM and final diagnostic snapshots matched, including RAM
and device fingerprints. The snapshot comparator sorts the printed keys of
the two MMU diagnostic maps. An earlier long-run process was terminated
during boot and its timings are excluded from this table.

Separate single-CPU controls compared the first 32 million outcomes after
the Solaris kernel banner, in eight alternating intervals. SS-5 took
3.684 s before and 3.513 s after (−4.7%), SS-20 took 3.271 s and 3.209 s
(−1.9%), and Axil took 2.901 s and 2.815 s (−3.0%). Guest outcomes, serial,
NVRAM and final diagnostic snapshots matched. The two-CPU SS-20 repeated the
same state comparison through the scheduled execution path. That run
overlapped test-suite validation and is used only to check behavior.

## Separate interrupt-line experiment

An initial candidate combined six level-sensitive device inputs in one
controller update. Its early Solaris interval took 1.6% less time on SS-20
and 8.0% less on Axil. The controller replay varied in both directions and
the SS-5 512-byte median increased. The indexed-bank comparison uses the
baseline interrupt-line implementation. The earlier patch, its measurements
and the indexed-bank evidence are retained under
`local/sun4m-irq-sampling-2026-11-25-98A2jk0r/`.

The final code passed 1,037 default and 1,040 `perf-counters` test
executions. Each suite had 23 ignored tests and includes the isolated MXCC
child run in the passing count. Formatting, both all-target Clippy runs
with warnings denied, and all four built-in demos passed.
