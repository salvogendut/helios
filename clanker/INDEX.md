# Agent notes

- [Combined sun4m performance on Solaris](sun4m-integrated-performance.md) compares the four interpreter changes with the previous main and profiles small SS-20 disk reads.

- [sun4m interrupt and timer bank lookup](sun4m-processor-bank-performance.md) records the Solaris and SCSI comparisons for direct processor-bank indexing.

- [SS-5 sparse TLB lookup](ss5-tlb-search.md) records the Solaris boot and disk-read comparison, locked-slot tests and dense-table controls.

- [SS-5, SS-20 and Axil RAM access performance](sun4m-dram-performance.md) covers direct DSIMM lookup, Solaris kernel and filesystem comparisons, and the ESP/DMA control.

- [Ultra 5 performance compared with QEMU](ultra5-performance-analysis.md) covers the SPARC loop comparison, Solaris kernel profiles, and Xsun boot, file-copy and IDE DMA measurements for the TLB, board polling and sparse RAM changes.

- [Ultra 5 graphical console and Solaris 7](ultra5-graphical-console.md) — original OpenBoot, interactive CD, dedicated IDE disk, Mach64 FIFO status, the CDE desktop and the Solaris VIS framebuffer block-store fix.

- [Ultra 5 ATAPI CD-ROM and Solaris 7 boot](ultra5-atapi-cdrom.md) — original OpenBoot device identification, complete sector comparison and the reproducible LDSB stop.

- [Ultra 5 MMU refill and PCI setup](ultra5-mmu-refill.md) — original OBP trap handling, PCI and IOMMU setup, CPU timer calibration, PCIO configuration, RTC polling, error acknowledgements, cache-backed DRAM refresh, PCI enumeration, the normal banner and the verified interactive serial monitor.

- [Ultra 5 firmware decompression](ultra5-decompression.md) — independent expanded-image comparison, both primary cache tag clears and the original ROM’s `ttya initialized` output.

- [Ultra 5 MMU enable and relocated execution](ultra5-mmu-enable.md) — first translated fetch, full ROM-copy comparison and decompressor entry.

- [Ultra 5 physical DRAM discovery](ultra5-dram-discovery.md) — original-ROM capacity descriptor, selected boot region, workspace clearing and MMU-enable boundary.

- [Ultra 5 TLB clear and memory probe entry](ultra5-tlb-init.md) — 128 seeded slot clears and the first DRAM access on both reset paths.
- [Ultra 5 unplugged keyboard probe](ultra5-keyboard-timeout.md) — APB tick, polled transmit timing, three original-ROM timeouts and the shared cold/soft MMU boundary.
- [Ultra 5 cold-start keyboard UART](ultra5-keyboard-uart.md) — original UART writes and the APB timeout boundary.
- [Ultra 5 E-cache and firmware banner](ultra5-ecache-banner.md) — size-probe readback, tag verification, QEMU comparison and the two post-banner paths.
- [Ultra 5 DRAM controller setup](ultra5-auxio-mcu.md) — LED and MCU register evidence, QEMU comparison and first E-cache probe operands.
- [Ultra 5 first serial text](ultra5-first-serial-text.md) — original OBP cold and soft-marker diagnostics, QEMU comparison and the next EBus write.
- [Ultra 5/10 interrupt and window initialization](ultra5-reset-window-state.md) — ten original firmware writes, register readback, and the first trap-level write.
- [Ultra 5/10 reset MMU contexts](ultra5-reset-contexts.md) — ASI selection, two independent context clears and the first PIL write.
- [Ultra 5/10 NVRAM reset decision](ultra5-reset-nvram.md) — retained SRAM, three marker comparisons and the next ASI write.
- [Ultra 5/10 reset data read](ultra5-reset-probe.md) — seventeen original OBP instructions and the first unserved board request.
- [Ultra 5/10 window selection](ultra5-window-select.md) — six original OBP instructions and QEMU checks for the overlapping window registers.
- [Ultra 5/10 current window pointer](ultra5-reset-cwp.md) — first instruction at the branch target and the four-step QEMU comparison.
- [Ultra 5/10 reset branch](ultra5-reset-branch.md) — first OBP control transfer, annulled delay slot and three-step QEMU comparison.
- [Ultra 5/10 second instruction](ultra5-second-instruction.md) — LSU store, seeded-register regression and two-step QEMU comparison.
- [Ultra 5/10 first instruction](ultra5-first-instruction.md) — single-step of OBP's reset-time LSU control read and the QEMU comparison.
- [Ultra 5/10 first firmware fetch](ultra5-first-fetch.md) — corrected vendor image, CPU and flash addresses, and combined V9 validation.
- [Ultra 5/10 first-fetch review](ultra5-first-fetch-review.md) — findings on the original submission and independent acceptance fixtures.

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
- [Host networking](host-networking.md) covers the initial NAT implementation and the Ultra 5 OpenBoot RARP/TFTP regression.
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

- [Issue #5 frontend parking review](performance-issue5-idle-parking-frontend-review.md) — committed default-off checkpoint accepted; 114 tests and 14 independent fixtures; matching 99.746%→23.586% worker CPU evidence verified, latency scope recorded, next work order is cost attribution before tuning.
- [Issue #5 idle-parking profile](performance-issue5-idle-parking-profile.md) — uncommitted worker timing study; 74.2% CPU reduction verified, with overhead/phase/latency attribution corrections recorded in the review.
- [Issue #5 profile isolation](performance-issue5-idle-parking-profile-isolation.md) — accepted default-off `parking-profile` feature checkpoint `9b21916`; twenty-pair arithmetic verified, reproduced release binaries, 114 tests per build and 14/17 review fixtures pass; ordinary-build slowdown resolved.
- [Issue #5 token sub-profile](performance-issue5-token-profile.md) — accepted profiling checkpoint `21c9d2e`/`8e8d82a`; guard order restored and nine fixtures promoted; device digest is 93.4–93.8% of token+advance elapsed across pre/post idle; fixed-work timer overhead undetected; digest encoding is the narrow proposal.
- [Issue #5 typed device digest](performance-issue5-typed-digest.md) — accepted checkpoint `939a7bc`; allocation-free typed device encoders with Debug reference and mutation/token-invalidation tests; fixed core token work -89.7%; frontend idle/command variance reported separately.
- [Issue #5 typed-digest review](performance-issue5-typed-digest-review.md) — accepted independent review: 142 field mutations, payload-only token checks, 89.72% fixed-work reduction; unchanged ESP/SCSI formatting and the unexplained later idle CPU increase are retained as explicit limits.
- [Issue #5 later-idle investigation](performance-issue5-later-idle-investigation.md) — exact trace identifies CWP 7 as the pre-candidate exclusion; both core versions exhibit it; common-bin work and serial delivery sequences checked, experiment-specific endpoints corrected.
- [Issue #5 CWP proof handoff](performance-issue5-cwp-proof-handoff.md) — accepted synthetic all-window invariance and real CWP-0/7 instruction/state pairs; logical versus applied device time and hash-attribution limits recorded; proof-only checkpoint precedes the admission change.
- [Issue #5 CWP admission handoff](performance-issue5-cwp-admission-handoff.md) — accepted default-off checkpoint committed as Helios `3a1cf6f` / Heliacal `e8e2cfc`: idle CPU 24/43% to 8/8%, command medians +6/+11 ms; latency remains a follow-up and #5 stays open.
- [Issue #5 CWP proof review](performance-issue5-cwp-proof-review.md) — accepted after 13/13 focused tests per build and independent real-pair audit; all 14 words and CPU/MMU/RAM equality verified; timed device hash is not a field-level comparison.
- [Issue #5 CWP admission review](performance-issue5-cwp-admission-review.md) — prior binary mismatch resolved; corrected raw metrics/hashes and isolated default/profile CWP-0/7 wait/input/BREAK tests verified; accepted for an opt-in checkpoint with explicit latency cost.
- [Issue #5 parking latency and input-grace review](performance-issue5-parking-latency-handoff.md) — 40 ms default-off grace accepted and integrated; repeated ~12 ms response improvement, no-echo recovery and settled-idle savings verified; tracing parked as reproducible local patches, production trees clean at Helios `2358935` / Heliacal `682d85b`.
- [Issue #5 token sub-profile review](performance-issue5-token-profile-review.md) — accepted profiling checkpoint; guard order and helper comparisons corrected, nine fixtures promoted, benchmark independently rebuilt; commit before the separate typed-digest optimization.
- [Issue #5 typed-digest review](performance-issue5-typed-digest-review.md) — accepted; 142 field mutations and payload-only token changes covered, independent fixed-work reduction 89.72%, complete validation and remaining frontend/allocation limits recorded.
- [Issue #5 idle-parking profile review](performance-issue5-idle-parking-profile-review.md) — historical overhead/phase findings and next measurement work order; the +4.19% ordinary-build regression is resolved by accepted compile-time isolation, with both idle regimes retained.
- [Issue #5 parking-token review](performance-issue5-idle-parking-token-review.md) — final MMU-proof patch applied and all 27 fixtures promoted in accepted core checkpoint e3d87e3; 781/784 unique core tests pass.
- [Issue #5 opt-in Heliacal parking](performance-issue5-idle-parking-frontend.md) — committed default-off checkpoint `a88d769`; bounded wait/recheck/commit, console drain, fresh matched off/on Solaris CPU/PTY measurements; issue #5 open.
- [Issue #5 core parking token](performance-issue5-idle-parking-token.md) — accepted checkpoint e3d87e3; conditional proof, readiness, deadline and configuration invalidation validated; real Solaris admission confirmed; frontend trial reviewed separately.
- [Issue #5 foundation review](performance-issue5-idle-parking-foundation-review.md) — corrected foundation independently accepted; all ten promoted fixtures and 745/748 unique full-suite tests pass, with conditional loop proof and qualified performance evidence.
- [Issue #5 idle-parking foundation](performance-issue5-idle-parking-foundation.md) — accepted checkpoint 2f903cd; logical clock/deadline query, promoted review fixtures, Solaris idle-loop boundary trace, fresh frontend baseline and active-work comparison; no sleeps or recognizer.
- [Issue #5 idle-parking design review](performance-issue5-idle-parking-review.md) — eligibility, clock/deadline and wakeup corrections; PTY launch unblocked with 20 guest-verified transactions; time/deadline foundation is the next implementation slice.
- [Issue #5 idle-parking design handoff](performance-issue5-idle-parking-handoff.md) — evidence, wakeup inventory, proposed core/frontend API and required measurements; review corrections remain before parking implementation and issue #5 stays open.
- [Device-service readiness guards](performance-issue5-device-service.md) — ESP/Ethernet service predicates, matching guard-on/off boot outcomes and qualified local timing results.

- [Safe JIT prototype design](jit-prototype-design.md) — optional default-off straight-line SETHI/ALU block engine with MMU validation, conservative invalidation and interpreter fallback.
- [Native JIT prototype result](jit-prototype-results.md) — guard-patched x86-64 candidate, default-off and Unix-only; ~1.18x synthetic ALU, ~1.1% memory fallback cost; review edge fixtures promoted.
- [Issue #5 execution prototype review](jit-prototype-review.md) — merge blocked: decoded-block backend emits no native code; six independent failures reproduce speculative MMU effects, split-run delay-slot corruption and silent reset/error stops; bounded native-backend follow-up and retained fixtures.
- [Issue #5 native backend review](jit-native-review.md) — real native emitter and prior fixes verified; three boundary bugs reproduced, conservative patch passes 34 focused tests and retains a measured ALU benefit; production branch unchanged.
- [SPARC JIT reference comparison](jit-reference-comparison.md) — pinned QEMU, InfiniteSPARCjit and UltraViolet sources: block state/budgets, address-width handling, code invalidation, register caching and differential validation; bounded recommendations for the current milestone and later work.
