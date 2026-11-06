# microSPARC-II I/O configuration and the next OBP cache-tag stop

Checked 2026-11-06. Contract: docs/io-control.md. This slice implements SBus
arbitration, five slot configuration registers, IOCR with translation disabled,
and the page-table base latch. It does not implement DVMA translation or faults.
The later checkpoint in cache-diagnostics.md supersedes this note's ASI0e stop.

- Primary manual is /tmp/helios-ms2.pdf, revision 1.1 July 1994, sections
  5.7.1–5.7.2, 5.7.7, 5.7.10, 9.4 and table 30. Printed pages 90–91 (PDF 88–89)
  give the exact control map. No MBus arbiter register at +0x1008 is listed.
- Figure 5.23 on printed page 99 (PDF 97) explicitly labels SBAE[5:0] at
  bits 21:16. Section 9.4 independently says six DVMA masters. The adjacent prose
  incorrectly calls bit 21 reserved. QEMU's 0x001f0000 mask loses master 5;
  use 0x003f0000 with fixed MID=8.
- IOCR hardwires IMPL=0/VER=4 per 5.7.1. QEMU's SS5 profile says 0x05000000;
  the implementation follows the target's primary manual and records the difference.
  RANGE=bits4:2, reserved/version writes ignored. EN=1 stops before mutation
  because no translated DMA path exists yet. IBAR mask=0x07fffc00.
- Figure 5.21 printed page 96 (PDF94) places BA8 at bit1, BY at bit0 and SA30
  at bit16. Bits15:2 are reserved in the diagram (prose is off by one).
  SSCR0–3 mask=0x10003; SSCR4 shared by masters4/5 has only BA8, mask=2.
  Both masters must use translation; do not copy QEMU's four-slot array behavior.
- SystemBus reserves [0x10000000,0x10008000). Aligned words only; no byte,
  masked, atomic, doubleword or image writes. Known missing register facilities
  stop explicitly; reserved holes return Unmapped. Do not fabricate registers
  as a generic writable RAM array or import QEMU's large pad region.
- I/O TLB flush ports, AFSR/AFAR, MFSR/MFAR, breakpoint/performance controls and
  local graphics queue registers remain explicit Unsupported stops. Future DMA
  work must define actual translation, TLB sharing, slot arbitration/bypass,
  fault capture and IRQ wiring; configuration latches alone do not provide it.
- Both power-on and guest software reset clear writable fields. Existing
  Machine watchdog stop behavior is unchanged; automatic partial reset is pending.
- Original OBP now passes the old MID store and completes 20004464 instructions
  with NO trap, interrupt, reset or console output. Next attempt (20004465) stops
  at PC=7000ac40, nPC=7000ac44, instruction=c0a401c0: ASI0e word store, data-cache
  tag initialization. It is an emulator stop, with no CPU error latch.
  The earlier ASI06 TLB diagnostic stop was reached only after the prior fault
  and guest reset. Implement the cache diagnostic spaces next, following the
  current normal initialization path rather than the obsolete error path.
- Probe settings unchanged: original ss5.bin hash in docs/firmware.md,
  128 MiB, default NVRAM, no input, 100 ns per successful step, budget30M.
  local/probes/io-control-ss5 retains CLI and bounded library records/source.
  No firmware prompt or OS boot; eight new tests cover field masks, guest ASI
  access, protection, reset and precise stops, without relying on firmware.
- Disposable rendered manual pages: /tmp/helios-ms2-mid.png,
  /tmp/helios-ms2-slot.png, /tmp/helios-ms2-iomap-first.png and iomap.png.
  These were inspected; do not reintroduce ambiguities caused by text extraction.
