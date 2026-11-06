# Comparisons, deferred exceptions and the OBP divide stop

Historical checkpoint; [fpu-division.md](fpu-division.md) records the subsequent
division implementation and current FADDs stop.

- Primary V8 B.34, N.3/N.4 and table7-1: FCMP invalid for SNaN only, FCMPE
  invalid for any NaN; enabled traps inhibit FCC/AEXC/register results, CEXC
  records NV alone. Priority: privilege, EF, alignment, FP exception, data faults.
  microSPARC-II4.4/figure4.10 defines actual deferred detection. Diagram visually
  checked /tmp/helios-ms2-fpu-states.png, printed57/PDFpage56. Section4.5 retains
  FTT across transfers/STDFQ. Existing status masks/Meiko identity unchanged.
- FCMPs/d and FCMPEs/d now classify raw IEEE words, no host floats: exponent
  allones+nonzero fraction =NaN; top fraction bit quiet. Signed zeros equal;
  same-sign negative encodings reverse numeric order. Double comparison uses
  both words and masks reserved low register bits, like transfer subset.
- Unmasked success sets FCC, clearsFTT/CEXC, retainsAEXC. Masked NV setsFCC=U,
  CEXC.NV and AEXC.NV. EnabledNV preservesFCC/AEXC/allregs, setsFTT1/CEXC.NV,
  captures originalPC/word and entersExceptionPending without immediate trap.
- FpuState exposes Execute/ExceptionPending/Exception; DeferredEntry holdsPC
  andinstruction. QNE derives from entry presence, cannot be set by LDFSR.
  Interpreter resolves each FPop before another issues: only one faulting entry
  can remain. Hardware has3pipelineentries, but multi-entry timing/younger in-flight
  FPops aren't modeled. Do not describe this as full pipelined FPU emulation.
- CPU checks privilege/EF/alignment before Fpu::prepare and any data translation.
  Pending + any FP opcode/load/store/branch ->Exception and TT8, no detecting
  instruction effects. Integer instructions/interrupt entry don't acknowledge it.
  Saved CPU PCs belong to detector; queue retains originalFPop. ET0 latches CPU
  error on detection. Issuing comparison retires without producing an FP result.
- Exception+load/FPop ->FTT4,pending, skipnormal effects/translation, no newentry.
  NextFP again triggersTT8. Exception+stores/branches allowed. This follows the
  explicit exclusion of fbcc in section4.4's sequence-error rule. Keep manual
  precedence over QEMU's generic branch/sequence behavior.
- STDFQ writes [originalPC,instruction] through atomic doublewordbus, then
  removesentry and returnsExecute. Failed/unsupportedstore retainsqueue. FTT/CEXC
  survivequeue drain untilnextFPop. EmptyFQ retains prior immediateFTT4/TT8 rule.
  Current MMU PCR.NF completion semantics also commit dequeue for suppressed
  storefaults; recorded/tested as functional contract. No hoststop turns into
  guest success. Both CPU reset methods defaultFpu and clearqueue.
- Unsupportedarith/convert/quad behavior stays hoststop inExecute. Pending or
  Exception statechecks precede opcode implementation lookup. Do not map missing
  hardware-supported FDIV to unimplemented_FPop. Actual quadFTT3 support pending.
- Compared pinned QEMU f71afb409a87c4ecd0f9d63a5963eb645dfab780 translate.c
  gen_trap_iffpexception/do_fcmp[sd]: QEMU says it skipsPending and traps immediately.
  MAME cd883a1be2fe0d120bfe594b0efcab5751ae565b sparc.cpp comparison dispatch
  checked; don't infer NaN/flag behavior from generic paths instead of V8 rules.
- 15 new tests;237total. fmt/clippy/alltests/all4demos pass. Covers single/double
  subnormals/zeros/infinities/order, quiet/signalingNaNs eitheroperand,bothsigns,
  FCMP/FCMPE andNV/otherTEM settings, no source changes, saved PCs, delayed entry,
  higherpriorityexceptions, everyFP triggerclass, sequence-error deferral,
  permittedexceptionstatestores/branches, STDFQ partialwrite isolation/retry/NF,
  interrupt isolation,ET0watchdog and reset. Oldmissing-FCMPfixture nowusesquad.
- OriginalROM hash unchanged:
  e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  Machine::ss5,128MiB four32MiB DSIMMs,defaultNVRAM,no input,100ns/step,budget30M.
  local/probes/fpu-compare-ss5/ holdsconfiguration/source/boundedtrace/CLIconsole
  andstop; CLI/libraryemptyserialtranscriptsmatch. Completed25374278instructions,
  2busdiagnostictraps,0IRQs,0resets. Stopsattempt25374281 PC70002860,nPC70002864,
  word91a189a7=FDIVs f6,f7,f8. f6=42c80000(100),f7=40800000(4),f8=0,
  PCR04004000,PSR044010e6,FSR00080000,stateExecute,noentry,CPUerrorclear,CLIexit1.
- FPcounts:LDDF66,STDF66,LDF12,STF1,FCMPs2,FMOVs8,FBfcc2. No exception queue
  exercised by this ROM run; tests provide that evidence. Priorcachecounts
  10752/65536/10752/32768 and traps20588909/20588995 unchanged. No POST/promptclaim.
- Next: IEEE arithmetic, starting with single-precision division. Must handle
  exact bits, allRD modes, invalid/divzero/overflow/underflow/inexact flags,
  signedzero/infinity/subnormal inputs and microSPARC-II4.3 NaN result rules.
  Reuse deferred mechanism for enabledexceptions. Avoid host f32 shortcuts that
  lose flags/rounding or quietly substitute generic NaN propagation for Meiko.
