# SMP slice 2B: standalone IOMMU ownership

Implemented from merged checkpoint `d799ac2`; independently accepted after
the corrections recorded in [smp-slice2b-review.md](smp-slice2b-review.md).
Scope is limited to I/O translation ownership. No scheduler, CPU startup,
arbitration, shared device-clock change, CPU-count setting or UI work.

## Ownership design

- SS-5 keeps the existing integrated `Srmmu` CPU/I/O TLB, replacement,
  diagnostics and destructive flush consumption. Its DMA path still uses CPU
  0's MMU and CPU-derived initiator attributes.
- SS-20/Axil now have a `Machine`-owned `StandaloneIommu`. It contains a
  16-entry LRU cache of DVMA pages and cached IOPTEs, with no CPU context,
  fast cache, diagnostic arrays, replacement pointer, MXCC state or
  referenced/modified writeback.
- Both SCSI and Ethernet DMA use `Machine::dma_address`; on MBus that selects
  the standalone owner instead of CPU 0. The owner emits explicit device
  memory requests with CPU=0, boot mode=0, cacheable=0 and supervisor=0.
- `IoControl` remains the configuration/table producer. On SS-20/Axil,
  `Bus::take_io_tlb_flush` intentionally returns nothing and
  `Bus::take_standalone_io_tlb_flush` is the only consumer; CPU MMU accesses
  therefore cannot steal queued I/O invalidations. SS-5 keeps the old routing.
- Page and global I/O flushes affect only the standalone cache. CPU-local
  flushes do not affect it, and I/O flushes do not touch CPU-0 or CPU-2 MMUs.

## Format and references

The standalone format follows Sun-4M System Architecture revision 50,
sections 5.8 and 7.2: one-level 4 KiB IOPTEs, PPN bits 31:8 (PA 35:12), C bit
7, W bit 2, V bit 1, WAZ bit 0, IBAR bits 31:10 (PA 35:14), 16-entry LRU TLB
and page/all flushes. Page flush tags use FA bits 30:12; reserved bit 31 and
low address bits are ignored on the standalone profile. The SS-5 IBAR mask
and page-flush interpretation remain unchanged. QEMU
`hw/sparc/sun4m_iommu.c` at revision `f71afb409a87c4ecd0f9d63a5963eb645dfab780`
was used as the pinned comparison for address decode and flush state. The
primary manual and pinned QEMU references are already listed in
`docs/references.md`; the SS-5 microSPARC-II PPN/31-bit behavior remains
distinct.

The current model preserves the existing bounded restrictions: page tables
outside 256 MiB and non-DRAM DMA targets are explicit host stops. Sun-4M
defines disabled ME as physical pass-through; the current disabled non-bypass
host stop documents that as an unimplemented specified mode. I/O PTE C is
retained for future coherence work but has no cache effect here. The LRU is a
functional policy with no timing claim.

## Regressions

- `tests/standalone_iommu.rs`: DMA with no CPU step and unchanged CPU-0/CPU-2
  MMU state; CPU-2 programming of the controller; profile-specific IBAR masks
  and high-address aliasing rejection; stale/fresh page and global flush
  behavior including reserved FA bit 31; 16-entry LRU capacity; CPU-local
  flush isolation; Axil parity; invalid/write-protected/misaligned entries,
  bypass, disabled state, reset and PPN/target limits; DMA attributes.
- `src/iommu.rs` unit tests: page-table bus error is a device `DmaError::Bus`,
  bypass does not populate the cache.
- Existing SS-5 DMA, SCSI and Ethernet tests retain shared-TLB coverage.

## Evidence

- Core: 561 successful test lines normal / 563 with `perf-counters` (one raw
  line is the isolated MXCC cycle child; 560/562 unique).
- fmt and Clippy `-D warnings` pass in both configurations; all four demos.
- Heliacal: 29 tests and release build pass.
- Fresh disposable boots: SS-5, SS-20 and Axil reach root, print
  `HELIOS-BOOT-ACCEPTED`, and cleanly halt. Axil still reports
  `3034423527 17 /helios-disk-test`.
- SS-20 disposable write/read/reboot: first boot writes
  `/helios-slice2b-test`; after clean halt and reboot `cksum` reports
  `1354101890 16 /helios-slice2b-test`.
- Evidence: `local/smp-slice2b-2026-11-14/`, including `review-fixtures.log`
  (8/8 independent fixtures) and `boot-review/` for the final boots and
  persistence reread.
- Fresh independent re-review checks are under
  `local/review-smp-slice2b-2026-11-14/accepted/`: 8/8 fixtures, 560/562 core
  tests, formatting, both Clippy configurations, demos and Heliacal 29 tests
  plus release build. The supplied final boot/persistence logs were inspected.

## Limits

No CPU scheduler, startup/arbitration, two-CPU firmware boot or IOMMU DMA
diagnostic access. Standalone cache replacement is functional LRU over 16
entries. Non-DRAM targets and the specified but unimplemented disabled
non-bypass pass-through mode remain explicit host stops. Bursts, atomics, concurrent
owners and coherence/C-bit behavior remain unsupported.
