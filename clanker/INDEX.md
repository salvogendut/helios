# Agent notes

- [OpenBIOS startup optimization](openbios-startup-optimization.md) — native dictionary search and `-O2`, measured serial/graphics improvements and firmware/OS acceptance.
- [OpenBIOS startup cost profile](openbios-cost-profile.md) — full serial/CGsix phase measurements, guest instruction sampling, and the Forth dispatcher bottleneck.
- [OpenBIOS fallback](openbios-fallback.md) — shared firmware selection, isolated firmware services/NVRAM, source port, OS/PTY checks, blank-startup diagnosis/RAM-clear optimization and compatibility limits for issue #8.

- [External ICMP echo](icmp-echo.md) — bounded Linux ping relay, actual NetBSD/Solaris guest tests, host-policy diagnostics and explicit networking limits.

- [RTC startup date](rtc-startup.md) — stale year-2000 clock diagnosis, explicit UTC startup seeding, Heliacal saved-time option and NetBSD/Solaris checks.

- [NetBSD SX console fix](netbsd-sx-console.md) — reproduced cursor ROP stop, corrected word/byte ROPs and queue-empty status, graphical root login, regression checks and private-media evidence.

- [NetBSD 11.1 installation](netbsd-11-install.md) — SS-20 CHD installation, normal GENERIC root login, checksum persistence, floppy/CPU probe corrections and remaining timing/APC limits.

- [CHD implementation and acceptance](chd-images-progress.md) — persistent v5 hard disks, explicit image creation, Heliacal integration, Solaris write/reboot tests and NetBSD SS-20 installation/persistence.

- [CHD images plan and handoff](chd-images-plan.md) — issue #6: dependency/MSRV proof, media abstraction, durable CHD writes, blank creation, Heliacal integration and acceptance slices.

- [Clock, serial boot and CDE follow-up](clock-serial-cde-followup.md) — profile clocks, keyboard reset collision, recovered Axil disk and incompatible TurboGX+ ROM diagnosis.

- [Configurable RAM completion](ram-capacity-completion.md) — Axil firmware-compatible populations, SS-20 split 32 MiB bank fix, CGsix console configuration, tracked frontend lifecycle tests and final acceptance evidence.

- [Configurable RAM slice D interim review](ram-capacity-slice-d-interim-review.md) — completed maximum-size serial evidence checked; Axil 416→448 MiB discrepancy independently reproduced; SS-20/CGsix 512 MiB guest panic confirmed in UP and SMP captures; focused handover and remaining acceptance checks.

- [Configurable RAM slice C review](ram-capacity-slice-c-review.md) — frontend accepted for slice D; 42 tests, 6 independent fixtures, confirmed RAM replacement and SS-5 256 MiB display smoke pass; maximum-capacity Solaris and interactive acceptance remain.

- [Configurable RAM slice B review](ram-capacity-slice-b-review.md) — SEGA aliasing correction independently accepted; 620/622 unique tests, 7/7 review fixtures and standard checks pass; ready for Heliacal slice C, with maximum-capacity guest acceptance still pending.

- [Configurable RAM slice A review](ram-capacity-slice-a-review.md) — topology/construction accepted for slice B; 8 independent fixtures, 611/613 unique tests, enumeration-oracle and Axil-documentation follow-ups; temporary build cleaned.

- [Configurable RAM plan and handoff](ram-capacity-plan.md) — issue #3 work order for both repositories: shared topology, SS-20 VSIMM limits, high-RAM DMA, CLI/F9 settings, firmware acceptance and cleanup.

- [SMP slice 4 review](smp-slice4-review.md) — CLI fixes, 605/607 tests, SS-20 SMP input and Axil CDE lifecycle/persistence accepted; independent Axil move/resize/expose/iconify/focus assertions complete; experimental slice ready for incremental merge, temporary clones/builds cleaned.

- [SMP slice 4](smp-slice4.md) — original two-CPU SS-20/Axil OBP prompts, experimental Heliacal CPU selection, and the Solaris SMP work order.
- [Solaris SMP progress](smp-progress.md) — corrected CLI power-off/hot path, two-CPU OBP/Solaris work and persistence, UP regressions, 605/607 tests and 5/5 CLI fixtures; SS-20 input and Axil CDE lifecycle/window/focus checkpoints accepted for experimental integration.

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
- [User-mode NAT stage one](host-networking.md): ARP/RARP, DHCP, ICMP, UDP, DNS relay and TFTP ownership/subset decisions.
- [SPARCstation 20 and Axil 320](ss20-axil.md): high sun4m map, reference-MMU/IDPROM profiles, original-firmware `ok` bring-up and remaining POST diagnostics.
- [x-windows.md](x-windows.md) — Solaris 2.6 X rendering: indexed FBC vertices, polygon coverage, trace replay and live desktop evidence.
- [performance.md](performance.md) — MMU translation caches, device-time coalescing and frontend/SCSI throughput measurements.
- [Performance investigation and handoff](performance-plan.md) — measured idle execution, redundant CGsix/CG14 scanout, frontend redraw and MMU/IRQ/DMA costs; ordered optimization plan, correctness gates and artifact cleanup.
- [First performance slice review](performance-slice-review.md) — confirmed fixed-work benchmark errors, mismatched scanout measurement phases, 59 focused tests and the next frontend acceptance work.
- [Corrected performance and event-loop review](performance-event-loop-review.md) — three review findings resolved; 63 focused tests, benchmark boundaries and guest-visible OBP capture/release/focus-loss acceptance pass; first performance slice ready to commit.
- [MMU performance review](performance-mmu-review.md) — diagnostic cache behavior checked against the previous core; eight regressions promoted, per-phase counters corrected and final 514/516-test validation complete.
- [SS-20/Axil SMP implementation handoff](smp-plan.md) — issue #2: two-module topology, CPU/MMU/MXCC ownership, startup arbitration, interrupts/timers, shared time and DMA; ordered implementation slices and original-OBP/Solaris acceptance gates.
- [SMP slice 1](smp-slice1.md) — explicit MBus numbering, independent module state, per-MID MXCC routing, corrected word-stream and cross-module recursion paths, reviewed absent-port/width policy, 533/535 tests and fresh single-CPU SS-5/SS-20/Axil boot validation; review gate before scheduling.
- [SMP slice 2A](smp-slice2a.md) — processor-addressed interrupt/timer banks, Current Interrupt Target routing, directed software interrupts, independent level-15 acknowledgement and one shared timer clock; synthetic bank/routing regressions with UP boot preservation; review before scheduling or startup work.
- [SMP slice 2A review](smp-slice2a-review.md) — all four corrections independently accepted, including hardware-ID bounds and bus-error pulse coverage; 546/548 core tests, eight review fixtures, zero per-update allocations, Heliacal rebuild and fresh SS-5/SS-20/Axil boots.
- [SMP slice 2B assignment](smp-slice2b.md) — isolate SS-20/Axil I/O translation and flush ownership from CPU 0, preserve SS-5's integrated shared TLB, and return for review before scheduling or startup work.
- [SMP slice 2B implementation](smp-slice2b-implementation.md) — standalone 16-entry MBus IOMMU cache and flush owner, independent CPU transport state, stale/fresh flush regressions and single-CPU/disk persistence validation; review gate before scheduling.
- [SMP slice 2B review](smp-slice2b-review.md) — both profile-boundary corrections independently accepted at `4555aa2` and approved for incremental merge; eight review fixtures, 560/562 core tests, Heliacal tests/rebuild and verified submitted boot/persistence evidence.
- [SMP slice 2C assignment](smp-slice2c.md) — explicit CPU/DMA/debug initiators and profile-specific processor arbitration state; bounded board-state review before deterministic scheduling or two-CPU boot.
- [SMP slice 2C review](smp-slice2c-review.md) — independently accepted initiator/arbitration and profile-specific fault handling; 17 review fixtures, 578/580 core tests, Heliacal tests/rebuild and inspected final boot/persistence evidence.
- [SMP slice 2C implementation](smp-slice2c-implementation.md) — module-derived bus initiators, scoped CPU/DMA/host contexts, Sun-4M AER/MID registers, arbitration eligibility/run-state API, module-local reset and tracked regressions; no scheduler or two-CPU boot.
- [SMP slice 3 assignment](smp-slice3.md) — authoritative module storage, deterministic scheduling and one shared clock, CPU-tagged outcomes and bounded synthetic SMP validation before original-firmware two-CPU startup.
- [SMP slice 3 implementation](smp-slice3-implementation.md) — independently owned CPUs, deterministic per-CPU deadlines, shared device time, guest AER/IPI/TLB-shootdown protocols and whole/module reset scopes; accepted synthetic SMP milestone.
- [SMP slice 3 review](smp-slice3-review.md) — accepted after timing/reset corrections, including re-grant wait preservation and reset-store RTC accounting; 601/603 core tests, seven independent fixtures, Heliacal rebuild and matched Solaris UP execution with identical guest results.
- [Artifact cleanup before SMP](artifact-cleanup-2026-11-14.md) — obsolete review builds and scratch disks removed; historical traces compressed with checksum verification; accepted media, catalog and regression evidence retained.
- [SMP slice 1 review](smp-slice1-review.md) — both host-failure regressions corrected and independently accepted; 533/535 core tests, Heliacal rebuild, documented port policy and next interrupt/timer-bank assignment.
- [SS-20/Axil disk and CG14 review](ss20-axil-disk-cg14-review.md) — reproduced CPU identity boot blocker, native CG14 mapping failure and working prompt experiment, Heliacal selection, and remaining MMU/MXCC work.
- [Solaris first-panic follow-up](ss20-solaris-panic.md) — confirmed wrong PFN from the segment-probe shortcut, MXCC completion wait, tested patches, and the next page-list alignment fault.
- [Page-color MXCC alias](ss20-page-color-mxcc-alias.md) — physical MXCC control alias, shared ASI-2/physical state, userland checkpoint, and cloned-disk device migration.
- [MXCC production review](ss20-mxcc-production-review.md) — recursive stream-transfer safety, read-only port identity, architected MBus alias, and disk persistence checks.
- [SS-20/Axil page-color panic](ss20-page-color-mxcc-alias.md) — missing physical MXCC control alias hides cache parity from OBP, causing a zero Solaris cache size and out-of-bounds page-list access; tested alias proof reaches userland, then exposes the SS-5 disk's stale device paths.
- [MXCC production follow-up](ss20-mxcc-production-review.md) — four isolated regressions expose recursive stream borrow panics, writable port identity and the missing architected MBus port address at 652d5df.
- [Native CG14/Xsun review](ss20-sx-xsun-review.md) — SX/PLOT and display regressions, verified text/wrap/scroll, Solaris's mouse startup hold, corrected title-bar hit testing, and the captured packed-load/ROP/masked-store outline failure on a real olwm drag.
- [SX doubleword queue panic](ss20-sx-std-panic.md) — first-fault correction to the ESP report: paired IQ writes generate a guest bus trap, with CPU/MMU reproduction, fault-atomicity, packed-tail and mask-selected ROP regressions.
- [SS-5 TRCR investigation](ss5-trcr-investigation.md) — measured page-walk storms, documented WP=0 uncertainty and deliberately unchanged replacement semantics.
- [Performance progress](performance-progress.md) — measured scanout invalidation and interrupt caching wins; fixed-work benchmark, corrected boot evidence and remaining performance plan items.
- [SX funnel and move acceptance](ss20-sx-funnel-review.md) — funnel-left opcodes and aligned packed stores; calibrated TERM-A moves accepted at 20/1/2/3 pixels, with remaining resize and Heliacal checks.
- [Unaligned SX load review](ss20-sx-unaligned-load-review.md) — packed-load lane/ninth-register fix independently verified; 20/1/2/3-pixel moves preserve text, visible window frames and the background, with remaining desktop/frontend/boot acceptance checks.
- [SX resize and final-store review](ss20-sx-resize-review.md) — Run 21 rendering verified, shrink remainder explained by the surviving border, 491-test review and two isolated store-boundary failures, subsequently fixed in the completion follow-up.
- [SS-20 desktop completion](ss20-desktop-completion.md) — store-boundary and VSIMM-2 identity fixes, 499 core tests, repaired legacy frontend startup, fresh SS-5/Axil boots, and actual Heliacal input acceptance.
- [Issue #5 Slice A baseline](performance-issue5-slice-a.md) — corrected per-CPU phase/counter baseline, sampled profile, frontend idle evidence and the completed single-CPU comparison harness; no decoded cache or JIT.
- [Issue #5 Slice B differential validation](performance-issue5-slice-b.md) — full CPU/window/MMU/FPU snapshots plus scheduler, bus and device fingerprints; deliberate mismatch tests before any cache.

- [Issue #5 Slice C decoded cache](performance-issue5-slice-c.md) — bounded opt-in PC/raw-word decode cache with interpreter snapshot validation; disabled by default pending performance work.

- [Issue #5 SS-5 page-walk investigation](performance-issue5-mmu-walk.md) — per-slot fast-cache generations; unsafe leaf-only walk shortcut removed; matched benchmark within +0.11%, WP=0 bottleneck remains open.

- [SS-5 WP=0 investigation](ss5-wp0-investigation.md) — manual range semantics, OBP TRCR write trace, Solaris idle evidence and a WP=0 regression; replacement semantics unchanged.

- [Issue #5 device-service review](performance-issue5-device-service-review.md) — accepted held-reset IRQ correction, six promoted fixtures, matching boot evidence and a pinned SS-20 idle follow-up.

- [Issue #5 parking-token review](performance-issue5-idle-parking-token-review.md) — previous 24 fixtures pass; two remaining MMU-proof details have a tested patch passing 27/27 in isolation; apply it and promote fixtures before commit.
- [Issue #5 core parking token](performance-issue5-idle-parking-token.md) — uncommitted corrected prototype; readiness, deadline and configuration invalidation fixes validated, with remaining MMU-proof details recorded in the review.
- [Issue #5 foundation review](performance-issue5-idle-parking-foundation-review.md) — corrected foundation independently accepted; all ten promoted fixtures and 745/748 unique full-suite tests pass, with conditional loop proof and qualified performance evidence.
- [Issue #5 idle-parking foundation](performance-issue5-idle-parking-foundation.md) — accepted logical clock/deadline query, promoted review fixtures, Solaris idle-loop boundary trace, fresh frontend baseline and active-work comparison; ready to commit, no sleeps or recognizer.
- [Issue #5 idle-parking design review](performance-issue5-idle-parking-review.md) — eligibility, clock/deadline and wakeup corrections; PTY launch unblocked with 20 guest-verified transactions; time/deadline foundation is the next implementation slice.
- [Issue #5 idle-parking design handoff](performance-issue5-idle-parking-handoff.md) — evidence, wakeup inventory, proposed core/frontend API and required measurements; review corrections remain before parking implementation and issue #5 stays open.
- [Device-service readiness guards](performance-issue5-device-service.md) — ESP/Ethernet service predicates, matching guard-on/off boot outcomes and qualified local timing results.
