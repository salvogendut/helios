# Ultra 5/10 interrupt and window initialization

The preceding MMU-context checkpoint at `6fd5ac4` passed 871 test
executions and nine explicit local-ROM checks on review. The two QEMU
firmware traces and the seeded context probe reproduced byte for byte.

This checkpoint executes ten original words at offsets `0x15ec..=0x1610`.
The zero-filled and matching-marker paths retire 38 and 47 instructions.
The next PC/nPC are `0xfffffffff0001614` and `0xfffffffff0001618`.
The original flash there contains `0x8f902005` (`wrpr 5, %tl`).
The next milestone is initialization of the five trap levels.

Both QEMU firmware traces use the original 1 MiB image. After the first
17 instructions the debugger redirects the NVRAM read operand to seeded
RAM because generic sun4u has a different board map. Helios uses the
physical board address. The independent QEMU program runs 40 instructions
to seed registers, read them, execute the ten writes and read them again.
The populated TBA, SOFTINT, CLEANWIN, OTHERWIN, WSTATE, CANRESTORE and
CANSAVE distinguish the firmware writes from retaining the reset values.

Tracked tests read the controls through guest instructions, verify each
write with nonzero CCR, switch all four global banks, and check failure
atomicity on unsupported modes and after dropping privilege. The earlier
LSU test now seeds its expected result in the active global bank.
Probe scripts, monitor transcripts and validation logs are under
`local/ultra5-reset-windows-2026-11-21/`. The source ROM is unchanged.
[Firmware words, supported modes and reproduction command](../docs/ultra5.md#initialize-interrupt-and-register-window-state)

Validation passed with 875 normal test executions, all ten explicit
local-ROM checks, formatting, offline all-target Clippy with warnings
denied, and the four demos.
