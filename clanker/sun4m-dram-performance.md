# SS-5, SS-20 and Axil RAM access performance

The sun4m interpreter resolves a physical RAM address after the SRMMU has
finished translating it. Previously the board checked several device windows,
decoded the DSIMM bank and searched a second mapping list for the backing
words. The board can resolve the last two operations once at installation.
An SS-5 needs 32 lookup entries for its 256 MiB aperture. SS-20 and Axil use
64 entries for 512 MiB. A word address selects an 8 MiB window and an offset
within it. Mirrored windows point into the same allocation. On a standard
board the byte, halfword and word entry points check low RAM before calling
the full device decoder. This lets the optimized x86-64 word-read entry skip
the decoder's register saves and stack frame for ordinary RAM.

## How the comparison was run

The baseline was Helios `603766a`. The host was a Ryzen 9 PRO 8945HS
running x86-64 Linux with Rust 1.98.1. Both sides used default-feature release
builds, the production single-CPU interpreter and identical guest media.
Each guest had 128 MiB and its original OBP. SS-5 and Axil had CGsix and the
SS-20 had SX. Each process had its own writable SCSI ID 3 disk clone. Both
started with the same per-model NVRAM seed.

- The physical bus control alternates read/modify/write operations across
  four 32 MiB ranges. Twelve million iterations use 1,024 words. The runner
  verifies a checksum and every final word against an independent array. Run
  `cargo run --release --example perf -- dram ss5` and substitute `ss20` or
  `axil` for the other profiles. This control needs no guest media.
- For the kernel comparison, each guest boots Solaris 2.6 from its disk.
  After the kernel banner, two processes alternate on host CPU 3. Each takes
  exactly four million outcomes per interval, with eight intervals per
  machine. Only one process executes at a time. The runner checks executed
  instructions, traps, interrupts, resets, PC/nPC and virtual time at every
  boundary.
- The SS-20 pair continues to root, then runs three separate
  `cp -R /kernel /var/tmp/heliosdramN && sync` commands. A separate timed
  `diff -r` verifies each copy before clean halt to OBP. All three profiles
  compare the serial output, NVRAM bytes and final diagnostic snapshot.
  Comparison of the two MMU diagnostic maps normalizes hash-map print order.
- The SCSI controller control reads a seeded 2 MiB image via ESP and the
  profile's IOMMU. It checks the residual, final DMA address and every guest
  word for both 512-byte and 8 KiB requests. The timer covers controller
  commands and DMA. The separate host backing-file reads and result checks
  are outside that timer.

Raw samples, guest transcripts, harness sources and build hashes are under
`local/sun4m-dram-perf-2026-11-25/`. The host used its normal powersave CPU
governor. Fixed guest work and alternating execution reduce the effects of
host variation. The eight kernel intervals cover one boot per build and
profile. The three copies share one filesystem cache per guest. The timings
apply to these specific phases.

## Solaris results

| Phase | Main | Branch | Wall time saved |
| --- | ---: | ---: | ---: |
| SS-5, first 32M outcomes after kernel banner | 5.286 s | 4.781 s | 9.6% |
| SS-20, same interval | 3.326 s | 3.021 s | 9.2% |
| Axil 320, same interval | 3.390 s | 3.139 s | 7.4% |
| SS-20, end of that interval to root shell | 83.773 s | 75.028 s | 10.4% |
| SS-20, copy and sync (median of three) | 6.809 s | 5.662 s | 16.9% |
| SS-20, verify copy (median of three) | 8.889 s | 7.978 s | 10.2% |

The three copy-and-sync pairs each improved, by 5.5%, 16.9% and 12.4%
in run order. This guest workload includes the Solaris filesystem, its cache,
the disk controller and the interpreter. Check it separately from controller
throughput or an entire desktop boot.

The SS-20 reached OBP after 1,698,200,000 outcomes on both builds. The
serial transcripts, NVRAM and final snapshot comparisons passed on all
three machines. SS-5 and Axil stopped after their early kernel interval.
The guest timings here cover single-CPU execution.

## Control measurements

| Workstation | 12M physical RAM updates, main → branch | 512 B ESP requests, main → branch | 8 KiB ESP requests, main → branch |
| --- | ---: | ---: | ---: |
| SS-5 | 191.8 → 68.8 ms | 150.3 → 147.5 ms | 139.9 → 141.1 ms |
| SS-20 | 221.0 → 52.8 ms | 106.1 → 103.1 ms | 94.8 → 93.7 ms |
| Axil 320 | 208.2 → 52.6 ms | 105.3 → 94.8 ms | 95.7 → 86.0 ms |

RAM values are medians of three runs per arm. ESP values are medians of
nine 2 MiB reads per request size and arm. SS-5's 8 KiB case increased by
0.8%. Results for the shared SS-20/Axil controller path varied across runs.
The bare-metal ALU and load/store controls measured 43.58 → 42.76 and
55.65 → 55.21 ns per outcome respectively. These controls use a separate,
contiguous memory implementation. The raw results for the final version
are in the `dispatch/` subdirectory.

## Validation

The default suite reported 1,034 passing test executions and the
`perf-counters` suite reported 1,037. Both reported zero failures and 23
ignored tests. The counts include the isolated MXCC child process.
Formatting, all-target Clippy with warnings denied in both configurations,
and the four built-in demos passed. The release `perf dram` example
returned checksum `730c7080` on all three boards.

## Development comparison

The first version built the window table and checked low RAM inside the
large bus methods. A sequential SS-20 boot and filesystem run took 156.36 s
on main and 174.82 s on that version. A second, interleaved comparison of the
same version found a 10.8% reduction in SS-20 copy-and-sync time, an 11.5%
reduction in the SS-5 early kernel interval and a 5.1% increase in that
interval on Axil. These observations motivated the separate RAM and MMIO
entry paths and a fresh comparison to main.
