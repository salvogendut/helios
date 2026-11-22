# Ultra 5 unplugged keyboard probe

Original OBP 3.31.0 sends byte 1 through the keyboard UART, sees no incoming
bytes and prints `Probing keyboard Done` on serial A with diagnostics enabled.
All four combinations of diagnostic byte and reset marker stop at
PC/nPC `0xffff_ffff_f000_0780/0784`. The next word is `0xc0f00b09`, a
D-MMU Tag Access store at offset `0x30`. The following loop clears
64 DTLB and 64 ITLB slots using indexed stores.

| Retired before the stop | Quiet | Diagnostics |
| --- | ---: | ---: |
| Cold | 30,722,687 | 30,725,175 |
| Soft marker | 171,971 | 174,083 |

On the cold diagnostic run the APB clears retired at zero-based step indices
173,486, 10,357,789 and 20,458,875. The guest took its timeout exits at
10,274,551, 20,458,854 and 30,559,940. The keyboard reset store was at
10,274,568, with transmitter-empty observed by the return at 10,357,772.
The ROM regression checks guest register values and timing at these points.
Both soft-marker transcripts match the previous checkpoint byte for byte.
Four bounded traces and validation logs are in
`local/ultra5-keyboard-timeout-2026-11-22/`.

Linux names the APB register, Themis documents its four-byte access size,
the reference Ultra 5 reports its PCI clock, and the original ROM supplies
the eight-byte zero-clear and timeout loop. QEMU's pinned Simba model
delegates configuration writes to its generic PCI bridge. It does not
implement a clock for this register. [Board and UART sources](../docs/ultra5.md#cold-start-keyboard-probe) · [Pinned QEMU Simba](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/pci-bridge/simba.c)
