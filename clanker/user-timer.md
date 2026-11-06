# Processor user timer and longer original-OBP probe

Checked 2026-11-06. Contract: docs/interrupts-timers.md. The NCR89C105
specification's SBus feature subset and figures 6-16–6-18 define the implemented
mode. Compared with pinned QEMU slavio_timer.c; MAME's Sun timer model is sun4c.

- 54 count bits at 62:9, sticky overflow bit 63, low nine bits zero, 500 ns ticks.
  Count wraps to zero on overflow; no processor IRQ. Any write to count clears
  overflow. Word writes preserve the other half. Run gates only user counting.
- The system counter continues independently; both use the existing shared tick
  phase. Start/stop, count writes and mode changes preserve that phase. Reset
  clears it. Large time advancement is bounded using u128 tick arithmetic.
- NCR explicitly permits two-word bursts only at the user count in user mode.
  SystemBus's only doubleword device exception is CPU_TIMER_BASE with this mode
  selected. Reject other widths/addresses/modes before effects; preserve MMU
  permission checks and atomic [MSW, LSW] transfers.
- Mode-transfer behavior is not specified by the chip text. Current documented
  choice: changed mode clears count/overflow/processor pending, retains Run and
  the counter-mode limit latch. Same-mode writes are inert. +8 still changes
  that limit latch in user mode. These choices need hardware confirmation;
  OBP explicitly initializes both count halves and Run, so its progress cannot
  validate transition contents. QEMU resets user count on entry but differs at
  other transitions and in allowing counter-mode doubleword MMIO.
- Eight user-timer regressions cover carry/overflow, start/stop phase, huge time,
  mode switches with pending IRQs, invalid accesses, guest LDDA/STDA, read-only
  user mappings and reset. Existing counter and four demo checks are retained.
- 10000-step original-OBP probe now reaches budget while polling keyboard SCC.
  Longer 30M-step budget completes timed waits and first traps at step 20000419:
  PC=70001004, nPC=70001008, instruction=c0a04400, ASI20 store zero to 10002000.
  This is microSPARC-II 5.7.10 MID/SBAE, in the SBus/IOMMU control space, NOT
  a timer failure. QEMU calls it IOMMU_ARBEN, offset 0x2000.
- OBP emits a synchronous-trap diagnostic, requests software reset at step
  20035524, then stops at step 20035688: PC=be7c, nPC=be74, instruction=c0a200c0,
  unsupported ASI06 word store. This is TLB diagnostic access (microSPARC-II
  section 5.14.1 and ASI table), not a generic cache flush to be ignored.
  Totals: 20035686 instructions including reset store, one trap, no IRQs, one
  software reset; error latch clear. Next: implement SBus/IOMMU control behavior
  for the FIRST fault, then follow the resulting firmware path.
- local/probes/user-timer-ss5 holds CLI output and an ignored Rust library probe
  source/log with first 500 steps, per-million timer snapshots, exceptional
  outcomes and 128-step context. It reproduces the CLI's console/stop without
  saving tens of millions of trace lines. Firmware hash unchanged; no prompt.
- New reference downloaded: /tmp/helios-qemu-sun4m-iommu.c at the existing pinned
  QEMU revision. Full register/DMA review remains pending. microSPARC-II 5.7.10
  calls SBAE six bits but lists overlapping reserved bits; QEMU masks five.
  Resolve and document that discrepancy during the next hardware review.
- Local nightly Cargo splits .rlib metadata into a sibling .rmeta; a standalone
  rustc probe needs both --extern helios=...rlib and --extern helios=...rmeta.
  Do not confuse a metadata-stub error with a Helios compilation failure.
