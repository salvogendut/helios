# Ultra 5 E-cache and firmware banner

From `eadebbd`, original OBP 3.31.0 runs its E-cache sizing helper twice,
returns 2 MiB both times, and writes ELIM 3. With diagnostics enabled the
firmware transmits its complete version string on serial A. The cold and
soft-marker paths both clear exactly 32,768 tags, in descending address order.
Seed byte 1 governs the serial output. [Reproduce the run and the two stopping points](../docs/ultra5.md#e-cache-sizing-and-the-firmware-version-banner)

| Reset marker | Byte 1 | Instructions retired before stop | Serial A bytes |
| --- | ---: | ---: | ---: |
| Zero | 0 | 171,730 | 0 |
| Zero | 1 | 173,441 | 107 |
| `soft-rst` | 0 | 171,971 | 0 |
| `soft-rst` | 1 | 174,083 | 132 |

The original-ROM test checks both probe results, counts and addresses every
tag store, and compares the complete serial byte stream with the expected
transcript. After each run, a separate synthetic instruction stream reads
all tag cells through the staging register. Every tag reads back as
`0x30000` (invalid, both odd-parity bits set). The fixture stores diagnostic
data and tags independently. Timing and ordinary cache traffic remain open.

The reference machine's POST reports 2048 KB, and its CPU node reports
size `0x00200000`, line size `0x40`, associativity 1 and mask `0x91`.
The IIi manual specifies index bits [20:3] for data and [20:6] for tags.
The single data-word wrap used by OBP at offset `0x200000` is inferred from
the sizing algorithm and independently reported capacity. The fixture
rejects other out-of-range data addresses. [Hardware transcript](https://rob.com/sun/sun4u-ss5/) · [Sun IIi manual, §§18.5 and A.9](https://www.cs.tufts.edu/comp/150PAT/arch/sparc/UltraSPARC-IIi-user-805-0087-02.pdf)

The bounded QEMU 10.2.2 run used the previous checkpoint's debugger setup.
Its four store operands agree with the guest code. The final diagnostic
load returns zero on generic sun4u. The pinned QEMU helper leaves these
diagnostic ASIs unimplemented. Hardware supplies the independent expected
capacity and banner. [QEMU SPARC ASI handlers](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/ldst_helper.c)

Validation recorded 891 passing standard test executions and 14 passing
original-ROM checks. Formatting, offline all-target Clippy with warnings
denied, and all four built-in demos passed.

Evidence is in `local/ultra5-ecache-2026-11-22/`, including byte streams,
exact stops, the QEMU probe and validation logs. Resume the cold path at
the Super I/O keyboard UART byte write (PA `0x01ff_f130_83f9`). The soft
path next clears D-MMU Tag Access (ASI `0x58`, offset `0x30`) before
clearing the I/D TLBs. Both still need further board initialization to reach
an interactive serial prompt.
