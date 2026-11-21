# Ultra 5 reset-time board read

The six-step original-ROM trace and eleven-step window-alias trace on
`8d14bb5` reproduced byte for byte in QEMU 10.2.2. Review also repeated
860 passing test executions, six explicit local-ROM tests, Clippy, formatting
and the four demos before fast-forwarding main.

The next trace used the original OBP 3.31.0 flash. A paused QEMU with
`-M sun4u -cpu TI-UltraSparc-IIi` took exactly 17 single steps. Its
flash offsets, words, output registers and window selections agree with the
Helios trace. The test checks the QEMU value for every destination in the
eleven setup instructions. At offset `0x2144`, disassembly of the next word
reads `ldxa [%o2] #ASI_PHYS_BYPASS_EC_WITH_EBIT, %o1`.

| At the unretired load | Value |
| --- | --- |
| Helios virtual PC / nPC | `0xfffffffff0002144` / `0xfffffffff0002148` |
| CPU request | Read 8 bytes, ASI `0x15`, PA `0x01fff1000010` |
| Comparison operand in %o0 | `soft-rst` |
| Original %o1 before writeback | `0x01ff00000000` |

A separate three-instruction QEMU guest read the seeded physical bytes
`11 22 33 44 aa bb cc dd` at `0x30` into the address-source register.
The Helios flat-memory regression exercises the same encoding and value.
The original-ROM regression attempts the board read once, checks the
reported request, and compares the complete CPU against its pre-read copy.

The Ultra 5 Service Manual identifies the non-cacheable CPU I/O range and
the strapped boot PROM base. Establish which device responds at
`0x01fff1000010` on reset and what it returns on a cold boot before using
the response to execute the firmware comparison. The literal in %o0 is the
expected comparison operand. [Board address map](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-7763-12/805-7763-12.pdf#page=229)

Validation passed with 863 normal test executions, all seven explicit
local-ROM checks, formatting, offline all-target Clippy and all four
demos. Bounded probe scripts, register traces and validation logs are in
`local/ultra5-reset-probe-2026-11-21/`. [Firmware and reproduction command](../docs/ultra5.md#reach-the-first-board-data-read)
