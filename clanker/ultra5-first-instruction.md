# Ultra 5 first instruction

The first-fetch fixture landed on main at `0497c80`. The follow-up on
`feat/ultra5-first-instruction` drives that fetch from a V9 CPU initialized
with the UltraSPARC-IIi power-on state. It retires the fetched word through
the regular V9 decoder. The fixture checks the reset PC/nPC before each
call, so a second call after the OBP instruction stops before fetching.

| Observation | Helios | QEMU 10.2.2, generic sun4u with TI-UltraSparc-IIi |
| --- | --- | --- |
| Instruction | `0xc4d808a0` | `0xc4d808a0` |
| Result | `%g2 = 0` | `%g2 = 0` (previously `0x08000000`) |
| Next PC | `0xfffffffff0000024` | `0x1fff0000024` |
| Next nPC | `0xfffffffff0000028` | `0x1fff0000028` |

The Helios PC/nPC expectations use the virtual reset vector in the IIi manual.
Table 17-5 specifies a zero POR value for every LSU_CONTROL bit. Appendix
A.6 specifies ASI `0x45`, VA zero. The implemented `ldxa` form takes the ASI
from the instruction and adds the two register operands. It requires
privilege and checks the address before writing the destination. Unsupported
ASI operations return a fixture error with the CPU state preserved.
[Hardware sources and test commands](../docs/ultra5.md#execute-the-first-instruction)

The local-ROM test starts from power-on. A separate synthetic run seeds all
writable integer registers with distinct nonzero values, checks the other
registers and CCR, and places a visible write in the next instruction.
Additional tests vary the fetched word, alias the destination with an address
operand, and exercise privilege, address, unsupported form and data-bus errors.
A CPU test seeds a nonzero upper LSU bit to check full-width writeback.

The default offline suite recorded 838 passing executions, including one
existing MXCC child-process repeat. Both opt-in firmware tests passed when
explicitly enabled. Formatting, all-target Clippy with warnings denied, and
all four built-in demos passed.

Evidence is in `local/ultra5-first-instruction-2026-11-20/`. The QEMU probe
starts with `-S`, reads the physical flash via QMP, sends one debugger step
over a private Unix socket, and records the stopped register state. Both
emulators used the 1 MiB image with SHA-256
`fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`.
