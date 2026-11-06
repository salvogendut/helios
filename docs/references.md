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
- [Sun microSPARC-II User's Manual, revision 1.1, July 1994](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf):
  primary hardware manual hosted by Temlib. Chapter 5 (tables 9, 18, 19, 23–28
  and 40) and chapter 11 define the target's 31-bit addressing, contexts,
  permissions, probe/flush, fault state, NF, boot mapping and ASIs. The PCR
  figure/reserved-bit wording and root PTE descriptions have inconsistencies;
  [memory.md](memory.md) states the implemented choices and limits.
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
| [target/sparc/translate.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c) | Integer flags, decoding and control transfers |
| [target/sparc/win_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/win_helper.c) | Window management and privileged state |
| [target/sparc/mmu_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/mmu_helper.c) | Independent SRMMU walk, permissions, R/M and fault comparison |
| [target/sparc/ldst_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/ldst_helper.c) | ASI dispatch, control registers, cache/TLB flush and bypass comparison |
| [target/sparc/int32_helper.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/int32_helper.c) | Synchronous trap entry and saved window state |

For memory behavior, prefer the microSPARC-II manual where the generic QEMU
implementation differs: 31-bit physical bypass, BM overriding EN for ASI 8/9,
the 28-bit boot offset, NF excluding both instruction spaces, destructive SFAR
reads, and exact-level probes. QEMU is an implementation comparison, not proof
of those hardware details. No differential execution has been performed.

For upcoming work, start with `hw/sparc/sun4m_iommu.c`,
`hw/intc/slavio_intctl.c`, `hw/timer/slavio_timer.c`, `hw/char/escc.c`,
`hw/dma/sparc32_dma.c`, `hw/scsi/esp.c`, `hw/net/lance.c`, and `hw/display/tcx.c`.
Verify each file at the selected revision before relying on its behavior.

## MAME

CPU reference revision: `cd883a1be2fe0d120bfe594b0efcab5751ae565b`
(inspected 2026-11-06). A local checkout is also available at `../mame`; its
inspected revision was `ed59c15fe78596ec6ff8fe04482ede78e6122e34`.

| Source | Use |
| --- | --- |
| [src/devices/cpu/sparc/sparc.cpp](https://github.com/mamedev/mame/blob/cd883a1be2fe0d120bfe594b0efcab5751ae565b/src/devices/cpu/sparc/sparc.cpp) | Independent interpreter reference: arithmetic, register aliases, SAVE/RESTORE, branch annul and JMPL |
| [src/devices/cpu/sparc/sparc.h](https://github.com/mamedev/mame/blob/cd883a1be2fe0d120bfe594b0efcab5751ae565b/src/devices/cpu/sparc/sparc.h) | CPU state and implementation structure |
| [src/mame/sun/sun4.cpp](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/mame/sun/sun4.cpp) | Sun machine context and device wiring |

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

