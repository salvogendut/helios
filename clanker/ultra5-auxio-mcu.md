# Ultra 5 DRAM controller setup

The bounded headless fixture on top of `395a006` advances original OBP
3.31.0 from the front-panel LED store through memory-controller setup to the
first E-cache sizing instruction. [Register map and reproduction](../docs/ultra5.md#front-panel-led-and-dram-controller-setup)

| NVRAM reset marker | Byte 1 | Retired at PC `0xffff_ffff_f000_0714` | Serial A |
| --- | ---: | ---: | --- |
| Zero | 0 | 6,865 | Empty |
| Zero | 1 | 7,169 | `Hardware Power ON\r\n` |
| `soft-rst` | 0 | 6,834 | Empty |
| `soft-rst` | 1 | 7,138 | `Software Power ON\r\n` |

All four cases run from reset in Helios. The regression checks both MCU
store operands and readback, exactly 1,000 LED reads in each delay, and the
return delay slot in window 0. The next word, `0xc0f2_4ec0`, tries to store
zero through ASI `0x76` at diagnostic address `0x80_0000_0000`.
Local evidence is in `local/ultra5-auxio-mcu-2026-11-22/`.

The QEMU 10.2.2 cross-check uses the original flash and a
`TI-UltraSparc-IIi` CPU. After the earlier redirected NVRAM read and guest
window initialization, the debugger enters the original ROM at offset
`0x18e8`. Execution from there to `0x0714` uses the guest's own instructions
and operands. QEMU reports `%ver = 0x0017_0012_9100_0507`, both MCU store
values, zero at each loop exit, and CWP 0 in the return delay slot. This
checks CPU control flow and operands. The board register readback and
serial transcripts are Helios tests. The script and trace are retained as
`qemu-mcu.py` and `qemu-mcu.json`.

Validation recorded 888 passing test executions in the ordinary suite and
13 passing original-ROM checks when selected explicitly. Formatting, offline
all-target Clippy with warnings denied, and the four built-in demos passed.

For the next checkpoint, determine the external-cache SRAM capacity from
the board evidence. Follow the alias probe at offsets `0x70c–0x744` through
its diagnostic write and read ASIs and check the size it returns. The IIi
manual documents the data and tag paths separately in Appendix A.9.
