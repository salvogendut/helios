# Ultra 5 first serial text

Original OBP 3.31.0 configures both SAB 82532 channels and transmits a
complete line on serial A. The headless example accepts an original flash,
an NVRAM seed and an instruction limit. It reads both files without
modifying them. [Run and reproduce](../docs/ultra5.md#run-the-first-serial-diagnostic)

| NVRAM bytes 0x10–0x17 | Byte 1 | Serial A | First / last TX instruction | Instructions before next unsupported access |
| --- | ---: | --- | ---: | ---: |
| Zero | 1 | `Hardware Power ON\r\n` | 607 / 1075 | 1115 |
| `soft-rst` | 1 | `Software Power ON\r\n` | 595 / 1063 | 1084 |
| Zero | 0 | Empty | — | 811 |
| `soft-rst` | 0 | Empty | — | 780 |

The first serial access on both marker paths is `stba` at flash offset
`0x033c` to PA `0x01ff_f140_002c`. The firmware loads the offset and value
from its table in flash. At `0x0368` it writes a character to the channel A
FIFO. Only the XF command at `0x0380` makes the byte available to the host.
Channel B remains silent in all four runs. Transfer completion is synchronous.

## Reference trace

Evidence and debugger scripts are in `local/ultra5-reset-trace-2026-11-22/`.
On the generic QEMU 10.2.2 sun4u board, the run with only the earlier
NVRAM read redirected loops during Super I/O identification. In a second
comparison the debugger also supplies the two Super I/O index reads, first
`0x88` and then zero, and restores the address register after each read.
The 181 flash-relative PC/nPC/instruction triples from offset `0x1614`
through the first serial store match Helios on both marker paths
(FNV-1a `0xcf95e55aca0a8416`). QEMU is paused before executing that
store. Helios executes the store and produces the serial output using
its own device models. The Super I/O index-port strap at `0x398` is an
assumption pending physical-board confirmation.

## Resume from the next EBus access

Both marker paths stop at virtual PC `0xffff_ffff_f000_18fc`, nPC
`0xffff_ffff_f000_1900`. Identify the register receiving the big-endian
32-bit store of `0xffff_ffff` to PA `0x01ff_f172_6000` before extending the
board map. Replay all four seeds and retain the per-byte TX assertions.
Serial receive and the interactive Forth exchange are subsequent milestones.
[Acceptance criteria](../docs/roadmap.md#ultra-5-serial-console)

The ordinary suite recorded 883 passing test executions. The 12 optional
original-ROM checks passed when run explicitly. Formatting, offline
all-target Clippy with warnings denied, and all four built-in demos passed.
The local evidence includes the test logs, both QEMU trace variants, raw
serial bytes and the four stop records.
