# SS-5 DRAM and TLB diagnostics

The microSPARC-II TRCR reports the SS5/85 board's speed pins: SBSP[24:23]=2
(CPU/4, 21.25 MHz SBus) and MEMSP[22:21]=1 (85 MHz memory timing). These fields
are read-only, survive reset and are independent of the writable replacement
controls. Firmware previously saw zero and incorrectly advertised divide-by-2.
Figure 5.15 and tables 29/30 of the microSPARC-II manual define these fields;
the figure and field descriptions override the stale reserved-bits sentence
which overlaps them. The SS-20/Axil reference-MMU path has no such SS-5 pins.

## DRAM bank map

`Machine::ss5` and `ss5_with_nvram` construct the hardware memory layout used by
the CLI `boot` command. Eight 32 MiB apertures occupy physical `00000000` through
`0fffffff`. Each slot holds an 8 MiB or 32 MiB DSIMM, or is empty. The selected
128 MiB profile populates slots 0–3 with 32 MiB modules; slots 4–7 are empty.

The size-based constructor fills low slots with 32 MiB modules, then 8 MiB
modules. A 16 MiB configuration is two 8 MiB modules at `00000000` and
`02000000`, rather than contiguous 16 MiB. Totals must be nonzero multiples of
8 MiB that fit eight slots. For example, 232 MiB fits seven 32 MiB and one 8 MiB
module, while 240 MiB cannot be built. `Ss5Dram::new` accepts an explicit
eight-slot layout for library users. The service manual's 16 MiB factory minimum
is not a hardware restriction imposed by this model; a single 8 MiB module is
allowed.

`DramLayout` validates a DSIMM population without allocating guest storage.
The `ss5_total`, `ss20_total`, `axil_total` and corresponding `valid_*_totals`
methods expose exact representable sizes. SS-20 uses eight 64 MiB apertures
with 16/32/64 MiB modules. With built-in SX, bank 7 holds the VSIMM, so seven
banks hold DRAM and the maximum is 448 MiB. Without a VSIMM, all eight banks
are available (512 MiB). The Axil also has eight RAM sockets, two capable of
holding VSIMMs according to the owner; its current CGsix profile uses no VSIMM.
`MachineProfile::ram_layout` applies these rules before
`Ss5Dram::from_layout` allocates storage.

The Axil profile uses **16/64 MiB modules** with the pinned original OBP 2.19.
Instruction traces show that firmware testing the +16 MiB alias, without a
+32 MiB size probe in that pass: the former contiguous 32 MiB simulation was
reported as 64 MiB. Therefore
32 MiB total uses two 16 MiB modules, and 416 MiB uses six 64 MiB plus two
16 MiB modules. Totals 432, 480 and 496 MiB do not fit eight such sockets and
are rejected; 512 MiB remains supported. This is a limitation of the emulated
profile and pinned firmware, not a physical module inventory for every Axil
revision. Smaller Axil modules, including the owner's suggested 8 MiB size,
need independent evidence and correct firmware discovery before being exposed.

SS-20's 32 MiB DSIMM consists of two distinct 16 MiB regions at bank offsets
0 and 32 MiB, with PA24 mirrored in each region. Original OBP 2.25R publishes
the two ranges separately in `/memory`'s `reg` property. Treating this module
as contiguous 32 MiB with modulo-32 addressing aliases the second published
range onto the first and corrupts guest memory. CPU lanes, doublewords, atomics,
IOMMU tables and DMA all use the same split mapping. Raw imports may address
either real region but cannot cross into a mirror.

At power-on, Helios resolves each physical 8 MiB window to its backing RAM
region and word offset. CPU and DMA accesses share these mappings. On the
standard boards, ordinary byte, halfword and word accesses to low DRAM go
directly to the bank decoder. Custom bus layouts use the full device decoder
so a relocated peripheral can still take precedence over RAM.

An 8 MiB module mirrors four times within its aperture. All normal widths,
doublewords and atomic exchanges use the same underlying words. Empty-bank
accesses complete: reads return zero, writes have no storage to modify. Zero is
a deterministic choice for hardware-undefined data. [Memory-request capture](fault-capture.md)
is implemented, while parity storage/checking and error generation remain pending.
This agrees with QEMU's empty-slot read value but follows the microSPARC-II bank decode, including smaller-module
mirrors. Addresses outside the DRAM aperture retain normal device/bus decoding.

Reset preserves populated storage and bank layout. The whole aperture is
reserved against overlapping RAM/ROM maps. Raw image imports must fit real
storage within one populated bank; they cannot silently wrap through aliases,
disappear into empty slots or partially modify RAM on failure.

`Machine::bare_metal` and the existing `sun4m`/`sun4m_with_nvram` synthetic
harnesses retain contiguous, strictly bounded RAM of arbitrary word-aligned
size. They support small test fixtures and deliberate unmapped-access tests.
Use `ss5` for hardware firmware probes. The library reports the installed slot
sizes through `SystemBus::dram().bank_sizes()`.

## TLB diagnostic ports

ASI `06` accesses the same 64 entries used by CPU and I/O translation. Word accesses
only; CPU privilege, encoding and alignment checks still apply.

| Address | Contents |
| --- | --- |
| `000`–`0fc` | Entry 0–63 PTE data, four-byte stride |
| `100`–`1fc` | Entry 0–63 lower tag |
| `200`–`2fc` | Reserved, explicit unsupported-register stop |
| `300`–`3fc` | Entry 0–63 upper tag |
| `400` and above | Reserved, explicit unsupported-register stop |

The upper tag retains VA[31:12], context[11:4], Valid[3] and Level[2:0]. Level
codes `011`, `001`, `000` select region, segment and page matches. The manual
excludes root tags (level bit 2 set) from CPU lookup. The lower tag retains
UE/UR/UW/SE/SR/SW in bits 9:4, supervisor-global in bit 3, I/O in bit 2,
PTP in bit 1 and Modified in bit 0. Bits 31:10 read zero.

Diagnostic data retains the implemented RAM bits. Bits 28:27 read zero and
bits 1:0 are synthesized as PTE/IOPTE `10` or PTP `01`. CPU PTE readback forces
Referenced=1; PTP readback clears bits 3:2; IOPTE readback clears bits 7:3.
Bits 31:29 hold the table level in the diagnostic data format. Normal fills use
`100`/`110`/`111` for levels 1/2/3. ASI `03` probe results retain the existing
memory-PTE format without those diagnostic level bits.

Normal walks and R/M updates are visible through these ports. Diagnostic edits
affect subsequent hits, including the separate tag permission bits, context,
physical mapping and validity. Cache flushes remain independent. MMU flushes
clear Valid while retaining diagnostic contents and the replacement counter.
Power-on/software reset clears storage; synthesized PTE readback bits still
read as specified above.

Diagnostic tests can retain arbitrary patterns while translation is disabled.
Active injected PTPs and reserved CPU level code `010` cause an explicit host
stop before translated access or an entire probe. PTP-assisted walks remain
unimplemented. IOPTEs are excluded from CPU leaf matching. DMA uses their
4 KiB VA tags without context comparison, PPN and W bit; injected IOPTEs
work through the same ports. See [I/O controls](io-control.md).
Diagnostic edits detach the entry from its software page-table address. Injected
CPU leaves support reads and, with tag M=1, writes. An M=0 store to a detached
entry stops before mutation: the hardware rewalk needed to locate/update its
backing PTE is not yet modeled. These stops are not suppressed by PCR.NF and
do not fabricate SFSR/SFAR failures.

## Replacement control

ASI `04`, VA[12:8]=`10`, selects TRCR. The mask is `01f1ffff`, following figure
5.15 rather than the adjacent reserved-bit typo. Fields are SBSP[24:23],
MEMSP[22:21], VP[20], PL[16:14], IL[13], WP[12:7], TC[6], TLBRC[5:0].

The functional replacement counter advances on CPU and I/O leaf fills. TC freezes it;
WP sets its inclusive wrap point. IL places I/O fills at `48 + (counter & 15)`
and limits CPU fills to 0–47. With IL clear, both use the same range. PL redirects
leaf fills from reserved slots 0/1/2 into alternate slots 3/4/5. These affect
actual replacement, not merely register readback. Flushes do not reset TLBRC.
The model clamps the CPU wrap point to 47 with IL set and reduces out-of-range
counter values modulo the eligible range. These are deterministic choices for
inconsistent guest configurations; software should set compatible values.

The CPU checks slot zero directly when all higher slots are invalid. As soon
as a higher slot can hold a valid tag it uses the full CPU lookup. The
integrated I/O search skips slots above the highest potentially valid tag.
A diagnostic write can install a valid tag in slot 63 even with WP=0, so
both lookups track populated slots independently of the replacement counter.

The real counter advances on CPU clocks; this emulator does not provide that
timing. Reset selects WP=63, TLBRC=0 and other fields zero as a functional
baseline. Speed fields are latches with no clock effect. VP is retained, but
normal walks do not cache intermediate PTPs. Exact silicon replacement timing
and reset/speed-strap readback remain pending.

## Sources and validation

The [microSPARC-II User's Manual](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf),
revision 1.1 July 1994, sections 8.2.1–8.2.2 define bank selection, mirroring and
empty-bank completion. Sections 5.2, 5.6.6 and 5.14.1, figures 5.4/5.6/5.8,
5.15/5.35/5.36 and table 41 define diagnostic fields and replacement controls.
The diagnostic tag/port diagrams and TRCR figure were inspected visually.
The [SS-5 Service Manual](https://docs.oracle.com/cd/E19127-01/sparc5.ws/801-6396-11/801-6396-11.pdf),
section 11.3, confirms eight slots with 8/32 MiB DSIMMs.

Pinned QEMU `hw/sparc/sun4m.c` maps an empty-slot region after configured RAM;
`hw/misc/empty_slot.c` returns zero and discards stores.
QEMU MB86904 TRCR mask is `00ffffff`; Helios uses the primary diagram's
`01f1ffff`, including the high SBus speed bit and excluding reserved fields.
The inspected MAME
sun4m registrations remain unfinished; their sun4c MMU is not an SS-5 oracle.
See [references.md](references.md) for revisions. No differential run is claimed.

`tests/dram.rs` checks layouts, mirrors, widths/atomics, empty banks, bounds,
image/mapping atomicity and reset. `tests/tlb_diagnostics.rs` checks all slots,
data formats, ordinary fills, injected mappings at all supported sizes,
permissions, invalidation, real replacement and precise unsupported behavior.
Existing translation/protection tests remain applicable. [Firmware records](firmware.md)
describe the separate original-OBP checkpoint.
