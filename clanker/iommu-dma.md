# IOMMU translation and first OBP serial diagnostic

Historical checkpoint: [fault-capture.md](fault-capture.md) records the following
milestone, which resolves the timeout/capture gaps and advances OBP to LDDF.

- Primary manual microSPARC-II rev1.1 sections 5.2.1–5.2.5, 5.6.6,
  5.7.1–5.7.4, 5.7.7/10, 5.8, 5.10, 9.1/4 and chapter11 table50.
  QEMU f71afb409a87c4ecd0f9d63a5963eb645dfab780 sun4m_iommu.c is a comparison,
  not the SS5 hardware contract. MAME sun4m remains unfinished.
- IOCR now accepts EN. All eight ranges validate high VA bits; table indexing
  concatenates high IBAR and VA page index, ignoring base bits within table
  size. IOPTE PPN26:8, W2, V1; other bits ignored, no R/M updates.
- Machine::dma_read/write accept master0..5 and Width byte/half/word. One aligned
  transfer, RAM targets/page tables only, no CPU execution or time advancement.
  SBAE gates each master. NotGranted is no transaction, not a device error.
  Masters4/5 cannot bypass; slots0..3 bypass only for VA31:30=00, SA30 supplies
  PA30, and section5.8 requires main-memory aperture. EN0 non-bypass is an
  explicit host stop pending a firm hardware contract, never identity mapping.
- Srmmu owns shared 64 entries. IOPTE tag upper=(VA&!fff)|8, lower=4, data
  level111; CPU ignores IO leaves, DMA ignores CPU context. W comes from PTE.
  IL puts I/O fills in48..63 using low4 counter bits. Counter advances on fills
  as before, TC freezes; no separate IOTLB. CPU conservative flush still clears
  all entries, including I/O; I/O flushes only invalidate IO tags.
- SystemBus/IoControl queues All/Page commands; Bus::take_io_tlb_flush defaults
  to None on plain buses. MMU drains before CPU/DMA/diagnostic accesses. Direct
  register writes, guest ASI20 writes and diagnostic reads therefore agree.
  Firmware control/base writes do not implicitly flush. Reset clears queues/TLB.
- Translation faults return an SBus error acknowledge to the master, NOT CPU
  SFSR/SFAR/AFSR or level15. Table50 explicitly distinguishes invalid/protected
  I/O mappings from parity and CPU SBus writes. Do not copy QEMU iommu_bad_addr's
  unconditional LE/AFSR/IRQ behavior. Device engines must handle their own errors.
- 12 DMA tests (190 total) cover ranges/alignment, endian lanes, stale entries,
  diagnostic injection, partitioned fills, flush isolation, permissions,
  arbitration, bypass, reset and error separation. fmt/clippy/all4demos pass.
- Primary original firmware SHA256 unchanged:
  e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  Machine::ss5,128MiB four32MiB modules, defaultNVRAM, no input,100ns/step,
  budget30M. /tmp/helios-iommu-probe.rs retains first500, per-million samples,
  first-fault context and last128. local/probes/iommu-ss5/ stores final evidence.
- First fault attempt20588909: PC700063d0,nPC700063d4,wordc0850400 LDA ASI20
  from PA7cc00000, unmapped SBus read -> trap29. Console125bytes says
  Address=00000300 exp=00000816 obs=00000016 xor=00000800, U0103,
  WARNING: Suspect Swift Module. Figure5.13 identifies missing bit11 as SFSR.TO.
  Current finish/record collapses bus errors without timeout detail. Fix this
  from bus source classification, not by hardcoding the firmware address.
- Software reset attempt20617884 atPC0000beb0. Final stop attempt21020692:
  PCffd27690,nPCffd27694,wordc8840400 LDA ASI20 from10001050(MFSR).
  Completed21020690 instructions,1trap,0IRQs,1reset. CPU error clear,CLIexit1.
  Cache-tag store counts0c/0e are1024 each due to restart. No OBP prompt and no
  firmware-driven DMA transfer claim. CLI/library serial output match.
- Next: accurate SBus read-timeout classification and SFSR flags, then MFSR/MFAR
  capture, AFSR/AFAR and associated interrupt routing. Section5.7 prose is
  inconsistent about DMA parity capture; use detailed register descriptions and
  table50. Keep unsupported host behavior distinct from guest hardware errors.
