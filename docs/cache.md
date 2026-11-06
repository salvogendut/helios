# microSPARC-II cache diagnostics

Helios implements diagnostic storage for the 16 KiB instruction cache and 8 KiB
data cache, with 512 tags each. Diagnostic accesses retain data, decode aliases
and honor the access widths below. These arrays are owned by `Srmmu` alongside
the CPU's other control spaces; they are not physical RAM or MMIO mappings.

This is a diagnostic-storage subset. Ordinary instruction/data accesses still
use the coherent physical bus after MMU translation. They do not fill these
arrays or consult their tags, including when PCR.IE/DE are set. Injecting a valid
tag does not redirect normal accesses. Cache-hit protection, virtual aliases,
write allocation, streaming, diagnostic-load effects on the D-cache fill path,
parity, write buffering and ASI `39` remain unimplemented. The arrays alone do
not establish working cache POST or a complete hardware cache model.

## Diagnostic spaces

| ASI | Storage | Address index | Supported loads/stores |
| --- | --- | --- | --- |
| `0c` | Instruction tags, 512 entries | VA[13:5] | Word |
| `0d` | Instruction data, 4096 words | VA[13:2] | Word |
| `0e` | Data tags, 512 entries | VA[12:4] | Word |
| `0f` | Data bytes, 8192 bytes | VA[12:0] | Byte, halfword, word, doubleword |

Normal CPU alignment and alternate-space privilege checks precede accesses.
Other address bits are ignored after alignment checking. Thus adjacent aligned
words within a line alias its tag, and high addresses wrap at the array size.
Data accesses ignore context, ACC and validity; writing data does not validate
a tag. Words retain guest bit patterns, narrow lanes are big endian, and the
lower-address word comes first in a doubleword pair. Doublewords complete
indivisibly. Unsupported widths and all diagnostic atomics stop before effects,
without CPU error state or changes to SFSR/SFAR; PCR.NF does not hide them.

| Tag field | Instruction | Data |
| --- | --- | --- |
| Virtual address | Bits 31:14 | Bits 31:13 |
| Reserved | Bits 13:12 | Bit 12 |
| Context | Bits 11:4 | Bits 11:4 |
| ACC | Bits 3:1 | Bits 3:1 |
| Valid | Bit 0 | Bit 0 |

Reserved fields read zero and ignore writes. Tags and data start at zero and
are cleared by both modeled CPU reset paths. Deterministic zero RAM and reserved
readback are emulator choices, not claims about unspecified silicon RAM contents
after reset. PCR/context/TLB changes do not themselves clear diagnostic storage.

## Line invalidation

Word stores to ASIs `10`–`14` inspect one indexed line in **each** cache. A match
clears only Valid; the remaining tag fields and data are retained. The stored
value is irrelevant. These operations do not translate addresses, access the
bus or invalidate the TLB. ACC 6/7 denotes a supervisor-only, context-independent
line; ACC 0–5 denotes a user line.

| ASI | Scope | Additional match criteria |
| --- | --- | --- |
| `10` | Page | VA[31:12] and either supervisor line or current context |
| `11` | Segment | VA[31:18] and either supervisor line or current context |
| `12` | Region | VA[31:24] and either supervisor line or current context |
| `13` | Context | User line and current context |
| `14` | User | User line, any context |

The selected index already supplies VA bits below the stored address tag.
Context/user flushes also affect only the indexed line; software must loop over
indices to flush a whole cache. Unsupported flush loads/widths remain stops.

The unprivileged `FLUSH` instruction invalidates only the indexed instruction
tag when its stored virtual address matches. Low address bits are ignored.
It conservatively includes matching virtual addresses from other contexts,
permitted by V8's broader invalidation allowance. It has no data-cache, TLB or
physical-bus effects. Prior normal stores remain immediately visible to fetches
under the existing coherent memory model.

## Evidence

The [microSPARC-II User's Manual](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf),
revision 1.1 July 1994, sections 6.1–6.3, 6.9, 6.15, 7.1–7.3, 7.8 and 12.1
define the arrays, widths, fields and flush criteria. Figures 6.1/7.1 were
inspected visually because extracted text omits their bit positions. Table 55's
segment criterion has a reversed `13:18` typo; SPARC V8 appendix I table I-2
confirms `31:18`. V8 B.32 defines `FLUSH` and permits broader invalidation.

At the [pinned QEMU revision](references.md), `target/sparc/ldst_helper.c`
returns zero for these diagnostic loads and ignores stores/flushes. Helios
retains diagnostic contents and applies indexed invalidation. The inspected
MAME driver has no working microSPARC-II cache implementation to use as an
oracle; sun4c cache layouts are not applicable.

`tests/cache.rs` checks array boundaries and aliases, reserved masks, narrow
lanes, doubleword order/atomicity, all five flush scopes across ACC/context/VA
matches, guest privilege and precise stops, `FLUSH`, and both reset paths.
These fixtures are independent of firmware. [Firmware records](firmware.md)
describe the separate original-OBP probe and its next limitations.
