# SS-5 DRAM and TLB diagnostics

## DRAM bank map

`Machine::ss5` and `ss5_with_nvram` construct the hardware memory layout used by
the CLI `boot` command. Eight 32 MiB apertures occupy physical `00000000` through
`0fffffff`. Each slot holds an 8 MiB or 32 MiB DSIMM, or is empty. The selected
128 MiB profile populates slots 0–3 with 32 MiB modules; slots 4–7 are empty.

The size-based constructor fills low slots with 32 MiB modules, then 8 MiB
modules. A 16 MiB configuration is two 8 MiB modules at `00000000` and
`02000000`, rather than contiguous 16 MiB. Totals must be nonzero multiples of
8 MiB that fit eight slots. For example, 240 MiB cannot be built with these
module sizes. `Ss5Dram::new` accepts an explicit eight-slot layout for library
users. The service manual's 16 MiB factory minimum is not a hardware restriction
imposed by this model; a single 8 MiB module is allowed.

An 8 MiB module mirrors four times within its aperture. All normal widths,
doublewords and atomic exchanges use the same underlying words. Empty-bank
accesses complete: reads return zero, writes have no storage to modify. Zero is
a deterministic choice for hardware-undefined data; parity-error injection and
the memory fault registers remain pending. This agrees with QEMU's empty-slot
read value but follows the microSPARC-II bank decode, including smaller-module
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

ASI `06` accesses the same 64 entries used by CPU translation. Word accesses
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
unimplemented. IOPTEs are excluded from CPU leaf matching; DVMA is still pending.
Diagnostic edits detach the entry from its software page-table address. Injected
CPU leaves support reads and, with tag M=1, writes. An M=0 store to a detached
entry stops before mutation: the hardware rewalk needed to locate/update its
backing PTE is not yet modeled. These stops are not suppressed by PCR.NF and
do not fabricate SFSR/SFAR failures.

## Replacement control

ASI `04`, VA[12:8]=`10`, selects TRCR. The mask is `01f1ffff`, following figure
5.15 rather than the adjacent reserved-bit typo. Fields are SBSP[24:23],
MEMSP[22:21], VP[20], PL[16:14], IL[13], WP[12:7], TC[6], TLBRC[5:0].

The functional replacement counter advances on CPU leaf fills. TC freezes it;
WP sets its inclusive wrap point. IL reserves entries 48–63 for I/O. PL redirects
CPU fills from reserved slots 0/1/2 into alternate slots 3/4/5. These affect
actual replacement, not merely register readback. Flushes do not reset TLBRC.
The model clamps the CPU wrap point to 47 with IL set and reduces out-of-range
counter values modulo the eligible range. These are deterministic choices for
inconsistent guest configurations; software should set compatible values.

The real counter advances on CPU clocks; this emulator does not provide that
timing. Reset selects WP=63, TLBRC=0 and other fields zero as a functional
baseline. Speed fields are latches with no clock effect. VP is retained, but
normal walks do not cache intermediate PTPs. I/O replacement and exact silicon
reset/speed-strap readback remain pending.

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
