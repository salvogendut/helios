# Division, IEEE flags and the OBP addition stop

Historical checkpoint; [fpu-addition.md](fpu-addition.md) records the subsequent
addition/subtraction implementation and FMULs stop. The probe/checker were later
renamed to `fp_arithmetic_probe` / `check_fp_arithmetic.py`; current commands are
in HACKING.md.

- FDIVs/d (`opf=4d/4e`) now execute; other arithmetic/conversions/quad traps
  remain explicit host stops. `src/fpu/division.rs` uses integer significands,
  exact u128 quotient/remainder and one rounding onto the normal/subnormal grid.
  No host floats, host exception state, dependencies or reference code copied.
  Largest intermediate has 106 significant bits; shifts below -1 are handled
  as below half a minimum subnormal, retaining directed rounding behavior.
- V8 B.34 and N.3–N.6: all four RD modes, nearest ties to even, gradual
  underflow; tininess BEFORE rounding. UFM=1 traps even exact nonzero tiny
  results. UFM=0 needs inexact for UF; rounding to minimum normal retains UF/NX.
  Overflow AFTER unbounded-exponent rounding; masked OF always includes NX.
  Finite nonzero / zero raises DZ; infinity / zero does not. Zero/zero and
  infinity/infinity raise NV. Signed finite/zero/infinity results use sign XOR.
- microSPARC-II 4.3 figure4.8 visually checked in /tmp/helios-ms2-nan.png,
  printed56/PDFpage55. Quiet NaNs clear sign; first operand wins if both quiet.
  Any SNaN yields ME_NaN: single7fff0000, double7fffe00000000000 with NV.
  Do not use the earlier microSPARC-I manual's different double ME_NaN.
  Numeric/numeric cell says IEEE754; for 0/0 and infinity/infinity we choose
  V8 N.4's positive all-fraction-ones qNaN (7fffffff/7fffffffffffffff).
  This numeric-invalid payload remains a documented choice, not silicon proof.
- Pinned QEMU f71afb409a87c4ecd0f9d63a5963eb645dfab780 cpu.c has signaling-first,
  B-before-A NaN propagation and generic all-fraction-ones default. Its defaults
  do not replace the chip-specific table. translate.c FDIV dispatch compared.
  Pinned MAME cd883a1be2fe0d120bfe594b0efcab5751ae565b sparc.cpp FDIVs/d dispatch
  delegates to SoftFloat, with a raw zero check on FDIVs that misses -0.
- `Fpu::complete_exceptions` is shared by comparisons/division. Normal result:
  clear FTT/CEXC, set CEXC flags, OR into AEXC. Enabled exception: highest enabled
  cause only (NV>OF>UF>DZ>NX), FTT1, preserve destination/FCC/AEXC, original
  PC/instruction captured, enter ExceptionPending. Existing next-FP detection,
  queue store/retry, sequence errors and reset behavior are unchanged.
  Double sources read before destination write; reserved low pair bits ignored
  as elsewhere. This remains a serialized one-entry deferred model.
- Ten new CPU-level regressions; 247 total pass with fmt/clippy and all four
  demos. Covers both widths, every destination and overlapping/odd pair aliases,
  signs, recurring fractions, subnormal halfway cases, tiny-to-normal rounding,
  huge exponent differences, overflow, special operands, NaN priority/payload,
  all five enabled causes, UF/NX and OF/NX priority, exact-tiny traps,
  destination/FCC/AEXC inhibition, deferred detection/drain/retry, EF and quad stop.
- `examples/fp_division_probe.rs` is a public-API guest-program harness.
  `tools/check_fp_division.py` independently computes Fraction results and
  binary-searches adjacent IEEE encodings; it does not reuse normalization or
  quotient rounding code. Run the commands in HACKING.md. Seed0x535335,
  2048 random operand pairs per width + targeted vectors, all RD modes and
  selected TEM settings: 48000 guest divisions pass result/FSR/queue checks.
  Neither this sample nor the directed tests establish full FPU conformance.
- Original ROM hash e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  Machine::ss5,128MiB four32MiB DSIMMs,default NVRAM,no input,100ns/step,30M budget.
  `local/probes/fpu-division-ss5/` retains configuration/source/bounded trace,
  CLI/library logs and oracle summary. Empty serial logs match, firmware hash
  verified. 25374279 instructions,2 diagnostic bus traps,0 interrupts,0 resets.
  FDIVs at70002860 computes100/4 ->f8=41c80000 (25); FSR00080000.
  Stop attempt25374282,PC70002864,nPC70002868,worda9a48833=FADDs f18,f19,f20.
  f18=41a00000 (20),f19=40a00000 (5),f20=0; PCR04004000,PSR044010e6,
  stateExecute,no queue,CPU error clear,CLI exit1. FP counts are the comparison
  checkpoint plus one FDIVs. No POST completion or PROM prompt claim.
- Next: single/double addition and subtraction with exact alignment, sticky
  bits/cancellation, all rounding modes and the same exception/NaN contract.
  The observed FADDs should produce25, but do not patch or shortcut this ROM case.
