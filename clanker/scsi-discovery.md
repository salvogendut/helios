# SCSI discovery milestone

User scope: implement selection and disk/CD-ROM images; stop when
`probe-scsi-all` lists available targets. Reached on 2026-11-07; do not continue
into OS boot without a new instruction.

## Result and reproduction

- Original `local/firmware/sun-ss5.bin` OBP 2.15 plus `turbogx.bin`, 128 MiB,
  Type 5 keyboard, unchanged Sun identity 08:00:20:c0:ff:ee / 80c0ffee.
- NVRAM `local/probes/graphics-ss5/128m-configured-nvram.bin` was produced by
  firmware `setenv`, with auto-boot and diagnostics disabled. No PROM patches.
- Read-only `local/probes/scsi-ss5/disk.raw`: 4194304 zero bytes, SHA-256
  `bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8`.
- Read-only backend for `local/probes/scsi-ss5/cdrom.iso`: 262144 zero bytes,
  SHA-256 `8a39d2abd3999ab73c34db2476849cddf303ce389b35826850f9a700589b4a90`.
  The filename does not imply an ISO9660 filesystem; this is a blank fixture.
- Headless graphical probe source `local/probes/scsi-ss5/probe.rs` queues actual
  Sun key events at step 100000000. Final capture at 182392204 shows Target 3,
  Unit 0 Disk, HELIOS SCSI DISK 0001; Target 6, Unit 0 Removable Read Only device,
  HELIOS SCSI CD-ROM 0001; followed by `ok`. See `final-run.log` and
  `probe-scsi-all.png`. Both image contents were unchanged after the run.
- An initial capture at 132392204 was too early; the scan was still underway.
  Allow missing-target timeouts to complete rather than treating the lack of
  immediate output as a stall.

## Implementation boundaries

`ScsiBus` owns read-only files separately from the clonable ESP state. Target
replies are bounded by READ(10)'s maximum transfer; no disk-sized preload.
`Esp::poll` services local phases and yields a byte request. The machine uses
the existing master-5 DVMA path, commits counters only after successful memory
access, retains work on arbiter denial, and latches DMA2 error on memory faults.
Byte writes drain immediately; no physical FIFO/burst timing claim.

PIO and DMA select 41/42, IDENTIFY, CDBs, data input, ICCS and message accepted
work. A FIFO-preloaded IDENTIFY precedes DMA CDB bytes. No selection-stop,
sync negotiation, target mode, disconnect/reselection or full command queue.
No write overlays yet: both media are write-protected. CD logical blocks remain
2048 bytes; MODE SELECT/512-byte CD blocks and TOC remain future boot work.

Sources: NCR integrated-core and DMA2 manuals linked in docs/references.md;
QEMU `f71afb409a87c4ecd0f9d63a5963eb645dfab780` esp.c; MAME
`ed59c15fe78596ec6ff8fe04482ede78e6122e34` ncr53c90.cpp; SCSI-2 clauses 8/9/14.
Do not use the archived select-with-ATN table's misaligned bit columns over its
prose or the independent controllers' FC|BS result. D_WRITE is bit 8 despite
the omission from the archived DMA2 text. TC W1C applies only with NEXT enabled.

## Validation

335 core tests, four frontend tests, both fmt/Clippy checks and release builds,
and all four demos passed with the local nightly toolchain. Ten new SCSI tests
cover actual target responses, raw disk/CD reads, image immutability, timeout,
PIO/DMA residuals, zero ESP count, reset attention, preloaded IDENTIFY, chaining
and IRQ, unsupported LUNs, arbiter denial and IOMMU fault recovery. The chaining
IRQ test must unmask both the source and the controller's global mask-all bit.
The public run instructions are in docs/scsi.md; no firmware/images are tracked.
