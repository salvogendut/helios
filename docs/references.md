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
- [Sun microSPARC-II User's Manual, revision 1.1, July 1994](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf):
  primary hardware manual hosted by Temlib. Chapter 5 (tables 9, 18, 19, 23–28
  and 40) and chapter 12 define the target's 31-bit addressing, contexts,
  permissions, probe/flush, fault state, NF, boot mapping and ASIs. The PCR
  figure/reserved-bit wording and root PTE descriptions have inconsistencies;
  [memory.md](memory.md) states the implemented choices and limits. Sections
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

For memory behavior, prefer the microSPARC-II manual where the generic QEMU
implementation differs: 31-bit physical bypass, BM overriding EN for ASI 8/9,
the 28-bit boot offset, NF excluding both instruction spaces, destructive SFAR
reads, and exact-level probes. QEMU is an implementation comparison, not proof
of those hardware details. No differential execution has been performed.

For upcoming work, start with `hw/sparc/sun4m_iommu.c`,
`hw/intc/slavio_intctl.c`, `hw/timer/slavio_timer.c`,
`hw/dma/sparc32_dma.c`, `hw/scsi/esp.c`, `hw/net/lance.c`, and `hw/display/tcx.c`.
Verify each file at the selected revision before relying on its behavior.

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

The inspected MAME sun4m machine registrations are unfinished; they are not a
working SS-5 platform oracle. QEMU is the primary machine-level starting point,
and MAME remains useful as an independent CPU/device reference. MAME's sun4c
MMU must not be substituted for sun4m's SRMMU.

## Provenance

The initial Rust implementation and fixtures are newly written. No MAME, QEMU
or IRIS implementation files are vendored, and no firmware is bundled. Hardware
constants and instruction behavior are documented with their references above.
Future source reuse must retain attribution and comply with the particular
file's license; consult file headers instead of assigning one license to all
reference projects. No differential reference run has been performed yet.

