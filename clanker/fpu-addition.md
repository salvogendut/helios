# Addition/subtraction, rounding boundaries and the OBP multiply stop

- FADDs/d (`opf=41/42`) and FSUBs/d (`45/46`) now execute. Primary V8 B.34
  printed146 defines encodings, rs1 minus rs2 ordering and NV/OF/UF/NX (never DZ).
  N.3–N.6 define inhibited results, tiny exact traps and overflow after rounding.
  microSPARC-II4.3 figure4.8 retains the Meiko NaN rules from the division milestone.
  Checked pinned MAME cd883a1be2fe0d120bfe594b0efcab5751ae565b sparc.cpp add/sub
  dispatch (SoftFloat), and QEMU f71afb409a87c4ecd0f9d63a5963eb645dfab780
  translate.c do_env_fff/do_env_ddd. No source copied or differential claim.
- New `src/fpu/addition.rs` uses u64 significands with three low guard/round/sticky
  bits. Align by exponent; right-shift-jam folds all discarded nonzero bits into
  bit0, including shifts>=64. Larger magnitude determines sign, except exact
  cancellation. Close exponents cancel exactly; when alignment has discarded
  bits, subtraction needs at most one normalization shift, preserving rounding
  information. Maximum intermediate is57bits for binary64; no host floats.
- Exact cancellation is -0 only for RD toward -infinity; equal-sign zeros retain
  their effective signs. Apply subtraction's sign change only after NaN handling,
  retaining rs1 quiet-NaN preference even when sorting numeric magnitudes.
  Opposite effective infinities raise NV; other infinite results retain sign.
  Numeric invalid keeps the documented V8 all-fraction-ones default; its payload
  remains unverified against Meiko silicon, unlike the manual's SNaN ME_NaN rule.
- Tiny same-format add/sub results are exact multiples of the minimum subnormal.
  They raise UF with UFM=1, otherwise no UF/NX; exact zero never underflows.
  Overflow is AFTER rounding: maxfinite+minsub is maxfinite/NX under RN but
  infinity/OF|NX under RP. At the half-ulp boundary RN overflows, while truncation
  can still return maxfinite with NX alone. Directed tests cover these distinctions.
- `Fpu::binary` dispatches supported add/sub/div and commits results only after
  common exception handling. Shared `src/fpu/ieee.rs` now owns the existing NaN,
  IEEE bit constants and overflow-result rules. Division's arithmetic is unchanged;
  its original48000-case sample still passes. All sources read before destination
  writes; odd double-register encodings keep the documented containing-pair choice.
- Ten new tests;257total. fmt/clippy/alltests/all4demos pass. Add/sub tests cover
  every destination/pair alias, cancellation signs, halfway neighbors and ties
  to even, shifts at/beyond64bits, sticky subtraction borrows, normalization,
  subnormal sums, infinities/NaNs, enabled NV/OF/UF/NX priority and inhibition,
  deferred detection/drain/retry, EF and unsupported quad variants.
  Old missing-FADD fixture now uses quad subtraction, preserving host-stop coverage.
- Renamed `examples/fp_division_probe.rs` to `fp_arithmetic_probe.rs`; optional
  argument selects div(default),add,sub. Renamed checker to
  `tools/check_fp_arithmetic.py`, with --operations selectors (default all).
  Exact Python Fractions plus binary search over representable neighbors remain
  independent of the Rust alignment/jam algorithm. Add/sub samples include nearby
  magnitudes for cancellation. Seed0x535335,--samples2048: div48000,add55168,
  sub55168,158336total passing result/FSR/queue checks. No exhaustive/silicon claim.
- Original ROM SHA256 unchanged:
  e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  Machine::ss5,128MiB,four32MiB DSIMMs,defaultNVRAM,noinput,100ns/step,30Mbudget.
  `local/probes/fpu-addition-ss5/` holds configuration, bounded trace/source,
  CLI/library console/stop and oracle JSONL. Empty serial logs match; hash verified.
  25374281instructions,2busdiagnostictraps,0IRQs,0resets. FADDs at70002864 computes
  20+5 ->f20=41c80000 (25); a following LDF completes. No FSUB in this ROM run.
  Stopattempt25374284,PC7000286c,nPC70002870,word9da3092d=FMULs f12,f13,f14.
  f12=f13=40a00000 (5),f14=0,PCR04004000,PSR044010e6,FSR00080000,
  stateExecute,noentry,CPUerrorclear,CLIexit1. FPcounts:LDDF66,STDF66,LDF13,
  STF1,FMOVs8,FCMPs2,FBfcc2,FDIVs1,FADDs1. Prior cache/trap counts unchanged.
- Next: exact floating-point multiplication, starting with FMULs/d. Reuse
  the shared Meiko/flag/deferred contract; cover rounding, underflow and overflow
  before relying on further OBP progress. No POST completion or PROM prompt yet.
