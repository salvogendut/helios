# Ultra 5 performance compared with QEMU

Measured on 2026-11-23 with Helios `d3380c1`, Heliacal `2bbbf03` and QEMU 10.2.2 on an AMD Ryzen 9 PRO 8945HS. The Solaris measurement used the production V9 and board code with a serial harness. The cross-emulator measurement used a synthetic ROM on both system emulators. Each answers a different question.

## Cost in an installed Solaris 7 guest

Original OpenBoot 3.31 booted a disposable copy of the installed IDE disk with `boot disk -s`. At the serial batch boundary containing the kernel banner, the harness measured the next 75 million board outcomes. The window started at outcome 40,019,317,967. It includes 5,852 synchronous traps, 731 interrupt entries and 9,679,766 instructions covered by countdown batching. No graphical frontend ran in the sampling process.

| Enclosing host function | Samples | Share of 3,974 CPU samples |
| --- | ---: | ---: |
| V9 address translation (`translate_asi`) | 1,589 | 40.0% |
| Board countdown check (`repeat_countdown`) | 418 | 10.5% |
| V9 instruction retirement (`retire_fetch`) | 343 | 8.6% |
| V9 cacheable instruction fetch and retirement (`retire_ram_instruction`) | 299 | 7.5% |
| V9 instruction execution (`execute_fetched`) | 287 | 7.2% |
| Board single step (`step_outcome`) | 253 | 6.4% |
| Board interrupt polling (`sample_interrupts`) | 247 | 6.2% |

The table assigns each sampled host instruction to one enclosing binary function, including compiler-inlined work. Out-of-line callees have their own samples. These are CPU samples from one early-kernel phase. Host disk wait, CDE startup time and Xsun drawing costs require their own measurements.

**Address translation.** In the baseline, the V9 instruction and data TLBs each remembered one uniquely matched 8 KiB subpage and context. A change of key took the slow lookup, which searches the 64 architectural entries and checks for a second match. The sampled leaf functions include page matching, size-mask construction and iteration over those entries. An alternating two-page workload could repeatedly pay for this search even with both translations resident. A larger host lookup hint can retain both slot indices while the architectural TLB continues to supply the permissions, physical address and Used bit. [V9 TLB lookup](../src/v9/tlb.rs)

**Board service.** Heliacal tries the countdown recognizer before each ordinary Ultra 5 step. The board samples interrupts, checks vector eligibility and calculates peripheral deadlines. When recognition fails, the normal step samples interrupts again at the same virtual time. The sampled window made 65,320,340 attempts with 106 successes. Those successes bypassed nearly 9.7 million individual retirements, so the profitable change is to make rejection cheap while preserving successful batching. [Heliacal worker](https://github.com/salvogendut/heliacal/blob/2bbbf03c272684c1eaf6917a9aed716a0d454dfc/src/emulator/ultra5.rs#L35-L72) · [Ultra 5 board](../src/ultra5/board.rs) · [Countdown recognition](../src/v9/primary_cache.rs)

### Isolated checks on the same Solaris work

Each run booted a fresh disk clone with the same ROM and NVRAM seed. Both experiments used the same banner boundary, release settings and 75 million outcomes. Timing runs had profiling disabled.

| Change from current code | Run order on one host core | Current | Experiment | Wall time saved |
| --- | --- | ---: | ---: | ---: |
| 32 host lookup hints per I/D TLB | Baseline first, core 6 | 4.028 s | 2.995 s | 25.6% |
| Same 32 hints | Experiment first, core 2 | 4.063 s | 3.044 s | 25.1% |
| Probe for countdowns every 31st outcome after the banner | Baseline first, core 6 | 4.028 s | 3.349 s | 16.8% |

The TLB experiment cached verified slot indices keyed by the full subpage/context, checked current permissions on use, and cleared hints on mapping writes and demaps. Both pairs matched the baseline serial transcript byte-for-byte, final PC/nPC, both recorded registers, trap/interrupt counts, countdown hits and accelerated instruction count. Fifteen enabled V9 TLB unit tests passed in the temporary build. The cadence experiment also matched the transcript, endpoint and trap/interrupt counts. It found all 106 countdown batches in this window with 2,107,266 attempts and retired 3,242 more instructions individually. The two experiments ran separately.

**The 32-entry hint is the first production candidate.** Validate it on duplicate and overlapping mappings, different page sizes, context switches, demaps, write protection and architectural Used-bit updates. Then time a cold boot to Xsun and a fixed guest file copy. For a second independent change, try a cheap countdown eligibility check or sharing the board poll within the same step. Preserve device deadlines, DMA snooping and interrupt ordering, and time an OBP delay-heavy phase as well as Solaris.

Raw logs, the harness, the scratch TLB diff, profile symbol counts and reproduction instructions are retained under ignored `local/ultra5-perf-2026-11-23/`. The experiments ran in temporary builds against disposable disk clones.

## Equal-work comparison with QEMU

Both system emulators ran the same synthetic 1 MiB flash image from reset through 41,943,045 SPARC instructions. Its hot loop contains ADD, XOR, SUBcc, a conditional branch and a delay-slot NOP. Five alternating runs used the same host CPU core. Both implementations reached the completion address with the expected counter and accumulator. QEMU used `sun4u`, the TI UltraSPARC-IIi model and single-threaded TCG. Timing excluded process and board construction. QEMU's completion breakpoint was on a separate guest page.

| Configuration | Median wall time | Five-run range |
| --- | ---: | ---: |
| Helios board with Heliacal’s probe cadence | 1.267 s | 1.253–1.281 s |
| Same Helios board with the unsuccessful probe omitted | 0.863 s | 0.862–0.870 s |
| QEMU system emulation | 0.0334 s | 0.0165–0.0339 s |

QEMU took about 1/38 of the Helios time on this small, branch-heavy flash loop. TCG translates guest instructions into reusable host code and can link eligible same-page branches between translated blocks. Its SPARC translator records PC/nPC for architectural recovery. Helios fetches, decodes and retires each ordinary V9 instruction through the Rust interpreter. Its existing optional native backend targets the separate V8 execution path. The measured ratio applies to this loop only. [QEMU TCG internals](https://www.qemu.org/docs/master/devel/tcg.html) · [QEMU 10.2.2 SPARC translator](https://github.com/qemu/qemu/blob/v10.2.2/target/sparc/translate.c#L5388-L5408) · [Helios V9 retirement](../src/v9.rs)

The 440 MHz banner describes the clock exposed to the guest. Helios charges 100 ns of virtual time per ordinary board outcome, with 44 guest clock ticks per retirement. Host throughput depends on the work the emulator does to retire each instruction. The countdown shortcut can account for many retirements at once, making a firmware-wide steps-per-second figure a poor measure of useful Solaris work. [Board time](../src/ultra5/board.rs) · [V9 tick accounting](../src/v9.rs)

The 33 ms publication interval in Heliacal limits display updates while the Ultra 5 worker continues executing batches. Frame publication also checks the scanout generation. The synthetic gap and the Solaris CPU samples above were measured without SDL. On an I/O-specific workload, Ultra 5 IDE DMA already performs a host read or write for a whole request, then moves data through the sparse DRAM bus in chunks of up to eight bytes. CPU-side copying and guest driver time should be measured separately from host file latency. [Frontend worker](https://github.com/salvogendut/heliacal/blob/2bbbf03c272684c1eaf6917a9aed716a0d454dfc/src/emulator.rs#L1324-L1336) · [ATA bulk transfer](../src/ultra5/ata.rs) · [DMA and DRAM](../src/ultra5/dram.rs)

The earlier SS-5 profile found 48.9% of samples in its SRMMU group. That machine and the SS-20/Axil use the separate sun4m implementation. The V9 finding and its prospective change apply to the Ultra 5. [SS-5 profile](performance-issue5-slice-a.md) · [SS-5 replacement investigation](ss5-wp0-investigation.md)

## Production lookup hints and cold boot

The production V9 lookup now keeps 32 uniquely verified subpage/context-to-slot hints in each TLB. Regression tests warm several subpages, then insert a smaller overlapping mapping and demap it. A second test alternates colliding virtual pages and contexts with different access permissions and checks the individual slots’ Used bits.

For the cold-boot comparison, each release build started from power-on on the same host CPU with a separate reflink of the installed IDE0 disk and identical auto-boot-enabled NVRAM. The headless measurement harness attached the graphical keyboard, used Heliacal’s ordinary board-stepping and countdown cadence, and compared the guest Mach64 pixels with the retained Xsun login image. Timing ended at the first sampled framebuffer with at least 1,035,001 matching pixels out of 1,036,800.

| Reset to the Xsun login | Wall time | Board outcomes | Final PC | Matching pixels |
| --- | ---: | ---: | --- | ---: |
| Baseline `d3380c1` | 309.767 s | 54,273,501,715 | `0x10040aa4` | 1,036,773 |
| 32 lookup hints | 254.065 s | 54,273,501,715 | `0x10040aa4` | 1,036,773 |

The single matched pair saved 55.7 seconds (18.0%). Host load can affect wall time. The frontend release linked against the changed core also passed its SDL smoke test. Measurements use the guest framebuffer as the endpoint and exclude SDL presentation.

### Solaris file copy

Both builds booted the same installed Solaris 7 image on separate disposable IDE disks. A serial single-user shell created three fresh copies of `/kernel` under `/var/tmp`. Each timed interval ran from a guest marker immediately before `cp -R` to a marker after `sync` returned. A subsequent `diff -r` verified each destination. The measurement pinned both processes to host CPU 4 and serialized the timed workloads. Passes two and three can reuse guest and host caches.

| Copy plus sync | Baseline | 32 lookup hints |
| --- | ---: | ---: |
| First copy | 3.864 s | 3.460 s |
| Second copy | 2.882 s | 2.604 s |
| Third copy | 3.004 s | 2.761 s |
| Median | 3.004 s | 2.761 s |

All six copies verified. Median wall time fell 8.1% for this filesystem workload. IDE and DMA code were identical in both arms. The change affects the CPU cost of executing the guest application, filesystem and drivers.

Raw timing results, harness sources and validation logs are under ignored `local/ultra5-tlb-speed-2026-11-23/`. The full core suite reported 1,006 passing test executions; all four explicit local-ROM V9 TLB checks, formatting, strict Clippy, all four demos, and Heliacal’s 121 tests passed.

## Share a board poll and instruction translation

On the Ultra 5, Heliacal now calls `step_or_repeat`. The board applies DMA snoops, samples interrupt sources, checks for an eligible vector, and resolves the instruction address once at each boundary that reaches fetch. When the countdown check misses, ordinary execution uses the same translation or instruction fault. The board calculates the next peripheral deadline only after recognizing a resident countdown. Proven loops retain the caller budget and the peripheral and CPU timer limits.

Both builds included the 32 TLB hints. Each timing arm started from the same Solaris 7 disk snapshot and NVRAM seed on its own disposable writable IDE0 disk. The graphical harness used the same Xsun pixel threshold and pinned core as the previous experiment. Both runs reached PC `0x10040a7c` after 54,283,508,138 board outcomes, with 1,036,773 of 1,036,800 pixels matching the reference.

| Workload | TLB hints alone | Combined step | Wall time saved |
| --- | ---: | ---: | ---: |
| Cold reset to Xsun login | 255.270 s | 179.236 s | 29.8% |
| First `/kernel` copy and sync | 3.588 s | 2.460 s | |
| Second copy and sync | 2.832 s | 1.880 s | |
| Third copy and sync | 2.794 s | 1.884 s | |
| Median copy and sync | 2.832 s | 1.884 s | 33.5% |

The copy timings cover three fresh destinations per serial boot, pinned to host CPU 4. All six recursive comparisons succeeded. These are observed times for one baseline and one final boot per workload on this host. The core tests separately compare the combined boundary with single stepping at serial timeout, interrupt entry, tick compare in a delay slot, DMA-modified instructions, and a demapped instruction page. They also cover a zero budget, a synchronous trap and an unsupported instruction.

Validation passed with 1,011 core test executions, four explicit local-ROM V9 TLB checks, all four demos, and 121 Heliacal tests. Both repositories passed formatting and strict Clippy. The rebuilt frontend passed the SDL smoke and displayed original Ultra 5 OBP output with the dummy video driver. Raw timings and harnesses are in ignored `local/ultra5-poll-2026-11-23/`.
