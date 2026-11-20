# Ultra 5 second instruction

The branch `feat/ultra5-second-instruction` starts at `eeb50db`. Its bounded
entry point executes OBP's original `ldxa` and `stxa` at flash offsets
`0x20` and `0x24`. Both go through the same privileged ASI/address checks.
The one-word API retains its original instruction budget.
[Usage, register scope and local-ROM command](../docs/ultra5.md#execute-the-first-two-instructions)

| At the stop before word three | Helios, POR | Helios, seeded original ROM test | QEMU 10.2.2 |
| --- | --- | --- | --- |
| PC | `0xfffffffff0000028` | `0xfffffffff0000028` | `0x1fff0000028` |
| nPC | `0xfffffffff000002c` | `0xfffffffff000002c` | `0x1fff000002c` |
| %g2 | 0 | 2 (old DC bit) | 0 |
| CCR | 0 | `0x44` | `0x44` |
| LSU control | 0 | 0 (previously 2) | Unobserved |

The synthetic pair test starts with IC set, distinct GPR values, and CCR
`0xa5`. It compares the entire post-execution CPU with the expected state
and puts an observable write in the third slot. The original ROM test runs
both from POR and with a seeded DC bit. Separate fixtures write nonzero cache
enable values from a guest register and read them back. Invalid access
fixtures check privilege, address, ASI, unsupported bits and partial progress.

Local evidence is under `local/ultra5-second-instruction-2026-11-21/`.
The QEMU probe records physical words, two debugger stops, and before/after
PC, nPC, %g2 and CCR from the unmodified vendor image. The mutation log
comes from a throwaway copy with the actual LSU register assignment removed.
The seeded test detected that mutation with LSU control still at 1.

The full offline suite reported 844 passing executions and three ignored
local-ROM checks. All three passed when run explicitly. Formatting, offline
all-target Clippy with warnings denied, and the four built-in demos passed.
