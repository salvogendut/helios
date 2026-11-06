# microSPARC-II SBus and IOMMU controls

Helios implements the configuration registers needed by early SS-5 OBP
initialization. This is a register subset, **not a working DVMA translation
engine**. Enabling IOMMU translation stops explicitly before changing state.

The primary source is the microSPARC-II User's Manual revision 1.1, sections
5.7.1–5.7.2, 5.7.7, 5.7.10 and 9.4. Table 30 and figures 5.21/5.23 were also
checked visually because their bit labels are missing from the PDF text.
[References](references.md) link the manual and pinned QEMU comparison.

| Physical address | Register | Implemented behavior |
| --- | --- | --- |
| `0x10000000` | I/O MMU control | IMPL=0, VER=4; range bits 4:2 writable; EN=1 unsupported |
| `0x10000004` | I/O page-table base | Bits 26:10 writable, others zero; represents physical base bits 30:14 |
| `0x10001010`–`0x1000101c` | Slot configuration 0–3 | SA30 bit 16, BA8 bit 1, bypass bit 0 writable |
| `0x10001020` | Shared slot configuration 4 | BA8 bit 1 writable; bypass and SA30 zero |
| `0x10002000` | MID / SBus arbitration | MID reads 8; six enable bits at 21:16 writable |

Writes ignore reserved and read-only identity bits. The sixth arbitration bit
controls the SCSI/Ethernet master. Masters 4 and 5 share slot register 4 and
cannot bypass the IOMMU. The fields retain guest configuration; no DMA engine
or SBus master currently consumes them. Do not treat the existence of these
registers as permission for future DMA code to access untranslated guest RAM.

The physical bus reserves `0x10000000`–`0x10007fff` for the control block.
Accesses require aligned words. Bytes, halfwords, doublewords, masked stores,
atomics and raw-image writes stop without effects. RAM/ROM cannot overlap this
aperture. Wider control-space decoding and mirrors remain unimplemented.

Known but unimplemented facilities return an emulator stop: I/O TLB flush
ports (`+0x14`, `+0x18`), asynchronous fault registers (`+0x1000`, `+0x1004`),
memory fault capture (`+0x1050`, `+0x1054`), performance/breakpoint registers
(`+0x3000`–`+0x301c`) and local-graphics queue controls (`+0x4000`, `+0x6000`,
`+0x7000`). Reserved holes return an unmapped bus error. No generic writable
register array stands in for unsupported behavior.

Power-on and guest software reset clear all writable fields while retaining the
fixed identity values. These registers belong to `SystemBus`'s `IoControl`;
the CPU's `Srmmu` remains responsible for CPU translations. I/O TLB ownership,
DVMA access checks, error latching and asynchronous interrupt wiring still need
implementation together with the DMA path.

The primary manual differs from QEMU's generic sun4m model in several places:

- Figure 5.23 puts six SBAE bits at 21:16; the six-master arbiter description
  agrees. The nearby prose incorrectly includes bit 21 among reserved bits.
  Helios follows the diagram and description; QEMU masks to five bits.
- The manual hardwires IOCR version 4; QEMU's SS-5 profile supplies version 5.
  Helios uses the documented microSPARC-II value and records the discrepancy.
- microSPARC-II has five slot registers, with special rules for the last.
  QEMU explicitly handles four and provides a generic MBus arbiter register at
  `+0x1008`. Table 30 has no such register for this CPU; Helios leaves that hole
  unmapped.

Tests exercise field masks, every arbitration enable, slot independence, reset,
guest ASI `0x20` accesses, translated write protection and precise rejection of
unsupported operations, including under the CPU MMU's no-fault mode.
The latest original-OBP probe passes this block and reaches data-cache tag
initialization; see [firmware.md](firmware.md).
