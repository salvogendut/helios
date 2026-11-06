# Development milestones

The first full-system target is SPARCstation 5 / sun4m with an 85 MHz microSPARC-II,
128 MiB RAM, CGsix graphics, SCSI disk/CD-ROM, Ethernet and TTYA/TTYB. Each milestone needs
observable acceptance criteria before its status can advance.

| Milestone | Deliverable and acceptance | Status |
| --- | --- | --- |
| 0 — Foundation | Library/CLI, integer subset, word bus, runnable bare-metal demo and regression tests | Implemented |
| 1 — CPU baseline | Complete V8 integer/privileged behavior, PSR/WIM/TBR/Y, precise traps/RETT, ASI dispatch, multiply/divide and atomics; independently checked bare-metal tests | In progress: core integer instruction groups, privileged state, synchronous traps, ASIs, doublewords and atomics implemented; implementation-specific ASRs and broader illegal-opcode classification pending |
| 2 — Memory system | microSPARC-II reset/boot behavior and SRMMU, permissions, contexts, faults, TLB invalidation and required cache controls; translation/fault tests | In progress: translation, TLB flush/probe and cache diagnostic storage/invalidation implemented; TLB diagnostics, cache hits/fills and memory-bank behavior pending |
| 3 — Serial firmware | Original Sun OBP loading, NVRAM/IDPROM, Z8530, interrupt controller and timers; reproducible OBP prompt transcript | In progress: OBP passes cache-tag initialization, then faults during memory probing; its reset path stops on ASI 0x06 TLB diagnostics; working prompt pending |
| 4 — Storage and kernel | IOMMU, DMA, ESP SCSI, disk/CD-ROM targets and safe image writes; installer/kernel reaches serial userspace | Planned |
| 5 — Usable workstation | Required FPU behavior, LANCE networking, CGsix/TurboGX framebuffer, keyboard/mouse and host frontend; documented guest compatibility matrix | Planned |
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
The next first fault is a store at `0x08000000`, beyond the configured 128 MiB,
at PC `0x700075f8` during memory probing. Investigate the SS-5 memory-bank map
and unpopulated-bank/error behavior before changing RAM size or fault handling.
The subsequent diagnostic/reset path still stops on ASI `0x06` TLB diagnostics.
Ordinary cache hits/fills remain pending; diagnostic storage alone is not a
complete cache model or proof that cache POST works.
IOMMU DVMA translation and its fault/flush machinery are still pending.
[Firmware records](firmware.md) capture the images and bounded probes.
Automatic partial CPU watchdog recovery and asynchronous hardware-error latches
remain explicit gaps.

SunOS/Solaris, BSD and Linux versions will be recorded individually once tested.
No guest release is currently supported. Additional sun4c, sun4m and sun4u
machines should become explicit profiles; SPARC V9 is a separate CPU milestone.

