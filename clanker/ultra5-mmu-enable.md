# Ultra 5 executes the relocated OBP from DRAM

The soft-marker original-ROM regression withdraws I-TLB slot 2 on a
probe CPU at the MMU transition and observes a fetch miss. On the intact
CPU the first translated fetch retires `membar #Sync` and marks slot 2
Used. The next `flush [%g0]` marks D-TLB slot 3 Used without reading a
device. The lookup supports all four IIi page sizes and selects the
primary context at TL zero and nucleus context zero above it. Diagnostic
I-TLB reads and writes observe the fetch Used bit in instruction order.
[IIi and QEMU sources](../docs/references.md) · [Installed mappings](../docs/ultra5.md#mappings-at-mmu-enable)

The guest copies 32,768 xwords from flash into PA `0x3ff00000`. The
regression checks every xword, both adjacent sentinels, and the identity
of the first RAM instruction by temporarily replacing it on a probe run.
Both diagnostic paths print the old and relocated PCs and reach
`Decompressing into Memory`. Both quiet paths have empty serial output.

| Reset | Diagnostic instructions | Quiet instructions |
| --- | ---: | ---: |
| Cold | 31,271,761 | 31,265,109 |
| Soft marker | 720,669 | 714,393 |

All four runs stop before `0xf024226c` at PC/nPC `0x26fc/0x2700`. This
guest store would write `0x00002a80` at PA `0x3ff4f26c` through the
VA-zero D-TLB mapping. Exercise word, halfword and byte writes as the
decompressor requests them, including preservation of adjacent DRAM lanes.
MMU misses/protection trap entry, fault status, automatic replacement and
cache timing are pending. Original ROM SHA-256 is
`fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`.
Validation passed 907 standard test executions, all 20 explicit local-ROM
checks, formatting, offline all-target Clippy with warnings denied, and
the four built-in demos. Bounded guest serial transcripts and logs are in
`local/ultra5-mmu-enable-2026-11-22/`.
