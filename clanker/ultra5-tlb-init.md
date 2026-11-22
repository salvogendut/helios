# Ultra 5 TLB clear and memory probe entry

Original OBP clears the separate D/I tag staging registers and writes the
64 indexed slots in each bank in descending order. The local-ROM regression
prepopulates every entry with a nonzero valid mapping on both sides before
starting the guest. Immediately after the loop it checks both staging
registers and all 128 stored tag/data pairs. The directed-access fixtures
check context and VA capture, sign extension, side-specific attributes,
redundant page-size encodings and fault atomicity. QEMU also selects its
separate indexed arrays using address bits 8:3. Field expectations for the
IIi are from the Sun manual. [Hardware and firmware behavior](../docs/ultra5.md#instruction-and-data-tlb-initialization) · [Pinned QEMU SPARC ASI handlers](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/ldst_helper.c)

| Reset | Diagnostics | Quiet |
| --- | ---: | ---: |
| Cold | 30,726,251 | 30,723,427 |
| Soft marker | 175,159 | 172,711 |

All four runs stop before word `0xe2de0280` at flash offset `0xfec`,
`ldxa [%i0] 0x14, %l1`, with `%i0 = 0`. OBP has prepared
`0xaaaaaaaa55555555` and its XOR complement. The next two reads use
`%o4 = 0x800000` and `%o5 = 0x1000000`; the routine then writes
patterns, tests for aliasing and restores all three saved words.
Use these operations to verify populated/empty bank behavior. The
register-write checkpoint alone establishes no installed RAM capacity.
Transcripts and validation logs are in `local/ultra5-tlb-init-2026-11-22/`.
The full suite passed 898 standard test executions and 17 explicit
original-ROM checks. Formatting, offline all-target Clippy with warnings
denied and the four built-in demos passed. The reconstructed ROM SHA-256
remained `fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`.
