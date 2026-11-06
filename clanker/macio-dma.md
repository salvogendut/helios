# MACIO DMA2 SCSI registers

- AFX checkpoint 2a77666 pushed; CI 42296880413 passed.
- Primary references: /tmp/helios-classic-io.pdf, /tmp/helios-classic-io.txt
  (Sun part three, not /tmp/helios-sparcclassic.pdf which is part one).
  Printed 3-26 (PDF page 117) checked visually: D_WRITE bit 8 missing from
  /tmp/helios-dma2.txt. Primary register semantics override QEMU's generic array.
- SCSI DMA register subset: CSR masks/reset held/W1C, address/count including
  initial NEXT copying, empty FIFO invalidation, IRQ gate. No DMA requests or
  terminal count generation yet. NCR source 18 routes to level 4, not level 9.
- Original ROM next failure: attempt 93375980, PC ffd1689c, IRQ15, AFSR
  a3820000 / AFAR 7880000c. Previous instruction writes ESP command (byte).
  No data-access synchronous trap here: CPU SBus stores capture async error.
  Trace must stop on InterruptTaken level15 as well as TrapTaken.
  No firmware patch or forced probe success. Timer warning remains separate.
- Local evidence local/probes/macio-dma-ss5. Next implement NCR53C9X controller
  reset/configuration and connect its reset/IRQ to DMA2. Primary text fetched
  /tmp/helios-ncr53c9x.txt; QEMU pinned esp.c comparison /tmp/helios-qemu-esp.c.
- 286 tests, fmt/clippy and four demos pass.
