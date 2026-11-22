# Ultra 5 physical DRAM discovery

The reference hardware reports an empty pair 0 and 256 + 256 MiB in pair 2.
The IIi MCU uses PA29 for the second rank and PA14 for column 10 of
these DIMMs. With 10-bit addressing PA14 also drives row bit zero.
The emulator folds the documented 1 GiB DRAM wrap within the low 4 GiB
cacheable range. Higher cacheable addresses stop explicitly. Ordinary
E-cache fills and ECC remain outside the fixture.
[Hardware and manual sources](../docs/ultra5.md#physical-dram-discovery)

At flash offset `0x126c` the ROM returns `%o0 = 0x1a0` and `%o1 = 1`.
The low seven size bits give 32 units of 8 MiB per DIMM, bit 7 marks
11-bit columns and bit 8 marks two ranks. The regression seeds both ranks
and checks the restored patterns in both column modes. At `0x1444` the
guest passes base `0x30000000` and extent `0x10000000` to its clear
routine. It zeros the xwords at both ends and the middle of the last
1 MiB while preserving the immediately preceding xword.

| Reset | Diagnostic steps | Quiet steps |
| --- | ---: | ---: |
| Cold | 31,138,089 | 31,133,297 |
| Soft marker | 586,997 | 582,581 |

All four runs stop on word `0xd0f008a0` with low-alias PC/nPC
`0x01fff0001f90` / `0x01fff0001f94`. The LSU store requests IM/DM
enable. A second original-ROM fixture checks the three I-TLB and five
D-TLB entries installed by the routine at `0x83c`, including size,
base, tag, context, low attributes and all remaining invalid slots.
The two I-TLB page tags ending in `f0000000` must remain distinct when
implementing translated instruction fetch. [Verified page map](../docs/ultra5.md#mappings-at-mmu-enable)

Validation passed 903 standard test executions, all 19 explicit local-ROM
checks, formatting, offline all-target Clippy with warnings denied, and
all four demos. The serial transcripts and validation logs are in
`local/ultra5-dram-probe-2026-11-22/`.
