# SMP slice 2B: standalone IOMMU ownership

Assignment for the implementing AI after the incremental merge of slices 1
and 2A, validated through core revision `a94f0aa`. Continue on
`feat/sun4m-smp` from the merged main revision. This is the next bounded part of
[issue #2](https://github.com/salvogendut/helios/issues/2) and the
[SMP plan](smp-plan.md). Return for review before committing this slice or
starting scheduling, startup/arbitration or a two-CPU firmware boot.

## Problem to solve

`Machine::dma_address` in `src/dma.rs` always translates through
`self.cpu.mmu`. `Mmu::sync_io_flushes` destructively drains the bus's I/O flush
queue. This models SS-5's integrated shared CPU/I/O translation state, but
leaves the SS-20/Axil standalone IOMMU tied to CPU 0. Adding another CPU must
not make DMA mappings or flush results depend on which CPU happens to execute.

Give the MBus profiles an explicit standalone I/O translation owner. Keep
SS-5's shared CPU/I/O TLB, replacement and diagnostic behavior intact. This
slice must remain testable without scheduling a second CPU.

## Implementation boundaries

1. Read the existing IOMMU/DMA and MMU documentation, then confirm the
   standalone controller's PTE format, physical-address width, cache/flush
   semantics, register masks and reset behavior against the primary hardware
   sources and pinned QEMU/MAME revisions in `docs/references.md`. Document
   evidence and remaining functional choices. Do not copy microSPARC-II
   replacement or diagnostic rules into the standalone controller by default.
2. Choose one authoritative owner for the SS-20/Axil I/O translation cache
   and invalidations. A dedicated board-owned type is appropriate; a spare
   cloned CPU/MMU is not. Reuse format/validation helpers where justified,
   without coupling I/O entries to a CPU's context, fast cache, diagnostic
   arrays or replacement pointer.
3. Route both SCSI and Ethernet DMA through that owner on SS-20/Axil. Keep
   SS-5 on its integrated path. Make reset, disabled/bypass behavior and
   supported target restrictions explicit. Preserve byte/halfword/word
   transfers and the word-transparent data path.
4. Give each queued I/O invalidation exactly one intended consumer. On the
   MBus profiles, CPU instruction/data accesses must not steal standalone
   IOMMU invalidations. CPU-local TLB flushes must not invalidate another
   CPU or the standalone IOMMU. Ordinary PTE writes must not hide missing
   guest flushes by globally invalidating cached translations.
5. Audit the incidental CPU-0 reads in `dma_address`, including PSR-derived
   privilege and MMU boot mode. Define DMA initiator attributes explicitly
   for the standalone path, without attributing a device fault to whichever
   CPU last touched the bus. Preserve the integrated SS-5 behavior unless
   independent evidence requires a separately documented correction.

Do not change instruction scheduling, the shared device clock, module startup,
CPU-count configuration or the UI. Do not combine this with the previously
failed ESP service guard, SS-5 WP=0 replacement changes, broad performance
refactoring or an expansion of currently unsupported DMA targets.

## Required regressions

- On both MBus profiles, populate two independently constructed module MMUs
  and distinct CPU translations. Perform DMA without stepping CPU 0. Show
  correct data transfer and unchanged CPU contexts, fault registers, TLB
  diagnostics and translations. Exercise a controller write through the
  second module's access path without adding a scheduler.
- Populate an I/O translation, change its backing PTE, and demonstrate the
  documented stale/fresh behavior before and after page and global I/O
  flushes. Include two cached pages to establish page-flush scope.
- Queue an I/O flush, then execute CPU-0 and CPU-2 MMU accesses before the
  next DMA operation. The correct owner must still consume the invalidation;
  neither CPU may silently discard it.
- Exercise CPU-local flushes separately and prove they do not flush the
  standalone I/O cache or the other module's CPU translation state.
- Cover invalid PTE, write protection, alignment, bypass/disabled state,
  table/target bus errors and reset. Preserve failure atomicity and distinguish
  retryable host stops from guest/device faults. I/O PTEs must not acquire CPU
  referenced/modified bits.
- Retain meaningful SS-5 tests for shared I/O entries, diagnostic visibility,
  flush behavior and replacement semantics. Existing disk/Ethernet transfer
  tests must continue to pass.

Use independently derived addresses and expected values. If the sources do
not establish a cache detail, document a bounded functional policy instead
of claiming hardware confirmation. Any change in performance-counter units
or ownership must be documented and covered without adding normal-build
counter overhead.

## Return evidence

Report the final ownership and invalidation design, primary references,
tracked regressions and unresolved limits. Run normal and `perf-counters`
tests, formatting, Clippy with `-D warnings` in both configurations and all
four demos. Test and rebuild Heliacal against the resulting core.

Boot disposable SS-5, SS-20 and Axil disk clones to a root shell, record
`uname -a` and `HELIOS-BOOT-ACCEPTED`, then sync and halt cleanly. Verify the
existing Axil persistence checksum. Include a bounded SCSI write/read/checksum
and clean reboot on a disposable MBus disk because this slice changes its
translation owner. Preserve source media and retain the logs/harness, removing
only the scratch clones created for the completed run.

Stop at the standalone-IOMMU review gate. Arbitration/startup and deterministic
execution are later assignments; successful single-CPU DMA tests are not
evidence of a working two-CPU Solaris boot.
