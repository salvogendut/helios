# Ultra 5 reset decision and first ASI write

Main advanced to `d127db1` after reproducing the 17-step QEMU trace,
863 passing test executions, all seven explicit ROM checks, formatting,
all-target Clippy and the four demos.

The reset firmware reads a guest-supplied M48T59 SRAM image. Test images
are 8192 bytes and the marker spans offsets `0x10..0x18`. Blank and
nonblank seeds are fixture inputs. Hardware retains those bytes across
power loss while its battery is good. The address inference combines
the Ultra 5/10 EEPROM node with the reset value documented for the PCIO
base register. [Board sources and test command](../docs/ultra5.md#execute-the-nvram-reset-decision)

The original ROM executes LDXA, CMP, BNE on XCC, then its delay slot.
An exact marker falls through to offset `0x2154`. Neither eight zero
bytes nor a match confined to the low word passes the comparison.
Both take the branch to `0x21c0`. With the deterministic zero in reset
`%g7`, even an exact marker fails the additional comparison and rejoins
the other path. The zero-marker and exact-marker runs stop after 22
and 31 original instructions at PC/nPC `0xfffffffff00015d4` /
`0xfffffffff00015d8`.

The CPU comparison on QEMU 10.2.2 used the same original ROM. After
instruction 17 the probe seeded eight bytes of RAM at `0x1000` and
redirected `%o2` there. This checks the load, flags, branch and delayed
PC/nPC. The actual Ultra 5 physical address was used on the Helios side.
The three QEMU traces, scripts and validation logs are under
`local/ultra5-reset-nvram-2026-11-21/`.

The next word `0x87802058` writes `0x58` to `%asi`; it is followed
by `stxa` through the ASI register and `membar`. The reset checkpoint
covers reads of ordinary SRAM. The clock and watchdog register block,
NVRAM writes and physical-hardware verification are open.

Final validation passed with 867 normal test executions, eight explicit
local-ROM checks, formatting, offline all-target Clippy with warnings
denied and all four demos.
