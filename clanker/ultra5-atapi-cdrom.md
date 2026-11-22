# Ultra 5 ATAPI CD-ROM and Solaris 7 boot

The serial-console checkpoint is `18b7a22`. The storage branch adds a
read-only packet device at the secondary master behind the CMD646 on Simba B.
The optical target shares the existing media backend with sun4m. The
original OpenBoot driver uses PIO and a 12-byte READ(12) packet. Its first
disk-label request is 32 blocks at LBA 0 with a 2,048-byte phase limit.
The transport bounds READ(12) staging to one block and checks the full LBA
range before exposing data.

## Reproduction and results

Use the command and OpenBoot device-method sequence in
[the Ultra 5 guide](../docs/ultra5.md#attach-a-solaris-installation-cd).
The input hashes and console transcripts are retained in the ignored
`local/ultra5-atapi-cdrom-2026-11-22/` directory.

| Input or observation | SHA-256 |
| --- | --- |
| Original OBP 3.31 flash | `fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616` |
| Serial NVRAM seed with autoboot disabled | `b32f818e9b392b152199ff7da18d75643656bde53f0de7699cf6b90d60094602` |
| SOL_7_1199_SPARC.iso, before and after | `d1709cf47fe560f780af1769732a52eca706c8a1a52367136f8a8a14dd276cbf` |
| Complete OBP sector at LBA 16, 2,048 bytes | `bddf7038838b226ae3365d70d6f10f0c4ccd9f3d460d9d19459b7d04c01a9b72` |

With no image, `probe-ide` reports four empty positions. With the ISO,
only device 2 identifies as a removable ATAPI CD-ROM. The ROM opens
`cdrom`, reports block size `0x800` and returns one block for LBA 16.
Eight 256-byte hex rows read out of guest memory compare byte-for-byte
with file offset `0x8000`. The serial output and the exact commands are
in `sector-and-boot.ttya` and `sector-and-boot.fth`.

After closing the handle, `boot cdrom` printed the OpenBoot device path.
The first unsupported operation was V9 memory op3 `0x09` at
PC/nPC `0x116524/0x116528`. A separate cold run with only `boot cdrom`
reached the same stop. The ISO's
`/Solaris_2.7/Tools/Boot/platform/sun4u/ufsboot` ELF has a loadable text
segment at VA `0x100000`, file offset `0xb0`. At file offset `0x165d4`
it contains `0xd8492000` (LDSB from `[%g4]` to `%o4`). Both transcripts
end before any Solaris loader message. This gives the next CPU regression
a specific instruction, PC and guest reproducer.

Three board tests exercise the actual PCI/BAR routing, drive selection,
reset, identification, PIO phase counts, bounded READ(12), out-of-range
reads, request sense and write protection. The offline suite produced
978 passing test executions, 0 failures and 22 ignored. Formatting,
all-target Clippy with warnings denied, and all four demos passed.
