# Ultra 5 reset branch

The parent commit is `4c24977`. The original OBP 3.31.0 reset sequence retires
one LSU load, one LSU store and one predicted unconditional branch. Execution
stops before the first target word. The new entry point returns all three
fetch records. The one- and two-word entry points retain their budgets.
[Firmware, instruction reference and test command](../docs/ultra5.md#execute-through-the-reset-branch)

| After the branch | Helios from POR | QEMU 10.2.2 generic sun4u |
| --- | --- | --- |
| PC | `0xfffffffff000210c` | `0x1fff000210c` |
| nPC | `0xfffffffff0002110` | `0x1fff0002110` |
| %g2 | 0 | 0 |
| CCR before and after the branch | 0 | `0x44` |

The QEMU probe used the same 1 MiB ROM (SHA-256
`fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`).
The raw register stops, physical words and the bounded probe are in
`local/ultra5-reset-branch-2026-11-21/`.

The synthetic ROM in `tests/ultra5_reset_branch.rs` puts distinct register
writes in the delay slot and at the branch target. Both destination registers
must retain their seeded values. A backward non-annulled branch runs its delay
slot and preserves nonzero CCR. Additional cases exercise signed displacement
limits, both prediction hints and atomic rejection of unsupported words.

In a throwaway build, disabling annul for BPA left PC at the delay slot. The
synthetic reset-branch regression failed as expected. The exact ROM branch
also preserved CCR `0x05` in a separately seeded flat-memory test.

Validation recorded 849 passing test executions and four ignored local-ROM
checks. All four passed when run explicitly. Formatting, all-target Clippy
with warnings denied, and all four built-in demos passed.
