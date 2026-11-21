# Ultra 5 reset-target CWP read

Parent commit `077110f` stops after the branch. From that state, one
`BootFlash::step` retires the original `rdpr %cwp, %g1` at flash offset
`0x210c`. The word is fetched through the live CPU PC. The same operation
can execute a synthetic delay slot with a nonsequential nPC.
[Manuals and local-ROM test command](../docs/ultra5.md#read-the-current-window-pointer)

| After word four | Helios from POR | QEMU 10.2.2 sun4u / TI UltraSPARC-IIi |
| --- | --- | --- |
| PC | `0xfffffffff0002110` | `0x1fff0002110` |
| nPC | `0xfffffffff0002114` | `0x1fff0002114` |
| %g1 | 7 | 7 |
| %g2 | 0 | 0 |
| CCR | 0 | `0x44` |

The reset value of 7 is a fixture choice. The IIi manual specifies unknown
CWP at power-on. The four-step QEMU trace captures CWP alongside %g1 before
and after each step. The reset image SHA-256 is
`fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`.
The probe, raw results and test logs are under `local/ultra5-cwp-2026-11-21/`.

The CPU regression seeds all eight CWP values, all 32 destination encodings,
distinct registers, nonzero CCR, LSU state and a pending branch target. It
checks the entire resulting CPU. The flash fixtures cover privilege faults
including destination %g0, other selectors, malformed encodings, failed
fetches and a visible fifth-word sentinel. The original ROM run stops after
four instructions.

Replacing the CWP read with a constant 7 in a throwaway build failed the
seeded regression. The offline suite reported 854 passing executions and
five ignored local-ROM checks. All five passed when explicitly enabled.
Formatting, all-target Clippy with warnings denied and all four demos passed.

The word at the stop address, flash offset `0x2110`, is `0x93902000`
(`wrpr %g0, 0, %cwp`). It was inspected in the image and left unexecuted.
