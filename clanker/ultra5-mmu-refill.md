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

OBP loads D-MMU Data In at `0xf0000f70` with a store, then executes
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
