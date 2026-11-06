# Multiplication and the timer/breakpoint blockers

- FMULs/d (49/4a) now execute. V8 B.34 printed147 and N.3–N.6 define operands,
  rounding, exception inhibition, underflow before rounding and overflow after.
  microSPARC-II4.1 says same-precision multiplication completes in hardware even
  if the fast multiplier restarts in Meiko; only FsMULd can need unfinished.
  FsMULd (69), FMULq (4b), FdMULq (6e) remain explicit host stops.
- `src/fpu/multiplication.rs`: exact integer significands times powers of two;
  product <=48bits(single)/106bits(double) fits u128. Determine top bit, choose
  normal/subnormal grid, round once using exact discarded remainder versus half.
  Shifts>=128 imply strictly below half, avoiding host shift overflow while
  retaining directed rounding. Sign XOR applies to finite/zero/infinite results.
  Zero*infinity raises NV; other infinity products are signed infinity; no DZ.
  Shared Meiko NaNs and numeric-invalid default unchanged (latter not silicon-verified).
- Fpu::binary dispatches mul before add/sub; shared flag/queue handling inhibits
  destination/FCC/AEXC on enabled exceptions. Reserved odd pair choice unchanged.
  Pinned QEMU f71afb409a87c4ecd0f9d63a5963eb645dfab780 translate.c FMULs/d and
  MAME cd883a1be2fe0d120bfe594b0efcab5751ae565b sparc.cpp SoftFloat dispatch checked.
  No reference implementation copied; multiplier pipeline timing not modeled.
- Ten new guest-program regressions;267total. fmt/clippy/tests/all4demos pass.
  Covers all destinations/pair aliases, low product bits, ties-even, binade carry,
  tiny-to-normal rounding, exact-tiny UFM, extreme exponents, nonnormalized input
  becoming normal, overflow/NX-only boundary, zero/infinity/NaN, exception cause
  selection/inhibition, deferred detection/drain/retry, EF and missing wide/quad ops.
- Exact Fraction oracle now accepts mul in probe/checker; existing sequences
  unchanged. Seed535335 hex,--samples2048: div48000,add55168,sub55168,mul50128,
  total208464 pass result/FSR/queue checks. Boundary pairs include half-minsub,
  subnormal-to-normal, normal ties and products just below overflow's next binade.
- Original ROM SHA256 unchanged:
  e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  Machine::ss5,128MiB,four32MiB DSIMMs,defaultNVRAM,noinput,100ns/step,30Mbudget.
  `local/probes/fpu-multiplication-ss5/` stores metadata/source/bounded trace,
  CLI/library console/stop and oracle JSONL. Matching71-byte serial output:
  No interrupt received, expected0000001e;UNUMBER U1507. 30M budget ends
  PCffd2f554/nPCffd2f558,29999968instructions,16traps,16IRQs,1reset,CLIexit0.
  Instruction count includes the resetting store. No host error at this budget.
- FPU counts:FMOVs8,FADDs5,FADDd4,FSUBs2,FSUBd1,FMULs2,FMULd1,FDIVs5,FDIVd4,
  FCMPs12,FCMPd6,FBfcc18,LDF25,LDFSR12,LDDF89,STF13,STFSR36,STDFQ12,STDF74.
  Synchronous traps:12FPexceptions,2misalignedFPstores,2previousbusdiagnostics.
  Software IRQL1..15 then timer diagnostic fails; software reset at28539752;
  another IRQ14 at29475023. Cache writes0c/0d/0e/0f=20480/65536/20480/32768.
- Longer100M-budget CLI run prints bad configuration checksum/default NVRAM
  messages and stops PCffd27690,nPCffd27694,wordc8840400,ASI20read10003018,
  unsupported microSPARC-II performance/breakpoint register,CLIexit1. Logs saved.
- Next investigation: /tmp/helios-irqdiag-probe.rs/.log captures timer/IRQ MMIO
  after25375000. CPU limit set7ffffe00 (4194303ticks at500ns), interrupts unmasked
  at25377381, loop expires at28523118 after~0.315s of current100ns/instruction
  time, before the~2.097s timer limit. Not an FPU failure. /tmp/helios-disasm.py
  decodes subset for inspection; 700069a4 sets loop counter80000, six instructions
  per iteration. PROM is 8-bit per NCR89C105, dynamic bus sizing requires four
  transactions for word fetches; current model assigns identical time to RAM and
  PROM instructions. Investigate realistic functional PROM access timing before
  changing timer semantics. NCR text /tmp/helios-ncr89c105.txt lines248–376.
  Then implement documented breakpoint register reads/writes at10003018.
- User explicitly requests continuous progress until original OBP serial `ok`
  prompt. Do not end work after each opcode milestone; continue through blockers.
  No completed POST or PROM prompt has been demonstrated yet.
