# Development milestones

The first full-system target is SPARCstation 5 / sun4m with an 85 MHz microSPARC-II,
128 MiB RAM, CGsix graphics, SCSI disk/CD-ROM, Ethernet and TTYA/TTYB. Each milestone needs
observable acceptance criteria before its status can advance.

| Milestone | Deliverable and acceptance | Status |
| --- | --- | --- |
| 0 — Foundation | Library/CLI, integer subset, word bus, runnable bare-metal demo and regression tests | Implemented |
| 1 — CPU baseline | Complete V8 integer/privileged behavior, PSR/WIM/TBR/Y, precise traps/RETT, ASI dispatch, multiply/divide and atomics; independently checked bare-metal tests | In progress: core integer instruction groups, privileged state, synchronous traps, ASIs, doublewords and atomics implemented; implementation-specific ASRs and broader illegal-opcode classification pending |
| 2 — Memory system | microSPARC-II reset/boot behavior and SRMMU, permissions, contexts, faults, TLB invalidation and required cache controls; translation/fault tests | In progress: translation, TLB flush/probe and cache diagnostic storage/invalidation implemented; DRAM banks and live TLB diagnostics implemented; injected PTP walks, M-bit rewalks and cache hits/fills pending |
| 3 — Serial firmware | Original Sun OBP loading, NVRAM/IDPROM, Z8530, interrupt controller and timers; reproducible OBP prompt transcript | Device enumeration and 128 MiB RAM test complete; timer diagnostic and Ethernet loopback pass; serial BREAK returns from RARP retry to verified prompt/Forth response; parallel-port POST fault remains |
| 4 — Storage and kernel | IOMMU, DMA, ESP SCSI, disk/CD-ROM targets and safe image writes; installer/kernel reaches serial userspace | In progress: Solaris 2.6 and 7 reach the serial installer. ESP/DMA2 transfers, opt-in durable raw disk writes, CD modes/media locking, SCC interrupts and DMA setup registers implemented; broader commands and installation validation remain pending |
| 5 — Usable workstation | Required FPU behavior, LANCE networking, CGsix/TurboGX framebuffer, keyboard/mouse and host frontend; documented guest compatibility matrix | FPU state/transfers/comparisons/division/addition/subtraction/multiplication/integer-single-double conversions/branches and deferred IEEE exceptions implemented; other arithmetic and remaining devices pending |
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
and memory-request capture now have regressions. Original OBP exercises
single/double arithmetic and deferred exception recovery, then software interrupt
levels 1–15. Documented PROM service time, skip-zero counter overflow and
limit-flag acknowledgement now let its processor-timer diagnostic pass. MACIO
SCSI register tests, Ethernet initialization and internal loopback complete.
The remaining POST fault is a read of parallel-port DMA CSR `0x7c800000`.
OBP continues through enumeration and the full 128 MiB memory test, transmits
RARP requests and retries when no server replies. A decoded serial BREAK
returns to `ok`; a Forth command returns `3` and another prompt. Clean POST,
The CGsix/keyboard console and SCSI target-discovery milestones are complete.
Installed-system boot, broader SCSI commands and a host network backend remain unfinished.
Widening multiplication, square roots, quad conversions
and quad/unfinished FPU traps also remain pending.
Ordinary cache hits/fills, injected PTP walks and diagnostic M-bit rewalks
remain explicit gaps; diagnostic storage alone is not a complete cache model.
[Firmware records](firmware.md) capture the images and bounded probes.
Automatic partial CPU watchdog recovery, memory parity/error generation,
SBus late errors and doubleword store fault completion remain explicit gaps.

SunOS/Solaris, BSD and Linux versions will be recorded individually once tested.
No guest release is currently supported. Additional sun4c, sun4m and sun4u
machines should become explicit profiles; SPARC V9 is a separate CPU milestone.

## Ultra 5 serial console

**Original OBP 3.31.0 executes its expanded firmware from RAM on 22 November 2026.**
Cold and soft-reset runs reach the same entry with diagnostics enabled or disabled.

| Milestone | Evidence |
| --- | --- |
| First serial register access | At virtual PC `0xffff_ffff_f000_033c`, the guest stores one byte, `0xc3`, at physical `0x01ff_f140_002c` (channel A CCR0). Both tested reset-marker paths reach this write. |
| First firmware text | Serial A emits `Hardware Power ON` on the cold path and `Software Power ON` on the matching-marker path. Each ends in CR/LF. With the diagnostic byte clear, these writes to the transmit FIFO are skipped. |
| DRAM controller setup | All four marker/diagnostic combinations turn on the front-panel LED, program both MCU registers, complete both polling delays and reach external-cache sizing. |
| External-cache initialization and build identification | All four runs detect 2 MiB, select ELIM 3 and clear 32,768 tag entries. Both diagnostic-enabled runs print the original firmware build string. |
| Cold-start keyboard probe | The unplugged keyboard probe completes three timed waits and one reset-byte transmission. With diagnostics enabled, serial A prints `Probing keyboard Done`. |
| Instruction and data TLB clear | On both paths the firmware clears all 64 slots in each bank, returns to its caller and reaches the ASI `0x14` read at physical address zero. |
| Physical DRAM discovery | OBP detects the two 256 MiB DIMMs in pair 2, selects 11-bit columns, reports the upper 256 MiB rank at `0x30000000` and clears the top 1 MiB. |
| MMU enable and relocation | The original ROM copies 256 KiB to PA `0x3ff00000`, branches to VA `0x2038` backed by RAM and starts its decompressor. Both diagnostic runs print the flash and RAM PCs. |
| Decompression and entry | OBP expands 453,536 bytes at VA `0xf0000000` and executes the entry at VA `0xf0051f8c`. A local-ROM test compares every byte against an independent decoder. |
| Serial initialization in the expanded image | The guest clears both on-chip cache tag banks and prints `ttya initialized` with diagnostics on. Both reset paths then enter `fprs!`. |
| Interactive OpenBoot console | Open. Acceptance is a serial `ok` prompt that takes `1 2 + .`, replies `3` and prompts again. |

The next instruction is a write of `4` to the floating-point registers state
register (FPRS) at VA `0xf0023944`. [Reproduce the current checkpoint](ultra5.md#executing-the-expanded-firmware)
