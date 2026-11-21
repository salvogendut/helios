# Ultra 5/10 firmware reset instructions

The headless Ultra 5/10 fixture executes six original OBP 3.31.0 instructions on a V9 CPU, through the first register-window switch and restore.

## Reset and flash mapping

The UltraSPARC-IIi enters RED_state at the default virtual reset vector plus `0x20`. PSTATE.AM is clear, so PC and nPC retain all 64 bits. The instruction-side physical read uses PA[40:0]. The board has 1 MiB of flash divided equally between OBP and POST/OBDiag. [UltraSPARC IIi User’s Manual, §17.3.2 and tables 17-3/17-5](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=303) · [Sun Ultra 5 Service Manual, §C.1.6.3](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-7763-12/805-7763-12.pdf#page=224)

| Quantity | Value |
| --- | --- |
| Default virtual RED_state vector base | `0xffff_ffff_f000_0000` |
| Architectural PC / nPC | `0xffff_ffff_f000_0020` / `0xffff_ffff_f000_0024` |
| Physical flash read | `0x01ff_f000_0020` |
| Offset in the 1 MiB flash | `0x20` |
| Big-endian instruction | `0xc4d808a0` (`ldxa [%g0] #ASI_LSU_CONTROL_REG, %g2`) |

## Reconstruct and test the original firmware

Use the `flash-update-Ultra510-18` utility from Sun patch 106121-18. The extractor checks the updater SHA-256 and follows both board descriptors to the OBP and POST records. For each board it takes exactly `0x80000` bytes per half, including the `OBMD` headers, then joins OBP followed by POST. Both descriptors must yield identical bytes and the expected image digest.

| Board | OBP file offset | POST file offset |
| --- | --- | --- |
| `SUNW,375-0009` | `0x5f078` | `0x15f0b8` |
| `SUNW,375-0066` | `0xdf098` | `0x1df0d8` |

| Artifact | SHA-256 |
| --- | --- |
| `106121-18.zip` | `21561476a550286a0fc28fd331f231b59f9db2ab774779bb3aec164d02d8153f` |
| `flash-update-Ultra510-18` | `e0ef541b64de9ab23b9423fce71fd56a7ddcc29cb28de8b66a4313d48516a29e` |
| Reconstructed 1 MiB image | `fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616` |

From the repository root, run the extractor and the explicit local-ROM test. The extractor writes a JSON manifest next to the image. The test uses Python 3 to verify the full image digest before checking the first fetch.

```sh
python3 tools/ultra5-fixture/extract.py \
  local/firmware/ultra5/106121-18/flash-update-Ultra510-18 \
  --output local/firmware/ultra5/ultra5-obp331-post310-1mb.bin
cargo test --locked --offline --test ultra5_first_fetch -- --ignored --nocapture
```

## QEMU comparison

QEMU 10.2.2, paused at startup with `-S -M sun4u -cpu TI-UltraSparc-IIi` and the reconstructed image, gives the following monitor results.

```text
(qemu) xp /1wx 0x1fff0000020
000001fff0000020: 0xc4d808a0
(qemu) x /1i 0x1fff0000020
0x1fff0000020:  ldxa  [ %g0 ] #ASI_LSU_CONTROL_REG, %g2
```

QEMU’s generic sun4u machine reports PC/nPC in the low physical range. This comparison verifies the bytes and disassembly. The Sun reset table supplies the architectural PC/nPC used by Helios.

## Execute the first instruction

Create the CPU with `V9Cpu::from(PowerOnReset::ultra_sparc_ii())` and call
`BootFlash::execute_first_instruction(&mut cpu)`. The CPU fetches through its
reset address and executes the flash word with the same decoder used by
`V9Cpu::step`. The original word reads the privileged 64-bit LSU control
register at ASI `0x45`, VA zero. All its bits are clear at power-on, so
`%g2` receives zero. The other integer registers and CCR retain their values.
[IIi reset register table](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=305) · [LSU register address and format, Appendix A.6](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=412)

After the step, PC is `0xffff_ffff_f000_0024` and nPC is
`0xffff_ffff_f000_0028`. Execution stops before the following `stxa`.
A single debugger step in QEMU 10.2.2 independently observed `%g2` change
from `0x08000000` to zero, with PC/nPC advancing by four in its low address
range. Sun specifies the initial general-register contents as unknown.

## Execute the first two instructions

From the same power-on state, `BootFlash::execute_first_two_instructions(&mut cpu)`
returns the two flash fetch records. At offset `0x24`, OBP executes
`0xc0f008a0` (`stxa %g0, [%g0] 0x45`). It writes zero to LSU control.
`%g2` retains the value read by the preceding `ldxa`, and CCR is preserved.
Execution stops with PC/nPC at `0xffff_ffff_f000_0028` / `0xffff_ffff_f000_002c`.
If the second instruction fails, the first is already retired and the CPU
stays at the second instruction with its effects unapplied.

The fixture models register storage for the instruction and data cache enables
(IC and DC, bits 0 and 1). The all-off write clears the register. Writes that
set other bits return an explicit unsupported-operation error. Cache timing,
MMU enable, watchpoints and parity injection are outside this checkpoint.
[UltraSPARC IIi User’s Manual, Appendix A.6](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=413)

Two QEMU 10.2.2 single steps on the same image reached low PC/nPC
`0x01ff_f000_0028` / `0x01ff_f000_002c`, with `%g2 = 0` and `CCR = 0x44`.
As a separate check on the store, the Helios local-ROM test also starts with
DC set and CCR at `0x44`. It verifies `%g2 = 2`, LSU control zero, and the
original CCR after the pair.

## Execute through the reset branch

Call `BootFlash::execute_reset_branch(&mut cpu)` from power-on to retire the
first three words. The third word at offset `0x28`, `0x30680839`, encodes
`ba,a,pt %xcc`. Its signed 19-bit displacement is `0x839` words. Adding
`4 × 0x839` to the branch PC selects flash offset `0x210c`. The annul bit
skips the instruction at `0x2c`. The fixture returns before the first
instruction at the target, with virtual PC/nPC at
`0xffff_ffff_f000_210c` / `0xffff_ffff_f000_2110`.
[SPARC Architecture Manual, Version 9, Appendix A.7](https://icps.u-strasbg.fr/~loechner/enseignement/SPARC/v9.pdf#page=173)

Three debugger steps on the same ROM in QEMU 10.2.2 reached low PC/nPC
`0x01ff_f000_210c` / `0x01ff_f000_2110`. The fixture implements the
unconditional BPA form for ICC and XCC with either prediction hint.
Other BPcc conditions return an explicit unsupported-instruction error.

## Read the current window pointer

After `execute_reset_branch`, call `BootFlash::step(&mut cpu)` once. This
fetches the word at the live virtual PC using the same physical flash mapping
as the reset prefix. Offset `0x210c` contains `0x83524000` (`rdpr %cwp, %g1`).
The instruction copies the current window index into `%g1` and preserves CCR.
The fourth step ends at virtual PC/nPC `0xffff_ffff_f000_2110` /
`0xffff_ffff_f000_2114`.

The IIi reset table leaves CWP unknown at power-on. Helios chooses window 7
for deterministic reset, matching the guest-visible value on QEMU 10.2.2
with the TI UltraSPARC-IIi CPU. Four debugger steps on the original ROM
showed `%g1` change from 0 to 7 and CCR stay at `0x44`.
[UltraSPARC IIi reset table 17-5](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=304) · [SPARC V9 RDPR, Appendix A.43](https://icps.u-strasbg.fr/~loechner/enseignement/SPARC/v9.pdf#page=235)

## Switch and restore the window

Two more calls to `BootFlash::step` execute `wrpr %g0, 0, %cwp` at flash
offset `0x2110` and `wrpr %g1, 0, %cwp` at `0x2114`. OBP first selects
window 0 and then restores the window saved in `%g1`. Helios and QEMU
10.2.2 both observed CWP change from 7 to 0 to 7 while `%g1` retained 7.
The six-instruction run ends with virtual PC/nPC at
`0xffff_ffff_f000_2118` / `0xffff_ffff_f000_211c`.

WRPR computes the XOR of its operands in the old window. The IIi keeps the
low three bits when writing CWP. Each of the eight windows has private locals.
Window `w`'s outs are the ins of window `(w + 1) % 8`. Globals remain
visible across CWP writes. This checkpoint covers explicit window selection.
SAVE, RESTORE and window traps remain future work.
[UltraSPARC IIi manual §14.2.1](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=223) · [SPARC V9 §5.1.1.3 and Appendix A.62](https://icps.u-strasbg.fr/~loechner/enseignement/SPARC/v9.pdf#page=266)

With the reconstructed image installed, run the six local-ROM checks together.

```sh
cargo test --locked --offline --lib --test ultra5_first_fetch --test ultra5_first_instruction --test ultra5_reset_branch --test ultra5_reset_cwp --test ultra5_window_select -- --ignored --nocapture
```
