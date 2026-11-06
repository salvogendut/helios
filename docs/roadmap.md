# Development milestones

The first full-system target is SPARCstation 5 / sun4m. Each milestone needs
observable acceptance criteria before its status can advance.

| Milestone | Deliverable and acceptance | Status |
| --- | --- | --- |
| 0 — Foundation | Library/CLI, integer subset, word bus, runnable bare-metal demo and regression tests | Implemented |
| 1 — CPU baseline | Complete V8 integer/privileged behavior, PSR/WIM/TBR/Y, precise traps/RETT, ASI dispatch, multiply/divide and atomics; independently checked bare-metal tests | In progress: state registers, synchronous traps/RETT and byte/halfword/word ASI loads/stores implemented |
| 2 — Memory system | microSPARC-II reset/boot behavior and SRMMU, permissions, contexts, faults, TLB invalidation and required cache controls; translation/fault tests | In progress: reset/PROM, hardware page walks, permissions, contexts, R/M, fault state and TLB flush/probe implemented; diagnostics pending |
| 3 — Serial firmware | PROM loading, NVRAM/IDPROM, Z8530, interrupt controller and timers, plus observed firmware probes; reproducible firmware prompt transcript | Planned |
| 4 — Storage and kernel | IOMMU, DMA, ESP SCSI, disk/CD-ROM targets and safe image writes; installer/kernel reaches serial userspace | Planned |
| 5 — Usable workstation | Required FPU behavior, LANCE networking, TCX framebuffer, keyboard/mouse and host frontend; documented guest compatibility matrix | Planned |
| 6 — Performance and tooling | Profiling-driven optimization, optional JIT checked against interpreter, monitor/GDB, snapshots and automated guest runs | Planned |

FPU implementation can move earlier when firmware or a selected guest requires
it. Use instruction and device evidence to adjust ordering; a milestone label
does not justify faking successful hardware behavior.

Architectural trap entry and privileged CPU state now have a runnable guest
demonstration and focused regressions. Missing instructions remain host stops.
ASI dispatch and the initial SRMMU/reset/PROM harness now have translation,
fault-recovery and synthetic PROM regressions. Next complete the remaining
integer operations needed by firmware. External interrupt delivery is a separate
task; storing PSR.PIL does not yet provide it.

SunOS/Solaris, BSD and Linux versions will be recorded individually once tested.
No guest release is currently supported. Additional sun4c, sun4m and sun4u
machines should become explicit profiles; SPARC V9 is a separate CPU milestone.

