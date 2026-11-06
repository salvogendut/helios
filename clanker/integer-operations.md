# Integer and multi-access rules

See HACKING.md and docs/memory.md for the human contract. Consult the pinned
references from docs/references.md rather than copying a generic emulator's
implementation choices.

- microSPARC-II User's Manual section 3.6, printed p41, chooses SPARC V8 signed
  negative overflow method B. Exact mathematical results below -2^31 overflow,
  including when truncation produces -2^31 with a nonzero remainder. Regression:
  -4294967297 / 2 -> 0x80000000, V=1; -4294967296 / 2 -> same value, V=0.
- Signed division uses i128 intermediates so i64::MIN / -1 does not panic. Y is
  retained after division as an explicit V8 implementation choice; not a guest
  ABI guarantee. UMUL/SMUL always update Y, even with rd=g0.
- MULScc must consume old N/V, old Y bit zero and old rs1 bit zero before writes.
  A 33-step multiplication test checks the composed operation independently.
- Tagged TV faults leave destination and ICC unchanged. Trap types: tag 0x0a,
  divide-by-zero 0x2a. Old tests that used unsupported UDIV now use FMOVs.
- Bus requires doubleword read/write, swap32 and ldstub8 explicitly. Do not add
  generic destructive-read-then-write fallbacks for devices. MemoryBus validates
  both doubleword destinations before stores. No partial register commits on LDD.
- Doubleword alignment keeps both words in one page. Even register pairs only;
  privilege and alternate i=1 checks precede odd-rd/alignment checks.
- Atomics use store permission/AT and set R/M before the indivisible bus action.
  Denied accesses under NF return zero and never touch the target device.
- FLUSH is unprivileged, coherent-memory no-op, with no data translation. It must
  not clear the TLB. Tests include self-modifying code and stale-PTE retention.

The hardware reference ASI chapter is 12 (not 11 as earlier notes said).
No reference emulator differential run or real firmware execution has been done.
