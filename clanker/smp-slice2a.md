# SMP slice 2A: per-processor interrupt and timer banks

Implemented on `feat/sun4m-smp` from accepted slice-1 commit `2bed9ff`.
Independently accepted after the corrections and final review recorded in
[smp-slice2a-review.md](smp-slice2a-review.md). This is the board-device
checkpoint only: no scheduler, CPU startup,
arbitration, IOMMU ownership change or two-CPU firmware boot.

## Implemented

- Interrupt controller banks keyed by hardware processor number. Processor 0
  is the default; SS-20/Axil can install processor 2. Processor-local timer and
  interrupt apertures are at `base + processor * 0x1000`, so processor 2 uses
  `0xff1302000`/`0xff1402000` in the high map. Processors 1 and 3 remain absent
  zero windows.
- The shared system controller keeps source pending and mask state. On
  SS-20/Axil, Current Interrupt Target TARGET bits 1:0 at system interrupt
  `+0x10` select the hardware processor for ordinary undirected device levels;
  bits 31:2 are reserved. Writing 2 selects the processor-2 bank. On SS-5 the
  NCR target address is reserved: reads return zero and writes are ignored. A
  valid but absent target leaves sources pending without delivering them;
  changing the target moves delivery while the source remains asserted.
- Each bank has independent software-pending bits, processor-timer pending and
  a level-15 capture. Bank set/clear writes affect only the addressed bank.
  MBus source status is separate from per-CPU capture: the source mask is
  sampled when the module-error source asserts, deassertion clears only system
  status, each processor clears its own captured bit, and the last
  acknowledgement does not clear an externally held source. The internal
  bus-error producer releases its live source as a pulse after capture. System
  source-mask writes do not acknowledge another processor. SS-5 retains its NCR
  single-CPU acknowledgement rule.
- Mask All retains the NCR89C105 policy: it gates delivered levels including
  software and processor timers, while pending state remains readable. This is
  deliberately different from QEMU's software-request shortcut and is
  documented in `docs/interrupts-timers.md`.
- Timers keep one shared 500 ns elapsed-time input and tick phase. Each
  installed hardware processor has independent counter limit/count/pending,
  user run/mode state and user count. Timer mode bits are hardware processor
  numbers 0 and 2, not compact bank indexes. Limit acknowledgements and mode
  transitions remain local to one bank.
- Bank installation is validated before mutation: SS-5 accepts only processor
  0, SS-20/Axil accepts 0 and 2, and timer/interrupt lower layers reject values
  outside hardware processors 0–3. Duplicate installs are idempotent and
  reset retains the installed population.
- `SystemBus` installs both banks through `install_processor_banks`; the
  default machine remains CPU 0 only. Processor-2 user-timer doubleword
  accesses are supported at its own base. Interrupt synchronization iterates
  the timer banks directly and performs no heap allocation.

## Evidence

- Core: 546 unique tests normal and 548 with `perf-counters`; captured logs
  contain 547/549 successful lines because the isolated MXCC cycle regression
  re-executes one test in a child process. Formatting, Clippy in both
  configurations and all four demos pass.
- Independent review fixtures: 8/8 pass, including SS-5 target read-only,
  MBus two-bit target, masked/deasserted/held level-15 states and invalid-bank
  rejection.
- Allocation probe: 20,000 device advances produce 0 allocations after warmup.
- `tests/smp_interrupts.rs` covers directed set/clear, CIT routing and absent
  targets, source masking/pending, Mask All, captured level-15 acknowledgement,
  independent timer limits/acknowledgements/mode bits, processor-2 bus
  addressing, invalid IDs and duplicate installation. Final review also
  covers bus-error pulse rearming/held input on both MBus profiles and the
  hardware-number 3/4 boundary; masked-event tests inspect captured state
  with global delivery enabled.
- Heliacal: 29 tests and a release build pass against this core.
- Fresh disposable SS-5, SS-20 and Axil boots reach root, print
  `HELIOS-BOOT-ACCEPTED` and halt cleanly; Axil retains
  `3034423527 17 /helios-disk-test`. The implementer's logs are under
  `local/smp-slice2a-2026-11-14/final-corrected-boot/`; independent final
  validation and fresh boots are under
  `local/review-smp-slice2a-2026-11-14/corrected/`.

## Limits

No scheduler, CPU startup/arbitration, directed IPI execution, IOMMU owner
split, two-CPU OBP discovery or two-CPU Solaris boot. The absent-bank and
Mask All policies are documented functional choices, not hardware validation
of every NCR/QEMU edge.
