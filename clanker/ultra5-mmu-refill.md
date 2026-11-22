# Ultra 5 OpenBoot fast MMU miss handling

Original 3.31.0 reaches `fprs!` with `%g4=4`. Its first missing mapping
occurs with TL 0, TBA `0xf0000000`, PSTATE `0x14` and primary context 0.
All four observed traps are fast data misses (TT `0x68`). The guest handler
begins at VA `0xf0000d00` with the MMU globals selected.

| Fault PC | D-MMU Tag Access | Operation |
| --- | --- | --- |
| `0xf0009018` | `0xffffffffff000000` | LDX, retried through D-TLB slot 5 at PA `0x3ff6cf00` |
| `0xf0009238` | `0xfffffffc003fe000` | STX |
| `0xf0009238` | `0xfffffffc003c0000` | STX |
| `0xf0009238` | `0x00000000fffc6000` | STX targeting the PCI host bridge |

OBP installs the mapping through D-MMU Data In at `0xf0000f70`, then executes
`membar #Sync` and `retry`. The first retried load returns zero from RAM.
The code also uses explicit I- and D-side page demaps. The core models
Uclear at replacement boundaries and commits demaps synchronously.
Protection faults, RED-state entry, window traps and interrupts remain open.

| NVRAM seed | Retired at first PCI control store | Traps |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,466,988 | 4 |
| Cold, diagnostics off | 53,456,505 | 4 |
| Soft, diagnostics on | 22,915,896 | 4 |
| Soft, diagnostics off | 22,905,789 | 4 |

The serial transcripts, first-miss trace and refill trace are in
`local/ultra5-fprs-2026-11-22/`. The next store targets PA `0x01fe00002000`
with `0x000000140020010f`, an eight-byte access to PCI Control/Status.
[Reproduction](../docs/ultra5.md#openboot-handles-its-first-mmu-misses)

## PCI register initialization

The next 5,766 retired instructions include the PCI Control/Status store
and 74 zero stores across the clear and partial mapping register banks.
The local-ROM regression checks each physical store and its value, starting
with one populated PCI and onboard interrupt state and one enabled mapping
in each bank. It also checks that the fifth guest trap is a fast D-TLB miss.

| NVRAM seed | Retired before PCI target address programming | Traps |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,472,754 | 5 |
| Cold, diagnostics off | 53,462,271 | 5 |
| Soft, diagnostics on | 22,921,662 | 5 |
| Soft, diagnostics off | 22,911,555 | 5 |

All four stop at the store of `0x40` to PA `0x01fe00002028`. The raw serial
transcripts and stop records are in `local/ultra5-pci-init-2026-11-22/`.
[PCI interrupt initialization](../docs/ultra5.md#pci-interrupt-initialization)

## First access to the Simba command register

After target selection the guest retires the diagnostic, host PCI command
and latency writes. Counting from immediately before the target store adds
69,533 retired instructions on each seed. The local-ROM test checks the
little-endian command readback (`0x0146`), latency (`0x28`) and the
operands of the next `stha %l1,[%l0] %asi`.

| Seed | Instructions retired | Trap entries |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,542,287 | 5 |
| Cold, diagnostics off | 53,531,804 | 5 |
| Soft, diagnostics on | 22,991,195 | 5 |
| Soft, diagnostics off | 22,981,088 | 5 |

The instruction at VA `0xf00266a0` attempts a zero write to PA
`0x01fe01000804` through ASI `0x15`. This is Simba function 0's primary
command, separate from the CPU's PCI host command at `0x01fe01000004`.
Diagnostic runs end in `ttya initialized\r\n` and quiet runs emit no bytes.
Serial transcripts and stop records are under `local/ultra5-pci-target-2026-11-22/`.
[Reproduction](../docs/ultra5.md#pci-host-bridge-configuration)

## Downstream bridge setup and first IOMMU read

All four seeds retire another 6,059 instructions. The ROM fixture checks
both enable/disable sequences, bus numbers, address maps, control and timer
readbacks. The next access is an LDX from VA `0xfffc4200` at PC
`0xf0009018`, nPC `0xf000901c`, targeting PA `0x01fe00000200`. Start
the IOMMU work with IIi §19.3.2.1 and this mapped read.

| Seed | Retired before IOMMU control read | Trap entries |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,548,346 | 5 |
| Cold, diagnostics off | 53,537,863 | 5 |
| Soft, diagnostics on | 22,997,254 | 5 |
| Soft, diagnostics off | 22,987,147 | 5 |

The per-seed serial transcripts match the preceding checkpoint byte for
byte. Stop records and validation logs are in
`local/ultra5-simba-2026-11-22/`.
[Reproduction](../docs/ultra5.md#simba-bridge-configuration)

## IOMMU setup and CPU TICK boundary

Each seed retires another 134,007 instructions and takes one additional
fast data miss. The guest reaches its Forth `tick!` primitive with
instruction `0x89912000` (`wrpr %g4, 0, %tick`), `%g4=0`, PC/nPC
`0xf00235f0/0xf00235f4`. IIi §14.1.8 defines TICK.NPT and the
63-bit cycle counter. The existing APB tick is a separate PCI clock.

| Seed | Retired before TICK write | Trap entries |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,682,353 | 6 |
| Cold, diagnostics off | 53,671,870 | 6 |
| Soft, diagnostics on | 23,131,261 | 6 |
| Soft, diagnostics off | 23,121,154 | 6 |

The local-ROM regression checks every IOMMU store and readback, starting
with 16 valid cacheable entries. The unmodified-ROM serial transcripts are
byte-identical to the Simba checkpoint. Per-seed results and validation
logs are in `local/ultra5-iommu-2026-11-22/`.
[Reproduction](../docs/ultra5.md#iommu-initialization)
