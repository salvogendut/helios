# Performance branch

This is the historical optimization record; those changes are now merged.
See [the 2026-11-13 investigation and handoff](performance-plan.md) for current
profiles, idle CPU findings and the next implementation plan.

Branch `performance` starts from `main` commit `619c80a` (the SS20 work remains
on `ss20-axil320`). `STEP_NS` is unchanged.

## MMU fast translation cache

`Srmmu` now keeps separate 256-entry direct-mapped instruction and data
translation caches, plus cached TLB diagnostic validation. Each cache entry
stores a full-page tag (virtual page, context, supervisor), the TLB slot, and
the TLB generation. Insertions, flushes and diagnostic TLB writes bump the
generation; TLB fills and tried translations are published only after
permission/R-M success, preserving the previous fault atomicity. Context and
supervisor changes are part of the tag, so they do not require a generation
bump. The reference/modified write-back still touches the real TLB entry.

The synthetic harness from `/tmp/helios-speed-review` (5M instructions, three
rounds) measured:

| Case | Before | After |
| --- | ---: | ---: |
| CPU, MMU enabled | 19.76 MIPS | 33.3 MIPS |
| Machine step, MMU enabled | 13.57 MIPS | 16.8 MIPS |
| Machine + CGsix | 12.65 MIPS | 15.1 MIPS |

The same-host Solaris 2.6 boot harness reached `The system is ready.` in
150.4 host seconds after the MMU cache, versus 165.5 seconds for the main
baseline measured in the same session.

## Device scheduling and SCSI batching

- Timer advancement is coalesced to 500 ns ticks, the idle mouse skips phase
  work, and the duplicate `sync_interrupts` in `interrupt_levels` is removed.
  DMA paths synchronize interrupt state after changing it.
- CPU-boundary device time is accumulated to 2 us (four timer ticks); direct
  `advance_ns` calls used by device tests remain exact, and observable NVRAM,
  timer and word-device reads flush pending device time first.
- `Machine::step` services up to sixteen pending ESP/DMA bytes per CPU
  boundary; `service_scsi` keeps its one-byte contract for direct callers.
  Address/count, arbitration, faults and terminal count still advance one byte
  at a time.
- Synthetic machine step with MMU improved from 16.8 to 21.1 MIPS; machine
  bypass improved to 28.4 MIPS.
- The same Solaris 2.6 boot harness reached `The system is ready.` in 92.4
  seconds / 1.145B steps, versus 112.1 seconds before the device-time/SCSI
  pass, 150.4 seconds after the MMU pass, and 165.5 seconds on the main
  baseline. The guest boot path changed as DMA/interrupt/device timing
  accelerated; it still completed normally.

The full test suite, including MMU permission, flush, PTE-walk, fault
atomicity and no-fault tests, passes. The next core target is a decoded
instruction cache; frontend batching and repaint reuse live in Heliacal.
