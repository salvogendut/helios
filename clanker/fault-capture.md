# SBus faults, capture registers and the OBP FPU boundary

- Primary microSPARC-II rev1.1 sections 5.3, 5.6.4/5, 5.7.5–9, 9.1 and
  chapter11 table50; NCR89C105 maps, SBus subset, figures6-9/6-11. SBus standard
  table9 encodes SIZE byte=1, half=2, word=0, not log2(bytes). References and
  functional limits are in docs/fault-capture.md and docs/references.md.
- BusErrorKind::SBusTimeout and SBusError preserve no-ack versus slave ERR.
  Installed sun4m controller classifies unmapped PA[30:28]=3..7 as timeout,
  documented reserved NCR holes as ERR. Byte-only floppy aperture returns ERR
  on wide accesses; byte operations explicitly stop pending FDC implementation.
  This is not full NCR mirror/device/width decoding. Raw physical accesses
  report failures without CPU capture; bare MemoryBus is unchanged.
- SFSR.TO11/BE10 flow through ordinary reads, table walks and probes via
  MmuFault.bus_error. SFAR remains virtual. Existing priority, OW, NF and ET0
  error-state rules remain. Unsupported/WouldBlock do not become guest faults.
- Plain CPU byte/half/word SBus store failures complete asynchronously. AFSR
  captures ERR/TO-or-BE/S/SIZE/FAV, AFAR physical bits30:0. First error locks,
  subsequent errors set ME only. AFSR read unlocks pair, AFAR read does not.
  Register writes preserve masks: AFSR ff0e0000 plus constant00800000 on read;
  MFSR 8188f8f0. Figure5.22 includes ATO15 despite reserved-bit prose typo.
  Diagnostic ERR writes establish locks as an explicit functional choice;
  writable fields are documented, exact interaction with hardware locks is not.
- NCR module-error pending bit30 is separately latched; delivered at level15.
  CPU clear-pending IC15 acknowledges it, independently of soft31. Setting
  module mask30 clears the latch as documented. MaskAll only gates delivery.
  Reading/clearing AFSR does not acknowledge NCR. New error relatches IRQ even
  while capture remains locked. Diagnostic status writes do not assert IRQ.
  Do not copy QEMU's AFSR/AFAR read-clears-IRQ behavior for this board.
- MFSR/MFAR observe actual CPU/DMA DRAM requests including walks/RM updates.
  Bus hooks carry S/CP/BM/C/type. S uses actual PSR.S, not alternate-space
  permissions. CP=false for IOPTE walks and DMA; direct CPU walks CP=true.
  PTE C only for translated CPU target. Current uncached reads use RD64 type1;
  writes8/16/32/64 use9/a/b/c. No ordinary cache-line fills yet. Host bus/image
  access does not update capture. Direct MMU tests set CPU mode explicitly.
  MFSR and MFAR read-unlock separately, per their register descriptions.
- No fabricated parity errors from empty banks, no DMA translation-error
  AFSR/IRQ. Real parity storage/checking, ATO/late-error events, write-buffer
  timing and doubleword SBus fault completion remain pending. Failed doubleword
  SBus stores explicitly stop before capture; don't invent split/burst results.
- 16 focused tests added (206 total). fmt, clippy, all tests and all four demos
  pass. Coverage includes trap precision, walks/probes, NF/priority, size/mode,
  first/multiple capture, independent unlocks, IC/masks/soft15 isolation,
  guest store then IRQ, metadata, reset and unsupported operation isolation.
- Original ROM SHA256 unchanged:
  e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  Machine::ss5,128MiB,four32MiB DSIMMs,defaultNVRAM,no input,100ns/step,
  budget30M. Evidence under local/probes/fault-capture-ss5/ includes source,
  configuration, CLI stop/console, bounded library trace/console. Both empty
  transcripts match. Earlier warning and software reset no longer occur.
- Diagnostic traps29: attempt20588909 PC700063d0 c0850400 ASI20 load7cc00000
  ->TO; attempt20588995 PC700062b4 c0840400 load71400300 ->BE. Completed25373701
  instructions,2traps,0IRQs,0resets. Final attempt25373704 PC70002454,nPC70002458,
  wordc11c0011 = LDDF [l0+l1],f0/f1. l0=2000,l1=0,PCR04004000,PSR044010e6
  (EF=1),physical2000. CPU error clear, CLIexit1 UnsupportedInstruction.
  Cache diagnostic word stores ASI0c/0d/0e/0f=10752/65536/10752/32768.
  This path bypasses the previous warning-recovery MFSR stop; capture-register
  tests provide separate evidence, not this ROM run. No prompt/POST/OS claim.
- Next: V8/microSPARC-II FPU state and transfers, then required operations and
  precise trap behavior. Check disabled-FPU, FSR, paired register addressing,
  MMU/alignment/fault preservation before following further original-ROM POST.
