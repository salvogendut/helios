# SS-5 banks, TLB diagnostics and the next IOMMU stop

Checked 2026-11-06. Human contract: docs/dram-tlb.md.

- The user supplied sun_ascii.txt and requested a startup logo like IRIS. CLI
  main embeds it and prints to stderr with the HELIOS expansion. Preserve the
  artwork exactly. The library stays silent and guest TTYA stdout stays clean.
  IRIS embeds its art in src/monitor.rs.
- Use Machine::ss5 / ss5_with_nvram for firmware probes; CLI boot does so.
  Machine::sun4m retains strict contiguous RAM for small synthetic tests.
  Reusing the old constructor produces the obsolete memory-probe fault.
- src/dram.rs reserves eight 32 MiB apertures. Slots accept 0/8/32 MiB;
  the total-size constructor packs 32 MiB then 8 MiB modules into low slots.
  Default 128 MiB is four 32 MiB modules and four empty slots. Totals 240/248
  MiB cannot fit these eight slots. An 8 MiB module mirrors four times.
- Primary microSPARC-II 8.2.1/8.2.2 explicitly says empty-bank accesses complete
  with unknown data and possible parity errors. Model zero reads/discarded
  writes, without parity injection. SS-5 service manual 11.3 confirms module
  sizes. Pinned QEMU sun4m.c / empty_slot.c also uses zero/discard for the tail.
- All widths and exchanges share real mirrored storage. Raw imports must fit
  real storage in one bank; no silent alias/hole writes or partial failures.
- ASI 06: 000..0fc PTE data, 100..1fc lower tag, 300..3fc upper tag. Reserved
  200..2fc and >=400 stop explicitly. Word only. Upper VA31:12, context11:4,
  V3, level2:0. Lower UE/UR/UW/SE/SR/SW9:4, S3, IO2, PTP1, M0. These are the
  actual CPU TLB entries, not a diagnostic shadow array.
- Diagnostic data retains level31:29 and implemented PTE bits; 28:27 read zero.
  ET derives from PTP. CPU PTE R reads one, PTP3:2 read zero, IOPTE7:3 read zero.
  Normal fill levels are 100/110/111. ASI 03 probes retain the existing memory
  PTE format without diagnostic level bits. Tag permissions govern hits.
- Normal fills and R/M updates are visible. Diagnostic edits detach the backing
  pte_address. Reads and tag-M=1 stores work; M=0 stores explicitly stop because
  the hardware rewalk is pending. Active injected PTPs or reserved CPU level010
  conservatively stop translation/entire probes. IOPTEs do not match CPU leaves.
  Raw patterns are allowed while translation is disabled.
- TRCR ASI04 address1000 mask01f1ffff follows figure5.15; prose incorrectly
  reserves some speed fields. QEMU MB86904 instead uses00ffffff. WP12:7,
  TC6, counter5:0, IL13, PL16:14, VP20, SBSP24:23, MEMSP22:21.
  Fills advance the counter; TC freezes it; IL clamps CPU wrap to47; PL redirects
  reserved0..2 to alternate3..5. Out-of-range counters are reduced modulo the
  eligible range. These are functional choices, not clock-accurate replacement.
  Reset WP63 and other fields0; flush clears V but retains contents/counter.
  PTP caching, DVMA replacement and speed-strap identity remain pending.
- Visually checked /tmp/helios-ms2-tlb-high.png (printed117), tlb-upper.png
  (actually LOWER tag, printed118), tlb-map.png (120), trcr.png (88).
- Final OBP: 20543176 instructions, zero traps/interrupts/resets/console bytes.
  Attempt20543177: PC70005b68, nPC70005b6c, e6a58400, ASI20 STA l3=a5a5a5a5
  through l6=10000000. IOCR.EN register-test pattern stops as unsupported DVMA
  enable; CPU error is clear. The earlier DRAM-only probe stopped at TRCR,
  PC70006178, address1000; the TLB work resolves that stop.
- Twelve new tests, 178 total; fmt, Clippy, tests and four demos pass. Records:
  local/probes/dram-tlb-ss5/. Temp source /tmp/helios-dram-probe.rs uses ss5;
  final log /tmp/helios-dram-tlb-probe.log. Original firmware is unchanged.
  No firmware prompt, cache POST completion or DMA transfer claim.
- Follow-up: [iommu-dma.md](iommu-dma.md) implements IOMMU translation and
  I/O flushes through the shared TLB, resolving the IOCR.EN stop. The findings
  above are the earlier DRAM/TLB milestone. Remaining fault-register work is
  recorded in the follow-up.
  Reuse docs/io-control.md and its documented microSPARC-specific masks.
