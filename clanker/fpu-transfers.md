# FPU state/transfers and the first OBP comparison

- V8 sections4.4/5.1.7/7, B.2/5/22/34 and C.7/9: EF gates FP instructions,
  word/pair transfers, FCC branch conditions/delay/annul, moves clear CEXC but
  retain AEXC. microSPARC-II4.3–5/table6: Meiko version4, NS=0, read-only
  FTT/QNE/VER, FTT survives transfers and clears on completed FPop. Table6
  visually checked at /tmp/helios-ms2-fsr.png (printed59, PDFpage58).
- Public Cpu::fpu contains [u32;32], raw word patterns with no host float or
  endian conversion. f0 ordinary storage; no integer-window aliases. LDF/LDDF,
  STF/STDF, LDFSR/STFSR use existing SRMMU/physical width paths. Paired access
  only commits after successful bus doubleword transfer. EF checked first.
- FSR_WRITABLE=cf800fff (RD/TEM/FCC/AEXC/CEXC), fixedVER=80000. FTT unchanged
  by LDFSR/STFSR. Reserved/NS alwayszero. FMOVs/FNEGs/FABSs raw bit transforms,
  clear FTT/CEXC and retain other fields, even for signaling NaN patterns.
- V8 reserves rd[0] in LDDF/STDF; optional invalid_fp_register trap recommended.
  We mask rd[0], matching MAME LDDF. microSPARC-II FTT table lists only0..4.
  Explicit functional choice for nonportable encodings, not silicon verification.
  LDFSR/STFSR ignore rd. Never interpret V9 alternate FP transfers as V8 FP.
- FBfcc all16 conditions, E/L/G/U=0/1/2/3, shares Bicc delay/annul mechanics,
  uses only FCC. FBA/FBN also checkEF. FSR/FCC updates immediate; retain portable
  instruction spacing in guest fixtures. No FPop timing/interlocks simulated.
- Recognized FP memory/branch/FPop1/FPop2 opcodes EF0 ->FpDisabled/TT4, even if
  enabled FPop behavior is missing. STDFQ priority privileged->EF->8byte alignment
  ->empty-FQ sequence error. Empty STDFQ setsFTT4, immediate TT8, no memory/table
  access, QNE0 and execution state unchanged. ET0 follows existing CPU error latch.
- Nonempty queue/deferred exception state machine is NOT implemented. Future
  actual FPop exceptions need3-entry FQ and execute/pending/exception states:
  cause queues faultingPC/word; next FP instruction (including stores/branches)
  triggers trap; in exception state stores/branches permitted, FPop/loads cause
  sequence error pending; draining FQ returns to execute. Read manual4.4 again
  when implementing. Don't copy QEMU's generic stores-ok-in-pending shortcut.
- Supported FMOV/NEG/ABS cannot cause exceptions and finish synchronously.
  Other FPop with EF1 is retryable UnsupportedInstruction, unchanged FPU and no
  guest exception. Includes comparisons, arithmetic, conversions and quad ops.
  Do not fabricate FTT3 merely because Helios lacks a hardware-supported FPop.
- QEMU pinned f71afb409a87c4ecd0f9d63a5963eb645dfab780 translate.c/cpu.c checked
  for EF/status/STDFQ and identity. Its empty-FQ path precedes memory alignment;
  Helios uses V8 priority. MAME cd883a1be2fe0d120bfe594b0efcab5751ae565b sparc.cpp
  load functions checked; generic LDFSR mask is not the microSPARC-II contract.
- Reset uses existing deterministic baseline: all FPU words/status cleared,
  fixedVER visible, EF0. Software reset retains integer registers only; don't
  claim all these undefined FPU values are hardware reset requirements.
- 16 new tests, 222 total, fmt/clippy/all4demos pass. Covers every register/raw
  patterns, word/pair paths, all FCC/condition/annul/forward/back combinations,
  mask/read-only fields, raw NaN moves, disabled/emptyqueue priority, delay-slot
  savedPC/nPC, MMU user permission/NF, partial-transfer isolation, MMIO host stops,
  async SBus capture and reset. Older unsupported-instruction fixtures changed
  from now-supported FMOV to ASR/CPop; dedicated enabled missing-FPop tests added.
- Original ROM SHA256 e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e
  unchanged. Machine::ss5,128MiB four32MiB DSIMMs,defaultNVRAM,no input,100ns/step,
  budget30M. local/probes/fpu-transfers-ss5/ stores source/config/boundedtrace and
  CLI stop/console. Empty CLI/library transcripts match. Completed25374200
  instructions,2diagnostictraps,0IRQs,0resets. 66LDDF +66STDF complete.
- Stopattempt25374203 PC70002728,nPC7000272c,word81a80a22=FCMPs f0,f2.
  f0=f2=01234567,f1=f3=fedcba98,PCR04004000,PSR044010e6,FSR00080000.
  FPop still hoststop, CPUerrorclear, CLIexit1. Prior fault traps unchanged at
  attempts20588909/20588995. Cache diagnostic counts unchanged10752/65536/10752/32768.
  This probe executes transfers only, not the separately tested FS ops/branches.
- Next: IEEE comparisons and microSPARC-II deferred exception handling. Avoid
  host f32/f64 comparisons losing signaling NaN semantics; preserve NaN payloads,
  signed-zero equality and FCC/FSR on enabled invalid traps. No prompt/POST claim.
