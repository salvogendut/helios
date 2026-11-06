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
- [Sun microSPARC-II User's Manual, revision 1.1, July 1994](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf):
  primary hardware manual hosted by Temlib. Chapter 5 (tables 9, 18, 19, 23–28
  and 40) and chapter 12 define the target's 31-bit addressing, contexts,
  permissions, probe/flush, fault state, NF, boot mapping and ASIs. The PCR
  figure/reserved-bit wording and root PTE descriptions have inconsistencies;
  [memory.md](memory.md) states the implemented choices and limits. Sections
  3.11.3 and 10.2 distinguish power-up, non-power-up and watchdog reset;
  [reset.md](reset.md) describes the functional subset and recovery limit. Sections
  3.5–3.6 define integer multiply/divide, including negative division-overflow
  method B on printed page 41. Sections 6.9 and 7.8 describe FLUSH/cache behavior.
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
- [NCR89C105 Chip Specification](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/NCR89C105.txt),
  primary chip documentation preserved in the historic Linux/SPARC archive and
  referenced by QEMU: table 6-7 maps both SCC instances and reset/status;
  figure 6-6 defines SR/RS/WD. Tables 6-21 through 6-25 and figures 6-8 through 6-18
  define interrupt pending/masks, source levels, reset and counter timers.
  The archived text has no revision header. Its explicit zero-limit overflow
  interrupt rule differs from QEMU. Mask All's treatment of software requests
  is less explicit; [interrupts-timers.md](interrupts-timers.md) records the choice.
- [Sun SPARCclassic Engine OEM Technical Manual, Part One, April 1993](https://bitsavers.trailing-edge.com/pdf/sun/sparc/801-3137-10_SPARCclassic_Engine_OEM_Technical_Manual_Part_One_Apr1993.pdf),
  801-3137-10: section 5's NCR89C105 overview and section 7's SLAVIO address map
  independently confirm the uniprocessor layout at `0x71d00000`/`0x71e00000`.
  This is a related machine's primary manual; QEMU's SS-5 profile confirms the
  same bases for the selected target. Do not import its CPU or RAM profile.
- [Sun SPARCstation 5 Service Manual, August 1994](https://docs.oracle.com/cd/E19127-01/sparc5.ws/801-6396-11/801-6396-11.pdf),
  801-6396-11: table 12-1 lists the 85 MHz system board, SCSI disk/CD-ROM options
  and 1 MiB TGX card for the selected initial profile. Chapters 2–4 cover POST,
  diagnostics and the serial tip connection; this is also the manual supplied
  by the user during bring-up.
- [Sun SPARCengine 5 OEM Technical Manual, September 1995](https://device.report/m/24e55e1efd74cebad92ef139a523192872b87937967e4567f9cdafcecf56e515.pdf),
  801-7104-11: table 2-1 covers microSPARC-II clock variants; section 3.2.9.2
  describes the 1 MiB TurboGX SBus card with BT458 RAMDAC. The selected 128 MiB
  RAM configuration is a project default, not a claim about factory base RAM.
- Local `../iris`, inspected at `b23f674c235b37c3ac83808085d28ac128bb79c4`:
  `HACKING.md`, `src/traits.rs`, and the machine/frontend boundaries. Architectural
  inspiration includes word-transparent data paths, functional hardware
  behavior, machine-owned wiring and headless diagnostics. Helios has no build
  dependency on IRIS.

## QEMU

Reference revision: `f71afb409a87c4ecd0f9d63a5963eb645dfab780` (inspected 2026-11-06).

| Source | Use |
| --- | --- |
| [SPARC system overview](https://www.qemu.org/docs/master/system/target-sparc.html) | Machine/peripheral inventory and firmware context |
| [hw/sparc/sun4m.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/sparc/sun4m.c) | SS-5 profile, physical addresses, wiring and firmware interfaces |
| [target/sparc/cpu.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/cpu.c) | MB86904 identity, window count and reset state |
| [target/sparc/translate.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c) | Integer flags, multiply/divide, tagged arithmetic, doublewords, atomics and control transfers |
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

For memory behavior, prefer the microSPARC-II manual where the generic QEMU
implementation differs: 31-bit physical bypass, BM overriding EN for ASI 8/9,
the 28-bit boot offset, NF excluding both instruction spaces, destructive SFAR
reads, and exact-level probes. QEMU is an implementation comparison, not proof
of those hardware details. No differential execution has been performed.

For upcoming work, start with `hw/sparc/sun4m_iommu.c`,
`hw/dma/sparc32_dma.c`, `hw/scsi/esp.c`, and `hw/net/lance.c`.
Verify each file at the selected revision before relying on its behavior.
QEMU's TCX framebuffer is a different device from the selected CGsix card.

## MAME

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
| [src/devices/bus/sbus/cgsix.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/sbus/cgsix.cpp) | CGsix/TurboGX device family and SBus-relative map starting point; full behavior review pending |

The inspected MAME sun4m machine registrations are unfinished; they are not a
working SS-5 platform oracle. QEMU is the primary machine-level starting point,
and MAME remains useful as an independent CPU/device reference. MAME's sun4c
MMU and timer/interrupt wiring must not be substituted for sun4m's. The local
SPARC CPU implementation was also checked for ET/PIL gating and level-15 entry.

## Provenance

The initial Rust implementation and fixtures are newly written. No MAME, QEMU
or IRIS implementation files are vendored, and no firmware is bundled. Hardware
constants and instruction behavior are documented with their references above.
Future source reuse must retain attribution and comply with the particular
file's license; consult file headers instead of assigning one license to all
reference projects. Firmware fetch sources, hashes and initial bounded runs are
recorded in [firmware.md](firmware.md); the binaries remain under ignored local/.
Original Sun OBP is the primary development baseline. No differential reference
run or working firmware prompt has been demonstrated.

