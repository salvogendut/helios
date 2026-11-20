# Ultra 5/10 first fetch on the V9 base

Ready for review on `feat/ultra5-first-fetch`, based on the accepted V9 fixture at `cb3468b`. The original dirty worktree was reapplied to that commit. The checkpoint ends after a single flash read.

## Corrected observation

```text
PC=0xfffffffff0000020  nPC=0xfffffffff0000024
PA=0x01fff0000020     flash offset=0x20     word=0xc4d808a0
```

The core retains the 64-bit virtual PC/nPC. It masks the PC to PA[40:0] at the flash read. The synthetic test places an arbitrary word at offset 0x20, so substituting the known firmware instruction for a real flash read fails the test.

The extractor checks the SHA-256 of Sun’s updater, follows the two board descriptors in its ELF data segment, checks both OBMD headers per board, and compares the complete images. For both boards the OBP half followed by the POST half yields SHA-256 `fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`. Board offsets, expected register values, and reproduction commands are in the [fixture documentation](../docs/ultra5.md).

## Validation

- The default offline suite reported 833 passing executions and one ignored local-ROM test. This includes all 15 V9 tests. One existing MXCC fixture runs again in a child process. The additional ordinary test introduced in this correction checks the OBP revision word.
- The explicit local-ROM test passed separately. It checked the full image digest, both halves and the literal fetch tuple. The original three independent review fixtures also passed against the working tree.
- The independent ELF-segment verifier reproduced the digest. Extraction checks covered both descriptors, four individually damaged OBMD headers, an invalid source digest and conflicting output paths.
- Paused QEMU 10.2.2 read `0xc4d808a0` from physical `0x01ff_f000_0020` and disassembled it as `ldxa [%g0] #ASI_LSU_CONTROL_REG, %g2`. Its register report showed PC/nPC in the low physical range. Sun’s reset table supplies the architectural expectations. The QMP transcript includes the prelaunch status.
- Formatting, offline all-target Clippy with warnings denied, and the four built-in demos passed.

Fresh logs and the extraction manifest are in `local/ultra5-first-fetch-2026-11-20/`. Its `superseded/` directory preserves the earlier generated slice and logs for comparison. The source zip and updater retain their original hashes. The [initial review](ultra5-first-fetch-review.md) records the two original failure modes and independent reproduction.
