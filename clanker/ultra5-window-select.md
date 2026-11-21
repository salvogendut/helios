# Ultra 5 window selection

Parent `e8d6a9c` was merged after its four-step QEMU trace and five local-ROM
checks were reproduced. This checkpoint adds the two WRPRs that follow the
CWP read. The original-ROM debugger trace stops after exactly six instructions.

| Retired flash word | CWP | %g1 | Next Helios PC |
| --- | --- | --- | --- |
| `0x83524000` at `0x210c` | 7 | 7 | `0xfffffffff0002110` |
| `0x93902000` at `0x2110` | 0 | 7 | `0xfffffffff0002114` |
| `0x93906000` at `0x2114` | 7 | 7 | `0xfffffffff0002118` |

QEMU 10.2.2 on generic sun4u with a TI UltraSPARC-IIi reported the same
window transitions and flash offsets. CCR stayed `0x44` in QEMU. The
original-ROM fixture in Helios also compares all physical registers and
control state after restoration with a copy advanced through two NOPs.
[Sources and test command](../docs/ultra5.md#switch-and-restore-the-window)

A second, eleven-instruction synthetic guest exercised both directions of
the 7/0 overlap. It wrote `0x171` to window 7's %o0 and read it from
window 0's %i0. Writing `0x2e2` there changed window 7's %o0. The two
windows retained local values `0x271` and `0x333`. Helios and QEMU agreed.
Tracked tests also sweep all eight windows and lanes, XOR two windowed
operands, exercise high-bit truncation, execute a WRPR in a delay slot and
check full-state preservation on denied writes.

A throwaway build reversing the overlap direction failed the QEMU-derived
guest regression. The final offline suite reported 860 passing executions
and six ignored local-ROM checks. All six passed when run explicitly.
Formatting, all-target Clippy with warnings denied and all four demos passed.

The raw traces, bounded QEMU probes and validation logs are under
`local/ultra5-window-select-2026-11-21/`. The next original word at offset
`0x2118` remains unexecuted.
