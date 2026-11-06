# Interrupt and timer implementation notes

Checked 2026-11-06. Human contract and choices are in docs/interrupts-timers.md.
Sources are pinned in docs/references.md. No firmware prompt or differential run.

User selected SS5/85: 85 MHz microSPARC-II, 128 MiB RAM, CGsix framebuffer,
SCSI disk/CD-ROM, Ethernet and serial ports. Default RAM is now 128 MiB and
CPU_CLOCK_HZ records 85 MHz, separate from STEP_NS. CGsix is planned as a
1 MiB TurboGX SBus card; do not reuse QEMU's TCX map. See docs/sparcstation5.md.

- NCR89C105 is the uniprocessor chip, not generic SMP sun4m. CPU bank at
  `0x71e00000`, system at `+0x10000`; timer banks are at `0x71d00000` and
  `+0x10000`. System timer uses source bit 19/PIL10; CPU timer is local PIL14.
- NCR table 6-25/figures 6-14..18 specify 22-bit counters in bits 30:9, 500 ns
  ticks, reload 1 on match, read-limit acknowledgement, non-resetting limit
  writes, zeroed reset, and overflow interrupts even with limit zero. QEMU's
  no-interrupt free-run differs. The later user-timer.md note covers implemented
  mode bit 0, 54-bit counting and doubleword access.
- Match-after-increment and pending clear on resetting limit write are recorded
  choices. The scalar oracle tests wrap and lower-limit cases. Large advances
  skip periods; do not introduce per-tick loops for u64::MAX time arguments.
- NCR's Mask All wording is interpreted to include software requests, unlike
  QEMU. This requires hardware confirmation. Pending display is unmasked;
  masks never erase level-held sources. Asynchronous hardware-error/IC latches
  are pending; do not claim hardware level-15 error-source support from soft15.
- CPU samples before fetch, chooses highest eligible level, requires ET even
  for 15, preserves PIL, ignores WIM on entry, and saves the current PC/nPC.
  InterruptTaken is distinct from synchronous TrapTaken. Cpu::step has no lines;
  Machine::step supplies them and advances 100 ns only after successful steps.
- Time belongs to Machine/SystemBus. Standalone Cpu and devices remain testable.
  Reset retains RTC fractional time but clears timer phase/controller state.
  Device MMIO is word-sized here, unlike byte serial/NVRAM. Wider/atomic/partial
  writes must fail before a timer-limit read acknowledges anything.
- New demo: examples/timer.s, timer_demo_image(), timer-demo [--trace]. Both
  sources fire once within 256 steps, with guest read acknowledgements and
  JMPL/RETT return to work. CI runs all four demos.
- Disposable references: /tmp/helios-qemu-slavio-intctl.c,
  /tmp/helios-qemu-slavio-timer.c, /tmp/helios-ncr89c105.txt,
  /tmp/helios-sparcclassic.pdf and .txt. Bitsavers main host returned 403; its
  trailing-edge.com mirror worked. MAME Sun timers inspected are sun4/sun4c.
- User supplied original firmware archive https://github.com/andarazoroflove/sparc
  and requires original Sun OBP as the primary development/accuracy target until
  OpenBIOS is proved working. Pinned images and hashes are in docs/firmware.md.
- Both images fetched into ignored local/firmware. Sun ss5.bin completes four
  instructions then reads unmapped system reset/status 0x71f00000 at PC 0xbda0;
  ET=0 latches CPU error state. This initial stop is superseded by the
  reset/status implementation; see reset-bringup.md for the current timer stop.
  OpenBIOS completes four then stops on ASI 0x2d/STHA at PC 0x5f0c, QEMU fw_cfg.
  Do not change the hardware ASI model to accommodate it during OBP bring-up.
- Retain hashes/configuration/transcripts/stops; SCC interrupt generation remains
  pending. No firmware prompt is reached by either probe.
