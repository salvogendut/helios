# SS-5 sparse TLB lookup

The original SS-5 OBP leaves TRCR.WP at zero. In the retained early Solaris
2.6 snapshot, only slot zero holds a valid tag. A miss in the host fast cache
used to examine all 64 architectural slots before walking the page tables.
The MMU now checks slot zero directly when all higher slots are invalid.
Otherwise it runs the original fixed-size CPU search. SS-5 integrated I/O
lookups scan only through the highest potentially valid slot. Fills extend
that bound, and flushes and SS-5 diagnostic tag writes recompute it. A
reference-MMU diagnostic invalidation can safely leave an overly generous bound.

All 64 slots remain addressable. The tests put two CPU mappings in locked
slots 23 and 63 under WP=0, exercise slot-zero replacement and invalidate the
duplicates in turn. The I/O test checks that a later matching entry wins and
that a malformed entry in slot 63 still reports an error after a low-slot hit.
The replacement counter, page walk, reference/modified bits and guest fault
handling use their existing paths. The firmware evidence is in the
[WP=0 investigation](ss5-wp0-investigation.md).

## Solaris 2.6

The baseline was `b7d925b`. Both executables were default-feature release
builds. The harness used the production single-CPU interpreter, original OBP,
128 MiB, matching per-model NVRAM seeds and a separate writable SCSI ID 3
disk clone per process. A coordinator ran one process at a time on host CPU
3, alternating order between adjacent kernel intervals and between repeats
of the same read size. Times below exclude the wait for the other process.

| SS-5 phase | Baseline | Bounded lookup | Elapsed change |
| --- | ---: | ---: | ---: |
| First 32M outcomes after the kernel banner, eight intervals in one boot | 3.443 s | 2.843 s | −17.4% |
| End of that interval through login to a root shell, one pair | 99.715 s | 83.947 s | −15.8% |
| Raw 2 MiB read, 512-byte requests, four-run median | 9.900 s | 8.275 s | −16.4% |
| Raw 2 MiB read, 8 KiB requests, four-run median | 1.051 s | 0.833 s | −20.8% |

Both request sizes improved in all four pairs. Each timer included serial
command delivery, guest execution and return to the shell prompt. Each
command checked the guest's input and output record counts.
Both guests synced and halted at the OBP prompt after 1,680,150,000 outcomes.
Each checkpoint had identical instruction, trap, interrupt and reset counts,
PC/nPC and virtual time. Serial transcripts and NVRAM were byte-identical.
The final snapshot comparison included the RAM and device fingerprints. It
normalized diagnostic-map print order and independently checked the new
host-only bound against every valid tag before comparing the remaining state.

The SS-20/SX and Axil/CGsix controls each ran the same eight post-banner
intervals and snapshot checks on Solaris 2.6. SS-20 took 2.349 s with the
baseline and 2.346 s with the change (−0.09%). Axil took 2.352 s and
2.358 s (+0.24%). Both controls ended after the first 32M kernel outcomes.

## Fixed translation workloads

Each run performed 4,194,304 checked word loads on a power-of-two page ring.
Four alternating baseline/candidate pairs used the same host CPU. Times are
median nanoseconds per access.

| Replacement wrap and ring | Baseline | Bounded lookup |
| --- | ---: | ---: |
| WP=0, 32 pages, repeated misses | 80.61 | 48.62 |
| WP=63, 128 pages, repeated misses in a full TLB | 90.32 | 90.34 |
| WP=63, 32 pages, warmed hits | 14.69 | 14.76 |

An earlier implementation used a variable-length CPU scan for all occupancies.
It made full-table misses 4.4% slower in the initial four-pair control. A
six-order comparison with all three binaries reproduced that cost. This
led to the direct slot-zero check and fixed-size dense scan used above.

The harnesses, raw pairs, release binary hashes, guest transcripts, snapshots
and validation logs are under `local/ss5-tlb-search-2026-11-25-5KtErFmy/`.
The `final/` subdirectory contains the timings for the retained implementation.

The default suite passed 1,036 test executions and `perf-counters` passed
1,039. Each count includes the isolated MXCC child run and excludes 23
ignored tests. Formatting, all-target Clippy with warnings denied in both
configurations, and all four built-in demos passed.
