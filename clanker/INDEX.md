# Agent notes

- [build-environment.md](build-environment.md) — howto: reuse the local Prismatic Rust toolchain.
- [trap-state.md](trap-state.md) — rule: follow V8 state-register and error-mode rules when references differ.
- [memory-system.md](memory-system.md) — reference: microSPARC-II ASI/SRMMU rules and QEMU differences.
- [integer-operations.md](integer-operations.md) — rule: microSPARC-II division overflow and indivisible memory operations.
- [device-state.md](device-state.md) — reference: SCC shared pointer, RTC latches and retryable device stops.
- [interrupts-timers.md](interrupts-timers.md) — reference: NCR89C105 counters, CPU IRQ entry and deterministic time.
- [reset-bringup.md](reset-bringup.md) — reference: reset/status, both SCCs and OBP's next timer-mode stop.
- [user-timer.md](user-timer.md) — reference: 64-bit timer access and OBP's SBus control/TLB diagnostic gaps.
- [io-control.md](io-control.md) — reference: microSPARC-II arbitration/slot masks and OBP's cache-tag stop.
- [cache-diagnostics.md](cache-diagnostics.md) — reference: cache diagnostic arrays/flushes and OBP's memory-probe fault.
- [dram-tlb.md](dram-tlb.md) — reference: hardware DRAM layout, live TLB diagnostics and the earlier IOMMU-enable stop.
- [iommu-dma.md](iommu-dma.md) — reference: shared I/O translation/flushes, DMA errors and OBP's first serial diagnostic.
- [fault-capture.md](fault-capture.md) — reference: SBus TO/BE, CPU store capture, NCR module IRQ and the next OBP FPU boundary.
- [fpu-transfers.md](fpu-transfers.md) — reference: FPU state, FSR/transfer/trap choices and the next OBP comparison boundary.
- [fpu-compare.md](fpu-compare.md) — reference: IEEE comparisons, deferred invalid/sequence exceptions, queue recovery and the historical OBP division stop.
- [fpu-division.md](fpu-division.md) — reference: exact FDIVs/d rounding, five IEEE exceptions, Meiko NaNs, independent oracle and the historical OBP addition stop.
- [fpu-addition.md](fpu-addition.md) — reference: guard/round/sticky arithmetic, cancellation, signed zeros, overflow boundaries and the historical OBP multiplication stop.
- [fpu-multiplication.md](fpu-multiplication.md) — reference: exact products, full arithmetic POST coverage, processor-timer diagnostic and breakpoint-register stop.

- [chip-revision.md](chip-revision.md) — reference: mask identity, comparison configuration and continuing OBP initialization.
- [auxiliary-port.md](auxiliary-port.md) — reference: NCR auxiliary pin state, terminal-count events and original-ROM initialization.
- [floppy-serial-prompt.md](floppy-serial-prompt.md) — milestone: NCR82077 subset and verified original-OBP serial `ok` with Forth response; requested stopping point reached.
- [afx-presence.md](afx-presence.md) — reference: board AFX-presence byte and the subsequent MACIO DMA control-register fault.

- [MACIO DMA2 registers](macio-dma.md): SCSI DMA configuration, primary manual corrections and next ESP access.

- [ESP initialization and RAM test](esp-initialization.md): completed enumeration, full original-OBP memory diagnostic and next Ethernet DMA access.
- [Timer and Ethernet](timer-ethernet.md): corrected PROM/counter timing, real Ethernet initialization/loopback/RARP, chosen Sun IDPROM and serial BREAK return to `ok`; next POST gap is parallel DMA.
- [Graphical console](graphics-console.md): original TurboGX FCode, FHC/THC initialization, drawing endpoints, Type 5 keyboard and native window validation.
- [SCSI discovery](scsi-discovery.md): ESP/DMA2 transfers, read-only raw disk/CD-ROM images and original OBP `probe-scsi-all` listing both targets before returning to `ok`; requested stopping point reached.
- [Solaris CD bootloader](cdrom-bootloader.md): pushed discovery milestone, CD mode/data-out support, and Solaris 2.6/7 first bootloader output; requested stopping point reached.
- [Solaris CD installer](solaris-installer.md): original-OBP CD boot reaches both Solaris 2.6/7 serial language-selection screens; CD locking, SCC interrupts, Ethernet diagnostic status and DMA2 aliases.
- [Writable disk installation](solaris-install-rw.md): explicit writable raw disks, durable SCSI writes and the ongoing Solaris 7 installation attempt.
- [Installed disk boot](installed-disk-boot.md): Solaris 7/2.6 installed-disk boots, SELATNS and CS4231 fixes, and the SS10/20 ASI 47 report.
- [Loopback telnet serial](serial-tcp.md): IRIS-style nonblocking TCP serial consoles, CLI port flags and telnet filtering.
