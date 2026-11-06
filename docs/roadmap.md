# Development milestones

The first full-system target is SPARCstation 5 / sun4m with an 85 MHz microSPARC-II,
128 MiB RAM, CGsix graphics, SCSI disk/CD-ROM, Ethernet and TTYA/TTYB. Each milestone needs
observable acceptance criteria before its status can advance.

| Milestone | Deliverable and acceptance | Status |
| --- | --- | --- |
| 0 — Foundation | Library/CLI, integer subset, word bus, runnable bare-metal demo and regression tests | Implemented |
| 1 — CPU baseline | Complete V8 integer/privileged behavior, PSR/WIM/TBR/Y, precise traps/RETT, ASI dispatch, multiply/divide and atomics; independently checked bare-metal tests | In progress: core integer instruction groups, privileged state, synchronous traps, ASIs, doublewords and atomics implemented; implementation-specific ASRs and broader illegal-opcode classification pending |
| 2 — Memory system | microSPARC-II reset/boot behavior and SRMMU, permissions, contexts, faults, TLB invalidation and required cache controls; translation/fault tests | In progress: translation, TLB flush/probe and cache diagnostic storage/invalidation implemented; DRAM banks and live TLB diagnostics implemented; injected PTP walks, M-bit rewalks and cache hits/fills pending |
| 3 — Serial firmware | Original Sun OBP loading, NVRAM/IDPROM, Z8530, interrupt controller and timers; reproducible OBP prompt transcript | In progress: OBP passes IOMMU/bus-error diagnostics and FPU transfers, then passes comparisons/branches and stops at FDIVs; arithmetic support and working prompt pending |
| 4 — Storage and kernel | IOMMU, DMA, ESP SCSI, disk/CD-ROM targets and safe image writes; installer/kernel reaches serial userspace | In progress: IOMMU translation/flushes and RAM DMA API implemented; SCSI/Ethernet DMA engines and storage pending |
| 5 — Usable workstation | Required FPU behavior, LANCE networking, CGsix/TurboGX framebuffer, keyboard/mouse and host frontend; documented guest compatibility matrix | FPU state/transfers/comparisons/branches and deferred invalid exceptions implemented; arithmetic and remaining devices pending |
| 6 — Performance and tooling | Profiling-driven optimization, optional JIT checked against interpreter, monitor/GDB, snapshots and automated guest runs | Planned |

FPU implementation can move earlier when firmware or a selected guest requires
it. Use instruction and device evidence to adjust ordering; a milestone label
does not justify faking successful hardware behavior.

Architectural trap entry and privileged CPU state now have a runnable guest
demonstration and focused regressions. Missing instructions remain host stops.
ASI dispatch and the initial SRMMU/reset/PROM harness now have translation,
fault-recovery and synthetic PROM regressions. Multiply/divide, tagged arithmetic,
MULScc, doubleword transfers, atomic exchanges and FLUSH now have focused tests.
NVRAM/IDPROM and the Z8530 polled console now have a synthetic reset-PROM demo,
device boundary tests and file persistence. Interrupt entry, source masking,
software requests and system/processor counter timers now have a guest demo
and deterministic timing tests. Original Sun OBP is the primary development
baseline; OpenBIOS remains a separate compatibility test until proved working.
System reset/status, guest software reset, watchdog status latching and both
SCC register blocks now support early OBP initialization. Processor user time
and doubleword access let it pass timed keyboard polling. The SBus/IOMMU
configuration subset now handles MID/arbitration setup at `0x10002000`.
Cache diagnostic storage and indexed invalidation now let OBP pass tag setup.
The hardware DRAM bank model now handles the earlier out-of-RAM probe, and
live TLB diagnostic entries plus replacement controls handle the following
initialization/tests. IOMMU page translation, selective I/O flushes, shared TLB
replacement and RAM DMA transfers now have focused tests. SBus timeout versus
slave-error classification, asynchronous CPU store capture and module interrupts,
and memory-request capture now have regressions. Original OBP completes
25374278 instructions with two diagnostic traps, no interrupts, no reset and no
serial output. It passes the earlier fault-status checks and stops at
PC `0x70002860`, instruction `0x91a189a7` (`FDIVs`). FPU state, transfers,
FSR, moves, comparisons, branches and deferred invalid/sequence exceptions
with queue draining are implemented. Implement IEEE arithmetic next, including
rounding and exception flags; the current ROM operation is single-precision division.
No firmware prompt or storage DMA engine is working yet.
Ordinary cache hits/fills, injected PTP walks and diagnostic M-bit rewalks
remain explicit gaps; diagnostic storage alone is not a complete cache model.
[Firmware records](firmware.md) capture the images and bounded probes.
Automatic partial CPU watchdog recovery, memory parity/error generation,
SBus late errors and doubleword store fault completion remain explicit gaps.

SunOS/Solaris, BSD and Linux versions will be recorded individually once tested.
No guest release is currently supported. Additional sun4c, sun4m and sun4u
machines should become explicit profiles; SPARC V9 is a separate CPU milestone.

