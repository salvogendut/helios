# microSPARC-II IOMMU and SBus DMA

Helios implements IOMMU translation to RAM, shared CPU/I/O TLB entries, I/O
flushes, SBus arbitration and slot bypass. `Machine::dma_read` and `dma_write`
accept a master number (0–5), a 32-bit DVMA address and byte/halfword/word width.
Transfers must be naturally aligned. Values remain guest bit patterns; narrow
accesses use big-endian lanes. These calls do not execute the CPU or advance time.
SCSI/Ethernet device engines and burst transfers remain pending.

The primary source is the microSPARC-II User's Manual revision 1.1, sections
5.2.1–5.2.5, 5.6.6, 5.7.1–5.7.4, 5.7.7, 5.7.10, 5.8, 5.10, 9.1/9.4 and
chapter 11 table 50. [References](references.md) link the manual and pinned
QEMU comparison. Hardware-specific behavior takes precedence over QEMU's
generic sun4m model.

## Registers

| Physical address | Register | Implemented behavior |
| --- | --- | --- |
| `0x10000000` | I/O MMU control | IMPL=0, VER=4; RANGE bits 4:2 and EN bit 0 writable |
| `0x10000004` | I/O page-table base | Bits 26:10 writable, representing physical base bits 30:14 |
| `0x10000014` | I/O TLB flush all | Any word write invalidates all I/O entries |
| `0x10000018` | I/O TLB page flush | Word write invalidates I/O entries matching value[31:12] |
| `0x10001010`–`0x1000101c` | Slot configuration 0–3 | SA30 bit 16, BA8 bit 1, bypass bit 0 writable |
| `0x10001020` | Shared slot configuration 4 | BA8 bit 1 writable; bypass and SA30 zero |
| `0x10002000` | MID / SBus arbitration | MID reads 8; six enable bits at 21:16 writable |

Reserved/identity bits ignore writes. Flush ports have no stored readback value;
reads stop explicitly. All registers require plain aligned word accesses.
Bytes, halfwords, doublewords, masked stores, atomics and raw-image writes stop
before device effects. The physical bus reserves `0x10000000`–`0x10007fff`;
RAM/ROM cannot overlap it. Wider control-space mirrors remain pending.

## Translation and ownership

RANGE selects a window of `16 MiB << RANGE` ending at `0xffffffff`. All address
bits above that window must be one, even on a TLB hit. An IOPTE is a big-endian
word in a single-level table, with PPN at bits 26:8, writable at bit 2 and valid
at bit 1. Other bits are ignored. Pages are 4 KiB; physical addresses are 31 bits.
There are no CPU referenced/modified updates to IOPTEs.

The table has `window_bytes / 1024` bytes. Its address concatenates the high
IBAR physical-base bits and the DVMA page index times four: base bits within
the table size are ignored. This matters for larger windows with misaligned
IBAR values; adding the index to an unmasked base is incorrect.

The CPU's `Srmmu` owns the one 64-entry TLB. I/O entries match virtual page and
I/O tag, ignoring CPU context and privilege. ASI `06` reads and edits these same
entries. Normal I/O fills use diagnostic level `111`, upper tag level `000`,
Valid=1, IO=1 and S=0. Writes require IOPTE.W, including on diagnostic entries.
With TRCR.IL set, I/O fills use slots 48–63, indexed by the low four counter bits;
CPU fills use 0–47. With IL clear they share the replacement range. TC freezes
the counter. Counter advancement on fills is a functional convention, not CPU
clock timing; [dram-tlb.md](dram-tlb.md) describes WP/PL and reset choices.

Page-table edits, IBAR/IOCR writes and CPU context changes do not automatically
invalidate I/O entries. Software must flush stale mappings. I/O flushes clear
Valid without touching CPU entries, retained tag/PTE contents or the counter.
The existing conservative CPU flush implementation clears the whole shared TLB.

`IoControl` belongs to `SystemBus`; it queues flush commands. The bus exposes
them through `Bus::take_io_tlb_flush`, and `Srmmu` consumes them before its next
CPU, DMA or diagnostic access. Commands can be coalesced while no TLB access
intervenes. There is no separate IOTLB or global lock. Machine reset clears
configuration, pending commands and the shared TLB while retaining RAM.

## Arbitration, bypass and failures

Each master needs its SBAE bit before a transfer. A disabled master gets
`DmaError::NotGranted`, meaning no transaction was issued. Masters 4 and 5 share
slot register 4 and cannot bypass; master 5 serves the on-board SCSI/Ethernet
DMA block. Slots 0–3 may bypass only for DVMA addresses with bits 31:30 zero.
In that case PA[29:0] comes from DVMA and PA[30] from SA30. Section 5.8 restricts
bypass to the main-memory aperture; addresses outside it return an error.
High addresses still use normal IOMMU translation despite slot bypass being set.

Out-of-range addresses, invalid IOPTEs and write-protected stores return
`DmaError::Fault` to the initiating device as an SBus error acknowledge. They
leave target memory unchanged, do not populate CPU SFSR/SFAR and do not raise
an interrupt themselves. CPU PCR.NF has no effect on DMA errors. A future
master must report its completion/error status and generate its own interrupt.
Physical table/target failures retain `BusError` and its physical address;
`Unsupported` and `WouldBlock` remain emulator stops/backpressure.

Unlike QEMU's `iommu_bad_addr`, invalid/protected mappings do not synthesize an
AFSR late error or a CPU level-15 interrupt. The manual's error table distinguishes
these DMA errors from parity errors and asynchronous CPU SBus write failures.
CPU SBus store errors now have separate [capture and interrupt wiring](fault-capture.md);
real memory-parity error generation remains pending.

## Limits and validation

This API currently supports RAM targets and page tables in the DRAM aperture.
Non-DRAM targets/tables, DMA with EN=0 and no qualifying slot bypass, and
unsupported active diagnostic I/O tag formats stop explicitly. The disabled
non-bypass case deliberately has no invented identity mapping. There are no
bursts, DMA atomics, concurrent owners, device engines, cache snooping, bus-cycle
timing or parity injection. Single-transfer failures do not partially write RAM;
future multi-transfer engines must track partial completion themselves.

[Fault registers](fault-capture.md) at `+0x1000/+0x1004` and `+0x1050/+0x1054`
now capture asynchronous CPU store errors and live memory-request metadata.
Performance/breakpoint registers (`+0x3000`–`+0x301c`) and local-graphics queue
controls (`+0x4000`, `+0x6000`, `+0x7000`) still stop explicitly. Reserved holes
are unmapped. No generic register array substitutes for these facilities.

Other documented differences from QEMU remain: IOCR version 4 rather than its
SS-5 profile's 5; six SBAE bits rather than five; five slot registers with shared
slot-4 restrictions; no MBus arbiter register at `+0x1008`. Figure 5.23 and the
six-master arbiter description resolve the prose's SBAE reserved-bit typo.

Tests cover all eight ranges, first/last pages, table-base alignment, lane order,
permissions, arbitration, bypass, reserved bits, stale mappings, CPU/I/O
partitioning, diagnostic injection, selective flushes, reset and error isolation.
The original ROM now passes IOCR enable writes and both bus-error diagnostics
without the previous warning/reset. It then passes floating-point transfers,
comparisons and arithmetic, then reports a missing processor-timer interrupt.
See [firmware.md](firmware.md).
This does not establish working SCSI/Ethernet DMA or a firmware prompt.
