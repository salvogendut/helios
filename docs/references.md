# Implementation references

Use the architecture manual for instruction semantics and consult independent
emulators for implementation details, device behavior and practical firmware
requirements. When they disagree, retain the disagreement and build a focused
test rather than assuming either emulator defines the hardware.

## Architecture and project model

- [SPARC International technical documents](https://sparc.org/technical-documents/):
  SPARC Architecture Manual Version 8, especially integer state, instruction
  encodings, windows, traps and the reference MMU.
- [SPARC V8 manual, compressed PDF](https://sparc.org/wp-content/uploads/2014/01/v8.pdf.gz):
  chapters 4 and 7, B.26–B.29 and C.8 were checked for state registers, trap
  entry, RETT priority and error mode. Immediate state writes use the permitted
  zero-delay implementation choice; guest examples retain portable spacing.
  B.1/B.4 and B.7/B.8 define doubleword transfers and atomic exchanges;
  B.14, B.16–B.19 and B.32 define tagged arithmetic, MULScc, multiply/divide and
  FLUSH. Chapter 7 distinguishes precise faults from optional partial effects
  after non-resumable multi-access machine errors.
  Sections 7.3–7.4 and C.2/C.8 also define interrupt eligibility, ET/PIL, level
  15, boundary sampling, saved PC/nPC and trap entry without modifying PIL.
  Sections 4.4 and 5.1.7, B.2/B.5, B.22/B.34 and C.7/C.9 define FPU state,
  memory transfers, moves, branches and trap priority. V8 reserves odd double
  register encodings; its recommended invalid-register trap is optional.
  [fpu.md](fpu.md) records the implemented subset and explicit choices.
  B.34 and N.3–N.4 define FCMP versus FCMPE NaN behavior and inhibited
  FCC/AEXC updates for enabled invalid exceptions. B.34 defines FDIVs/d, FADDs/d,
  FSUBs/d, FMULs/d and integer/single/double conversion encodings and rounding;
  N.4–N.6 define default NaNs, overflow after rounding, tininess before rounding,
  exact-tiny traps with UFM enabled and the UF/NX priority table. N.7 defines
  signed integer range limits after truncation. Table 7-1 puts privilege,
  EF and alignment ahead of an FP exception, ahead of data-access faults.
- [Sun microSPARC-II User's Manual, revision 1.1, July 1994](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf):
  primary hardware manual hosted by Temlib. Chapter 5 (tables 9, 18, 19, 23–28
  and 40) and chapter 12 define the target's 31-bit addressing, contexts,
  permissions, probe/flush, fault state, NF, boot mapping and ASIs. The PCR
  figure/reserved-bit wording and root PTE descriptions have inconsistencies;
  [memory.md](memory.md) states the implemented choices and limits. Sections
  3.11.3 and 10.2 distinguish power-up, non-power-up and watchdog reset;
  [reset.md](reset.md) describes the functional subset and recovery limit. Sections
  3.5–3.6 define integer multiply/divide, including negative division-overflow
  method B on printed page 41. Chapters 6/7 and table 55 define cache diagnostic
  storage, widths and indexed flushes. Figures 6.1/7.1 were checked visually;
  V8 appendix I table I-2 resolves table 55's segment-address typo. See
  [cache.md](cache.md) for the implemented subset and ordinary cache limitations.
  Table 30, sections 5.7.1–5.7.2, 5.7.7, 5.7.10 and 9.4 define the implemented
  I/O configuration subset. Figures 5.21/5.23 were checked visually to resolve
  omitted bit labels and the SBAE reserved-bit typo; [io-control.md](io-control.md)
  records the chosen masks and differences from QEMU. Sections 5.7.17–5.7.18,
  figure 5.30 (checked visually) and table 38 define the mask ID and virtual
  address compare configuration; section 5.14.2 distinguishes external lab
  breakpoint logic from CPU traps. Sections 8.2.1–8.2.2
  define the DRAM bank map, mirrors and empty-bank completion; sections 5.2,
  5.6.6 and 5.14.1 define TLB diagnostic data/tags and replacement controls.
  Figures 5.15/5.35/5.36 and table 41 were inspected visually. Sections 5.2.5,
  5.7.1–5.7.4, 5.8, 5.10, 9.1 and chapter 11 table 50 define IOPTEs,
  table indexing, selective I/O flushes, bypass and device-facing DMA errors.
  Unlike QEMU's generic IOMMU fault path, invalid/protected DMA mappings return
  an SBus error acknowledge without synthesizing a CPU late-error interrupt.
  [dram-tlb.md](dram-tlb.md) records implemented behavior and functional limits.
  Sections 5.3, 5.6.4, 5.7.5–5.7.9 and chapter 11 table 50 define SBus TO/BE,
  asynchronous write capture and memory metadata. Figures 5.13/5.19/5.22 and
  table 32 were checked visually; figure 5.22's ATO bit 15 overrides its prose's
  reserved-bit typo. [fault-capture.md](fault-capture.md) distinguishes implemented
  requests/capture from pending parity, late errors and bus-cycle timing.
  Sections 4.3–4.5 and table 6 define the Meiko FPU identity, read-only FSR
  fields, NS=0, FTT lifetime, three deferred-exception states and empty-STDFQ
  behavior. Table 6 and figure 4.10 were checked visually. Comparisons now
  implement deferred invalid exceptions and queue recovery using those states.
  Execution is serialized, so only one faulting entry remains; the hardware's
  three-entry pipeline timing is not modeled. Section 4.3 figure 4.8 was checked
  visually for arithmetic NaN propagation: first quiet operand wins, quiet NaN
  results have sign zero, and signaling operands produce ME_NaN (`0x7fff0000`
  single, `0x7fffe00000000000` double). The numeric/numeric cell does not specify
  an invalid-result payload; Helios chooses V8 N.4's positive all-fraction-ones
  default, pending silicon evidence. Section 4.1 distinguishes same-precision
  multiplication (always completed in hardware, possibly via Meiko restart)
  from FsMULd's possible unfinished exception. The current emulator implements
  FMULs/d without multiplier pipeline timing; widening multiplication is pending.
  Figure 4.9 was visually checked for format/integer conversion NaNs, including
  FDTOS canonicalization and FSTOI/FDTOI positive quiet NaNs. These chip rules
  override V8 N.4/N.7's generic recommendations; see [fpu.md](fpu.md).
- [Sun SBus I/O Chipset Data Manual, NCR89C100, revision 1.0](https://bitsavers.trailing-edge.com/pdf/sun/sparc/801-3137-10_SPARCclassic_Engine_OEM_Technical_Manual_Part_Three_IO_Chipset_Apr1993.pdf):
  chip-level address map (table 2-7, footnote 3 explicitly permits narrow ID
  reads; inspected visually) and chapter 3 pages 3-6–3-7 and 3-26–3-33 define MACIO
  identification, slave widths and the DMA2 SCSI register subset. Page 3-26 was
  inspected visually: D_WRITE is bit 8, omitted from the archived
  [DMA2 text](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/DMA2.txt).
  The [NCR89C100 text](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/NCR89C100.txt)
  also records the integrated SCSI/Ethernet cores and board-relative decoding.
  See [devices.md](devices.md#ncr89c100-scsi-dma-initialization) for implemented
  behavior and limitations. Table 2-7 footnote 2 and DMA2 table 3-1 define the
  bit-5 register aliases; pages 3-21–3-22 define E_TST_CSR and E_VLD. Helios
  exposes address/control state and an empty coherent cache, while diagnostic
  fills/tag loading remain unsupported. Pinned QEMU `hw/dma/sparc32_dma.c` is a comparison;
  its shared CSR mask and reset pulse do not override the primary manual.
- [NCR53C9X integrated Fast SCSI core specification](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/NCR53C9X.txt),
  also chapter 5 of the Sun I/O chipset manual above: register descriptions,
  miscellaneous/disconnected commands, initiator selection/transfer/status
  sequences and reset timing define the ESP subset. The NCR89C100 integration chapter excludes the
  standalone RESETO pin. Pinned QEMU `hw/scsi/esp.c` supplies a comparison;
  Helios follows the primary count/counter separation, ID gating, reset
  retention and FIFO-overwrite status rather than assuming QEMU's shortcuts.
  Selection timeout follows `timeout * 8192 * CCF / 40 MHz`; absent IDs report
  disconnect/step 0, successful selection reports function complete plus bus
  service/step 4. The archived text's select-with-ATN interrupt table has
  transcribed bit columns inconsistent with the chapter's prose and other
  selection tables; the prose and independent controllers establish the
  function-complete/bus-service result used here.
  DMA2 chapter 3 defines count expiration/chaining, D_TC W1C only in chaining
  mode, and D_INVALIDATE clearing memory error/TC. See [scsi.md](scsi.md).
  Chapter 5 page 5-20 permits CFG1 chip-test-enable readback but requires chip
  reset before normal operation. Helios retains that state; test-register
  execution remains unsupported. Chapter 3 pages 3-18–3-22 and chapter 4 define
  the Ethernet DMA/CSR, initialization block, packet descriptors and internal
  loopback. [ethernet.md](ethernet.md) records the subset and the E_BASE_ADDR
  bit-position disagreement with Linux/QEMU. CSR3 programmable IPG is an NCR
  extension, not PCnet's interrupt mask. The pinned MAME AM79C90 comparison
  confirms loopback FCS and 8–32-byte test-packet handling.
- [Intel 82077AA datasheet](https://ardent-tool.com/datasheets/Intel_82077AA.pdf),
  Intel document 290166: sections 2.1–2.2, 5.2–5.3, 6.4, 7.4 and 8.2 define
  command/result handshakes, reset retention, configuration, polling suppression,
  seek/recalibrate and status. NCR89C105 figure 6-5 overrides PC DOR wiring;
  only its listed byte registers are exposed. The 79/80-step recalibration
  discrepancy, disconnected drive pins and functional timing choices are
  recorded in [devices.md](devices.md). Pinned QEMU `hw/block/fdc.c` provides a
  comparison; its generic PC register/reset assumptions are not copied wholesale.
- [ST M48T08/M48T08Y/M48T18 datasheet](https://www.st.com/resource/en/datasheet/m48t18.pdf),
  DocID2411 Rev 11, June 2011: table 5 and sections 3.1–3.3 define the 8 KiB
  register layout, BCD calendar, R/W transfer latches and oscillator stop.
  The model follows the hardware latch behavior rather than QEMU's simpler
  host-clock-backed register handling.
- [Zilog SCC User's Manual, 1992](https://bitsavers.trailing-edge.com/components/zilog/_dataBooks/SCC_Users_Manual_1992.pdf),
  primary manual archived by Bitsavers: section 2.3.4 and tables 2-5/2-7 define
  Z85X30 register addressing, aliases and reset state; chapter 5 defines the
  SCC control/status registers. The pointer is shared across channels and a
  direct data-port access does not reset it. MAME agrees; the inspected QEMU
  model instead keeps a per-channel pointer. Helios uses the manual's behavior.
  Section 5.2.16 distinguishes WR14's disabled-DPLL configuration commands from
  Enter Search Mode, which enables the DPLL. OBP uses the former during setup.
  Sections 2.4.9, 4.2.2 and 5.3.1 define decoded receive BREAK, both status
  transitions and the single terminating null byte. Sections 2.4.4–2.4.9,
  5.2.1–5.2.3 and 5.2.11 define pending bits, interrupt-without-acknowledge,
  first/all-character receive, transmit-empty transitions, BREAK status latches,
  CRC reset commands and the vector encoding. Helios uses these asynchronous
  interrupts without SCC IUS state; bit timing and software acknowledge remain
  pending. Unlike QEMU's receive helper, reception alone does not set IUS.
- [Sun NVRAM/hostid FAQ, revision 1.39, 1996-11-26](https://lib.ru/TXT/faqsunnvram.txt):
  Mark Henderson's IDPROM programming reference, supplied by the user. It
  identifies SS5 machine type `80`, MAC bytes 2–7, serial bytes c–e and the
  XOR checksum at f; it recommends Sun's `08:00:20` prefix for compatibility.
  The chosen default is MAC `08:00:20:c0:ff:ee`, hostid `80c0ffee`, date zero.
- [NCR89C105 Chip Specification](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/NCR89C105.txt),
  primary chip documentation preserved in the historic Linux/SPARC archive and
  referenced by QEMU: table 6-7 maps both SCC instances and reset/status;
  figure 6-6 defines SR/RS/WD. Tables 6-21 through 6-25 and figures 6-8 through 6-18
  define interrupt pending/masks, source levels, reset and counter timers.
  Figures 6-21/6-22 define AuxIO1 writable output latches, read-only density and
  self-clearing terminal-count pulses; [devices.md](devices.md) records the
  functional event interface, AuxIO2 scratch/clear behavior and missing external
  device connections and power-state handling.
  The archived text has no revision header. Its zero-limit overflow interrupt
  rule conflicts with Sun-4M section 5.3.2 below; Helios follows Sun's explicit
  zero-limit suppression and skip-zero overflow rule. Mask All's treatment of software requests
  is less explicit; [interrupts-timers.md](interrupts-timers.md) records the choice.
  The SBus feature subset restricts two-word bursts to the processor count in
  user mode. Figures 6-16–6-18 define its 54-bit count, overflow flag, Run and
  mode select; the mode-transition transfer state is not specified. The address
  map and SBus feature subset define errors for reserved windows and invalid
  widths, including the byte-only floppy aperture. Figure 6-9 defines IC at
  processor-clear bit 15; figure 6-11 assigns module error to system bit 30.
- [ISO/IEC 15205:2000 / IEEE 1496, table 9 excerpt](https://www.normsplash.com/Samples/CSA/158135716/CAN-CSA-ISO-IEC-15205-02-%28R2016%29-en-2.pdf):
  primary SBus standard, archived excerpt pages 50–60; table 9 gives SIZ encoding
  word=000, halfword=010 and byte=001, used by AFSR capture.
- [Sun SPARCclassic Engine OEM Technical Manual, Part One, April 1993](https://bitsavers.trailing-edge.com/pdf/sun/sparc/801-3137-10_SPARCclassic_Engine_OEM_Technical_Manual_Part_One_Apr1993.pdf),
  801-3137-10: section 5's NCR89C105 overview and section 7's SLAVIO address map
  independently confirm the uniprocessor layout at `0x71d00000`/`0x71e00000`.
  This is a related machine's primary manual; QEMU's SS-5 profile confirms the
  same bases for the selected target. Do not import its CPU or RAM profile.
- [Sun SPARCstation 5 Service Manual, August 1994](https://docs.oracle.com/cd/E19127-01/sparc5.ws/801-6396-11/801-6396-11.pdf),
  801-6396-11: table 12-1 lists the 85 MHz system board, SCSI disk/CD-ROM options
  and 1 MiB TGX card for the selected initial profile. Chapters 2–4 cover POST,
  diagnostics and the serial tip connection; this is also the manual supplied
  by the user during bring-up. Section 11.3 confirms eight DSIMM slots and
  8/32 MiB module sizes.
- [Sun SPARCengine 5 OEM Technical Manual, September 1995](https://device.report/m/24e55e1efd74cebad92ef139a523192872b87937967e4567f9cdafcecf56e515.pdf),
  801-7104-11: table 2-1 covers microSPARC-II clock variants; section 3.2.9.2
  describes the 1 MiB TurboGX SBus card with BT458 RAMDAC. Sections 3.2.7.4/3.2.9 distinguish AFX from SBus graphics; microSPARC-II
  appendix A describes the static PRES-L pin. Original pinned OBP
  `afx_present-pa`/`afx-slot-empty?` establishes the board byte address and bit-0
  empty-slot check; its broader register behavior remains unverified. The selected 128 MiB
  RAM configuration is a project default, not a claim about factory base RAM.
- Local `../iris`, inspected at `b23f674c235b37c3ac83808085d28ac128bb79c4`:
  `HACKING.md`, `src/traits.rs`, and the machine/frontend boundaries. Architectural
  inspiration includes word-transparent data paths, functional hardware
  behavior, machine-owned wiring and headless diagnostics. Helios has no build
  dependency on IRIS.

SCSI target command layouts also follow the archived SCSI-2 standard:
[clause 8, all devices](https://www.staff.uni-mainz.de/tacke/scsi/SCSI2-08.html)
for INQUIRY, REQUEST SENSE and MODE SENSE;
[clause 9, direct-access devices](https://www.staff.uni-mainz.de/tacke/scsi/SCSI2-09.html)
for block addressing, READ(6)/(10) and READ CAPACITY; and
[clause 14, CD-ROM](https://www.staff.uni-mainz.de/tacke/scsi/SCSI2-14.html)
for removable type 5 and CD logical blocks. HELIOS inquiry strings describe
our synthetic targets, not a claim to reproduce a particular OEM drive.
Clause 8 sections 8.2.8–8.2.11 define six-/ten-byte mode commands, parameter-list
validation and current/default/saved modes. Clause 14.1.2 defines CD logical
block mapping; 14.3.3 defines the CD-specific mode header and density codes.
CD headers have no disk-style write-protect bit. The library models 512, 1024
and 2048-byte CD blocks and validates the entire MODE SELECT list before a change.
Clause 9's START STOP UNIT defines the readiness command used by OBP;
section 9.2.4 defines PREVENT ALLOW MEDIUM REMOVAL, its one-bit lock and
hard-reset clearing. Solaris locks the CD before mounting its root filesystem.
Sections 9.1.6, 9.2.18 and 9.2.20–9.2.21 define cache/FUA semantics,
SYNCHRONIZE CACHE and WRITE(6)/(10). The writable raw disk stages complete
commands and flushes host data before GOOD; it has no volatile write cache.
The [OpenBoot 2.x Command Reference](https://docs.oracle.com/cd/E19683-01/806-2906-10/806-2906-10.pdf),
page 70, documents `ttya output` for routing firmware/bootloader output to serial
without changing keyboard input.

SCSI-2 section 9.3.3.3/table 163 and 9.3.3.7/table 171 define the disk format
and rigid geometry pages. Helios advertises fixed virtual geometry, zero
changeable masks, and no saved pages. The pinned QEMU `scsi-disk.c` geometry
page was checked for field positions and disabled precompensation/reduced-current
encoding; its configurable host geometry does not define Helios's raw target.

## QEMU

Additional hardware timing references:

- [Sun-4M System Architecture, 950-1373-01](https://temlib.org/pub/SparcStation/Hardware/Sun4M_SystemArchitecture_edited2.pdf),
  July 19 1991 revision 50 as printed inside the document: section 5.3.2,
  page 24, inspected visually, defines zero-limit free-running without IRQ,
  overflow reload to 500 ns and the L flag in both counter and limit registers.
  This resolves the NCR timer disagreement described above.
- [SPARCengine 5 OEM Technical Manual](https://device.report/m/24e55e1efd74cebad92ef139a523192872b87937967e4567f9cdafcecf56e515.pdf),
  table 2-1, printed page 2-3: the 85 MHz board runs SBus at 21.25 MHz.
  NCR89C105 table 6-10, printed page 6-29 in the SBus I/O Chipset manual,
  specifies six SBus clocks per EPROM byte. These define physical PROM read
  service time, added to the functional CPU quantum without rounding each
  access separately. No ordinary-cache or pipeline timing is implied.
- [Linux v6.12 sunlance.c](https://github.com/torvalds/linux/blob/v6.12/drivers/net/ethernet/amd/sunlance.c):
  independent driver evidence for E_BASE_ADDR high-byte placement and the
  initialization block's MAC-byte pairs on SPARC.

Reference revision: `f71afb409a87c4ecd0f9d63a5963eb645dfab780` (inspected 2026-11-06).

| Source | Use |
| --- | --- |
| [hw/scsi/esp.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/scsi/esp.c) | Independent selection, CDB, data/status/message phase and IRQ comparison; NCR primary timeout/register semantics take precedence |
| [hw/audio/cs4231.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/audio/cs4231.c) | SS-5 CS4231 register array, indirect index/data bank, version bytes and control-reset comparison; audio streaming is not modeled |
| [hw/scsi/scsi-disk.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/scsi/scsi-disk.c) | MODE SELECT header lengths, descriptor parsing and block-size comparison; its per-device quirks do not define the HELIOS target |
| [hw/net/pcnet.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/net/pcnet.c) | LANCE-format DMA and packet/metadata byte-order comparison; PCnet extensions do not define NCR92C990 |
| [SPARC system overview](https://www.qemu.org/docs/master/system/target-sparc.html) | Machine/peripheral inventory and firmware context |
| [hw/sparc/sun4m.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/sparc/sun4m.c) | SS-5 profile, physical addresses, wiring and firmware interfaces |
| [target/sparc/cpu.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/cpu.c) | MB86904 identity, window count and reset state |
| [target/sparc/translate.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c) | Integer operations, control flow, FPU transfers/arithmetic dispatch, EF guards, status operations and STDFQ comparison |
| [target/sparc/win_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/win_helper.c) | Window management and privileged state |
| [target/sparc/mmu_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/mmu_helper.c) | Independent SRMMU walk, permissions, R/M and fault comparison |
| [target/sparc/ldst_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/ldst_helper.c) | ASI dispatch, control registers, cache/TLB flush and bypass comparison |
| [target/sparc/int32_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/int32_helper.c) | Synchronous trap entry and saved window state |
| [hw/char/escc.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/char/escc.c) | SS-5 port offsets/stride, byte MMIO and serial initialization comparison |
| [hw/rtc/m48t59.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/rtc/m48t59.c) | M48T08 variant, SRAM/clock decode and RTC comparison |
| [include/hw/nvram/sun_nvram.h](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/include/hw/nvram/sun_nvram.h) | IDPROM layout, XOR checksum and distinction from OpenBIOS configuration partitions |
| [hw/intc/slavio_intctl.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/intc/slavio_intctl.c) | Source bits/levels, soft requests, mask aliases and generic multiprocessor comparison |
| [hw/timer/slavio_timer.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/timer/slavio_timer.c) | Counter field layout, 500 ns ticks, acknowledgement and system/local IRQ wiring comparison |
| [hw/misc/slavio_misc.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/misc/slavio_misc.c) | Word-only system control, software-reset request and reset-status comparison |
| [hw/misc/empty_slot.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/misc/empty_slot.c) | Zero readback and discarded writes for unpopulated non-ECC RAM in the SS-5 profile |
| [hw/sparc/sun4m_iommu.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/sparc/sun4m_iommu.c) | IOMMU register/page-walk comparison; microSPARC-II differs in table alignment, shared TLB, DMA error routing, identity, six arbitration enables and shared slot 4 |

For memory behavior, prefer the microSPARC-II manual where the generic QEMU
implementation differs: 31-bit physical bypass, BM overriding EN for ASI 8/9,
the 28-bit boot offset, NF excluding both instruction spaces, destructive SFAR
reads, and exact-level probes. QEMU is an implementation comparison, not proof
of those hardware details. No differential execution has been performed.
For cache diagnostics, QEMU returns zero/ignores writes; Helios retains array
contents and invalidates indexed tags according to the primary manual.

For upcoming work, continue the review of `hw/sparc/sun4m_iommu.c`,
`hw/dma/sparc32_dma.c`, `hw/scsi/esp.c`, and `hw/net/lance.c`.
Verify each file at the selected revision before relying on its behavior.
QEMU's TCX framebuffer is a different device from the selected CGsix card.

## CGsix rendering

[Sun GX FBC Reference Manual 3.0 (January 1988)](https://bitsavers.trailing-edge.com/components/lsiLogic/sparc/GX/Sun_GX_FBC_Jan1988.pdf)
is the primary programming reference for the shared indexed vertex bank,
DRAW commands, polygon edge rules, RASTEROP selectors and font coordinates.
Section 3.2/table 3-1 was checked visually. The separate LSI SGX hardware manual
does not provide these complete programming rules. Original Solaris 2.6 Xsun
register traces exercise point/line/rectangle aliases and quadrilateral fills;
see [graphics.md](graphics.md). MAME remains a comparison, and QEMU's TCX is a
different framebuffer.

## MAME

The pinned [src/devices/machine/am79c90.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/machine/am79c90.cpp)
also supplies an independent LANCE descriptor, loopback and CRC comparison.
The same revision's [src/devices/machine/ncr53c90.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/machine/ncr53c90.cpp)
was checked for selection timeout, FIFO handshakes and transfer-count/IRQ
sequencing. Helios uses a functional byte interface rather than modeling each
SCSI signal edge.
The same revision's [src/devices/bus/nscsi/cd.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/nscsi/cd.cpp)
provides a separate CD block-size and MODE SELECT descriptor comparison.

CPU reference revision: `cd883a1be2fe0d120bfe594b0efcab5751ae565b`
(inspected 2026-11-06). A local checkout is also available at `../mame`; its
inspected revision was `ed59c15fe78596ec6ff8fe04482ede78e6122e34`.

| Source | Use |
| --- | --- |
| [src/devices/cpu/sparc/sparc.cpp](https://github.com/mamedev/mame/blob/cd883a1be2fe0d120bfe594b0efcab5751ae565b/src/devices/cpu/sparc/sparc.cpp) | Independent interpreter reference: arithmetic, register aliases, SAVE/RESTORE, branch annul, JMPL, tagged arithmetic, MULScc, doublewords and atomics |
| [src/devices/cpu/sparc/sparc.h](https://github.com/mamedev/mame/blob/cd883a1be2fe0d120bfe594b0efcab5751ae565b/src/devices/cpu/sparc/sparc.h) | CPU state and implementation structure |
| [src/mame/sun/sun4.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/mame/sun/sun4.cpp) | Sun machine context and device wiring |
| [src/devices/machine/z80scc.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/machine/z80scc.cpp) | Independent SCC pointer/reset/register comparison and manual commentary |
| [src/devices/machine/timekpr.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/machine/timekpr.cpp) | Independent timekeeper register/counter comparison |
| [src/devices/bus/sbus/cgsix.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/sbus/cgsix.cpp) | TurboGX ROM identity, ROP/mode fields, cursor and timing comparison; incomplete acceleration is not a correctness oracle |

The inspected MAME sun4m machine registrations are unfinished; they are not a
working SS-5 platform oracle. QEMU is the primary machine-level starting point,
and MAME remains useful as an independent CPU/device reference. MAME's sun4c
MMU and timer/interrupt wiring must not be substituted for sun4m's. The local
SPARC CPU implementation was also checked for ET/PIL gating and level-15 entry.
Its FPU load decoder masks odd double-register encodings and was compared for
word order and EF checks. Its generic LDFSR mask does not establish microSPARC-II
read-only status fields; the chip's table 6 takes precedence. QEMU's empty-FQ
check precedes the actual memory store; Helios explicitly checks STDFQ alignment
before the empty-queue exception, following V8's priority and pseudocode.
QEMU's `gen_trap_iffpexception` explicitly documents its immediate-trap shortcut.
Helios instead keeps the microSPARC-II pending state until the next floating
instruction, including stores and branches. The chip's section 4.4 permits
branches in exception state and defers a new sequence error for loads/FPops.
MAME's comparison dispatch was inspected, but the primary V8 rules define quiet
versus signaling NaN behavior and preservation of FCC when invalid traps are enabled.
For division, the same MAME source delegates to SoftFloat but checks a raw zero
word before FDIVs; that special path does not cover negative zero. QEMU's pinned
[target/sparc/cpu.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/cpu.c)
uses generic SPARC NaN defaults and a signaling-first, rs2-before-rs1 propagation
rule. Helios instead follows microSPARC-II figure 4.8 for quiet-operand priority
and ME_NaN. Neither emulator is used as the golden oracle for these chip details;
the independent fraction-based arithmetic checker and directed tests are described
in [fpu.md](fpu.md). The pinned MAME FADDs/d, FSUBs/d and FMULs/d dispatch delegates
to SoftFloat. QEMU's corresponding `do_env_fff`/`do_env_ddd` dispatch reads both
operands before calling helpers and writing the result. Helios implements the
integer arithmetic independently and retains its microSPARC-II deferred state
machine; no pipeline timing or QEMU/MAME differential conformance is claimed.
FITOS/FITOD follow V8 B.34's signed 32-bit input semantics. MAME's
[src/devices/cpu/sparc/sparc.cpp at ed59c15](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/cpu/sparc/sparc.cpp)
`i32_to_f32`/`i32_to_f64` and QEMU's
[target/sparc/fop_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/fop_helper.c)
`int32_to_float32`/`int32_to_float64` calls were checked independently; Helios
constructs the bits directly and uses its existing deferred exception path.

## Provenance

The initial Rust implementation and fixtures are newly written. No MAME, QEMU
or IRIS implementation files are vendored, and no firmware is bundled. Hardware
constants and instruction behavior are documented with their references above.
Future source reuse must retain attribution and comply with the particular
file's license; consult file headers instead of assigning one license to all
reference projects. Firmware fetch sources, hashes and initial bounded runs are
recorded in [firmware.md](firmware.md); the binaries remain under ignored local/.
Original Sun OBP is the primary development baseline. Serial and graphical
firmware prompts have been demonstrated; no differential reference run has
been performed. [Graphics references](graphics.md#evidence-and-references)
record Linux's CG6 driver, the LSI SGX/BT458 manual, Sun FCode and the keyboard
specification used for the graphical console implementation.

