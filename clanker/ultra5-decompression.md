# Ultra 5 original firmware decompression

The entire 3.31.0 decompressor and its callers execute in the V9 interpreter.
The stream header is `1f 9e 0e` at flash offset `0x2a80`. The independent
Python decoder uses whole byte strings for LZW dictionary entries, consumes
9- through 14-bit codes and stops on end code 257. It decodes only the local
ROM during validation. The emulator uses guest instructions to produce the
image. SHA-256 of the 453,536 output bytes is
`4084f24a619f2c14cd0a87c6a5f82f29f7513627c74a85d759c6271de184b366`.

The original-ROM regression checks all 56,692 xwords at PA `0x3ff80000`,
the end pointer at PA `0x3ff41f50`, both adjacent guard words and the first
instruction fetched at PA `0x3ffd1f8c`. The guest's size print is hexadecimal.
Both diagnostic paths end at the same PC as their quiet controls.

| Reset seed | Diagnostics | Instructions retired before VA `0xf0051fb8` |
| --- | --- | ---: |
| Cold | On | 51,181,204 |
| Cold | Off | 51,173,853 |
| Soft | On | 20,630,112 |
| Soft | Off | 20,623,137 |

Transcripts and validation logs are under `local/ultra5-decompression-2026-11-22/`.
The next instruction is `0xa580203d`, a privileged write to dispatch control.
The IIi manual's Appendix I.1.2 specifies the firmware's exact value as the
recommended initialization. [Reproduction and sources](../docs/ultra5.md#executing-the-expanded-firmware)
