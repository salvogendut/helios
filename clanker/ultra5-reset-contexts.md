# Ultra 5 reset MMU contexts

Before fast-forwarding main to `f5c9a26`, all three NVRAM decision traces
reproduced byte for byte in QEMU 10.2.2. The review also reproduced 867
passing test executions, eight local-ROM checks, formatting, Clippy and
the four demos.

The six original instructions at flash offsets `0x15d4..=0x15e8` select
the D-MMU ASI, clear the primary and secondary context registers with a
sync after each store, and clear `%g7`. Both the zero-filled and matching
NVRAM paths reach `0xfffffffff00015ec`. HMP disassembles the next word
`0x9190200f` as `wrpr 0xf, %pil`. No instruction at that PC is retired.

The original-ROM checks include unseeded runs and runs with a synthetic
preamble that leaves the contexts at `0x357` and `0x6ac` and `%asi` at
`0x15` before entering the POR vector. A CPU copy executes a sync and
literal-ASI readback after each original word. Both CC banks, CWP and
the visible integer registers are checked across the transition.

The QEMU original-ROM run reuses the preceding checkpoint’s redirected
NVRAM read. A separate 19-instruction QEMU guest checked seeded context
values, literal and register-selected ASIs, signed offsets and independent
clearing. The monitor reports context IDs in decimal. Probe scripts, JSON
traces and validation logs are in `local/ultra5-reset-contexts-2026-11-21/`.
[Reproduction and manuals](../docs/ultra5.md#clear-the-primary-and-secondary-mmu-contexts)

The next firmware group writes PIL and other privileged registers before
continuing initialization. The current reset path uses physical flash
fetch and synchronous loads and stores. Translation, interrupts and
instruction-side context synchronization require separate implementation.

Validation passed with 871 normal test executions, all nine explicit ROM
checks, offline all-target Clippy with warnings denied, formatting and
the four demos.
