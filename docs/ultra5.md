# Ultra 5/10 firmware reset instructions

The headless Ultra 5/10 fixture runs original OBP 3.31.0 far enough to
transmit its version banner on serial A, clear the external-cache tags and
complete the cold-start probe with the keyboard disconnected. Both boot paths
clear the instruction and data TLBs and reach the first physical DRAM probe.
An interactive OpenBoot prompt remains in development. [Serial-console roadmap](roadmap.md#ultra-5-serial-console)

## Run the first serial diagnostic

From the repository root, use the reconstructed 1 MiB flash and a temporary
8 KiB NVRAM seed. This example sets byte 1 to enable the diagnostic, leaves
the reset marker clear, and stops at the last byte of the line.

```sh
u5_nvram=$(mktemp)
python3 -c 'import sys; from pathlib import Path; Path(sys.argv[1]).write_bytes(bytes([0, 1]) + bytes(8190))' "$u5_nvram"
cargo run --quiet --locked --offline --example ultra5_serial -- \
  local/firmware/ultra5/ultra5-obp331-post310-1mb.bin "$u5_nvram" 1075
rm -- "$u5_nvram"
```

Standard output carries the 19 guest bytes `Hardware Power ON\r\n`.
The example reports its instruction count and PC on standard error and
keeps NVRAM writes in memory. With ASCII `soft-rst` at offsets
`0x10..0x18` and byte 1 set, the guest instead emits
`Software Power ON\r\n`. A zero diagnostic byte suppresses both messages.
The two-channel SAB 82532 drives the external serial ports. The Super I/O
UARTs serve the keyboard and mouse. [Sun Ultra 5 Service Manual, §§C.1.6.1–2](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-7763-12/805-7763-12.pdf#page=222)

| Original OBP event | Cold marker | Matching marker |
| --- | ---: | ---: |
| Channel A CCR0 write retires | 219 | 228 |
| First byte transmitted with diagnostics enabled | 607 | 595 |
| Final line feed transmitted | 1,075 | 1,063 |

The first access is a byte store from virtual PC `0xffff_ffff_f000_033c`
to physical `0x01ff_f140_002c`. OBP writes `0xc3` through ASI `0x15` to
select asynchronous mode and power up channel A. The firmware reads the
register offset and value from flash, repeats its configuration on channel B,
polls STAR, writes each character to XFIFO and issues the transmit command
at PC `0xffff_ffff_f000_0380`. The fixture completes transmission
synchronously and exposes the transmitted bytes through
`Ultra5ResetBoard::take_serial_output()`. Serial input, baud-rate timing
and interrupt delivery remain open. [Siemens SAB 82532 register descriptions](https://media.digikey.com/pdf/Data%20Sheets/Infineon%20PDFS/SAB%2CSAF82532.pdf)

The reset board models the host and APB bus numbers used to route the
onboard PCIO configuration access, the reset BAR1 value, and the three
observed EBus setup writes. It assumes the documented Super I/O index-port
strap at `0x398`. The installed board's strap has not been independently
verified. The fixture implements the one-shot identification byte and
double-write configuration protocol. PCI enumeration, BAR relocation,
EBus wait-state timing and the mouse UART remain open. [PCIO reset mapping, tables 36–37](https://www.artisantg.com/info/force_cpci_520_series_manual.pdf) · [National Super I/O, §§2.2–2.4](https://ardent-tool.com/datasheets/NS_PC87332VLJ.pdf)

## Front-panel LED and DRAM controller setup

At virtual PC `0xffff_ffff_f000_18fc` OBP writes all ones to physical
`0x01ff_f172_6000`, turning on the front-panel LED. The LED is bit 0 of a
little-endian 32-bit PCIO AUXIO register. Its on-state reads as `1` through
ASI `0x1d` and `0x0100_0000` through the big-endian ASI `0x15`. The
fixture starts with the LED off. [PCIO LED definition](https://github.com/torvalds/linux/blob/v6.12/arch/sparc/include/asm/auxio_64.h) · [SPARC64 EBus driver](https://github.com/torvalds/linux/blob/v6.12/arch/sparc/kernel/auxio_64.c)

The fixture reports a TI UltraSPARC-IIi mask revision of 9.1, seen on an
Ultra 5/10 with the same OBP revision. The guest reads `%ver` to choose the
DRAM timing word. On both reset paths the routine at flash offset `0x1468`
performs these operations in order.

| Register | Physical address | Value written | Following guest operation |
| --- | --- | --- | --- |
| Mem_Control1 | `0x01fe_0000_f018` | `0x0c4a_ab14` | Read the LED word 1,000 times |
| Mem_Control0 | `0x01fe_0000_f010` | `0x8000_053d` | Read the LED word another 1,000 times, then return |

The MCU registers implement reset values and masked readback. Writing the
refresh-enable bit records the configuration. DRAM storage, refresh timing and ECC
error generation are still open. [Sun IIi User’s Manual, tables 18-3 and 18-7](https://www.cs.tufts.edu/comp/150PAT/arch/sparc/UltraSPARC-IIi-user-805-0087-02.pdf) · [Ultra 5/10 hardware console and device tree](https://rob.com/sun/sun4u-ss5/)

After returning, OBP disables E-cache error reporting, clears the sticky
fault flags and calls its external-cache size probe. Its first diagnostic
store is `stxa %g0,[%o1] 0x76` at `0xffff_ffff_f000_0714`, with
`%o1 = 1 << 39`.

## E-cache sizing and the firmware version banner

Run the [serial example](#run-the-first-serial-diagnostic) with the same
cold NVRAM seed and a limit of `9008` to see the complete version string.
With the `soft-rst` marker, the last character appears at instruction `8977`.

```text
@(#) Sun Ultra 5/10 UPA/PCI 3.31 Version 0 created 2001/07/25 20:36
```

The mask-9.1 CPU module in the reference machine has a 2 MiB direct-mapped
E-cache with 64-byte lines. Its POST transcript and OpenBoot device tree
both report the capacity. The headless fixture models diagnostic data
and tag SRAM for that module. Normal cache fills, writeback and cache
timing remain open. [Ultra 5 hardware transcript and device tree](https://rob.com/sun/sun4u-ss5/)

OBP writes the data array through ASI `0x76`, reads cell zero through ASI
`0x7e` and gets `0x0020_0000`. It programs UPA_CONFIG.ELIM to 3, then
clears all 32,768 tags through the tag staging register and indexed stores.
The data word at offset 2 MiB aliases cell zero in the model. This specific
out-of-range access is inferred from the ROM's sizing algorithm and the
capacity reported by hardware. The manual defines the in-range index bits.
[Sun IIi User’s Manual, §18.5 and Appendix A.9](https://www.cs.tufts.edu/comp/150PAT/arch/sparc/UltraSPARC-IIi-user-805-0087-02.pdf)

| Diagnostic run | Serial A after the banner | Next access |
| --- | --- | --- |
| Cold, 30,726,251 instructions | `Probing keyboard Done`, `Clearing E$ Tags  Done`, `Clearing I/D TLBs Done`, then `Probing Memory` | PC `0xffff_ffff_f000_0fec` reads PA zero through ASI `0x14`. |
| Soft marker, 175,159 instructions | `Clearing E$ Tags  Done`, `Clearing I/D TLBs Done`, then `Probing Memory` | Same physical DRAM read. |

These limits stop immediately before the respective accesses. With NVRAM
byte 1 clear, both paths reach the same respective PCs and serial A stays
silent.

## Cold-start keyboard probe

The board device tree assigns the keyboard to EBus offset `0x3083f8`.
On a cold start OBP enables Super I/O UART1 at COM1, disables interrupts,
enables the FIFOs and sets the divisor to 96. It restores LCR to `3`
(eight data bits, one stop bit, no parity). OBP transmits one keyboard
reset byte (`0x01`) and polls for transmitter completion. At this divisor,
the ten-bit frame takes 8.320 ms of virtual time with the documented
24 MHz oscillator and divide-by-13 prescaler. The disconnected keyboard
supplies no receive bytes. Attached input devices, UART interrupts and other
framing modes remain open. [Hardware device tree](https://rob.com/sun/sun4u-ss5/) · [National Super I/O, §§2.5 and 6](https://ardent-tool.com/datasheets/NS_PC87332VLJ.pdf)

OBP starts its first receive timeout by writing zero to the APB tick
register at physical `0x01fe_0100_08b0` (ASI `0x1d`, PCI 00:01.0,
configuration offset `0xb0`). Three receive waits each reach `0x03f941f8`
ticks before returning. The guest then prints `Probing keyboard Done`.
To reproduce this on serial A, run the example above with its cold diagnostic
seed and an instruction budget of `30725175`.

The fixture uses 100 ns of virtual time per retired instruction and the
66 MHz primary PCI clock reported in the hardware device tree. The tick
register wraps at 32 bits and supports the documented word access plus
OBP's aligned eight-byte access with the count in the low dword.
The clock source and zero-clear behavior are inferred from the board frequency
and the ROM loop. Nonzero writes and accesses via the second bridge function
remain unverified. [Linux SPARC APB register definitions](https://github.com/torvalds/linux/blob/v6.12/arch/sparc/include/asm/apb.h) · [Themis APB register map, table 1-6](https://www.artisantg.com/info/Themis_USPIIe_cPCI_Software_Manual.pdf) · [Ultra 5 PCI clock and serial trace](https://rob.com/sun/sun4u-ss5/)

## Instruction and data TLB initialization

At flash offsets `0x780` and `0x788`, OBP clears the D-MMU and I-MMU Tag
Access registers. It then writes zero through ASIs `0x5d` and `0x55` for
each slot from 63 down to 0. Each directed store captures the matching
staged tag along with the entry data. Separate indexed data and tag ports
support readback. The original-ROM check starts with 128 populated entries
and verifies every tag and data word after the loop. Address translation,
automatic replacement and demap are still open. [Sun UltraSPARC-IIi User’s Manual, §§15.2 and 15.9.8–9](https://www.cs.tufts.edu/comp/150PAT/arch/sparc/UltraSPARC-IIi-user-805-0087-02.pdf)

To see the latest diagnostic on a cold start, use the serial example with a
budget of `30726251`. Serial A ends with `Probing Memory `. At this point OBP
has prepared complementary 64-bit test patterns and is about to save the
first xword at PA zero. The following two loads use offsets of 8 and 16 MiB.
The board still needs ASI `0x14` access to physical DRAM before OBP can
measure installed memory.

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
`0x01ff_f000_210c` / `0x01ff_f000_2110`. Integer BPcc evaluates its
condition using the encoded ICC or XCC bank. Both prediction hints have
the same functional result.

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

## Reach the first board data read

After restoring the saved window, OBP uses eleven SETHI, OR and SLLX
instructions to assemble an eight-byte comparison value and a physical
address. The final instruction in this sequence retires at offset `0x2140`.
Single-stepping the original image in Helios and QEMU 10.2.2 gives these
values immediately before the load.

| Register | Value | Use by OBP |
| --- | --- | --- |
| `%o0` | `0x736f_6674_2d72_7374` | ASCII `soft-rst`, the expected value for the subsequent comparison |
| `%o1` | `0x0000_01ff_0000_0000` | Upper address bits, then destination of the load |
| `%o2` | `0x0000_01ff_f100_0010` | Address of the eight-byte load |

The next word, `0xd2da82a0`, is `ldxa [%o2] 0x15, %o1`. The IIi defines
ASI `0x15` as non-cacheable physical access with side effects. The flat
V9 memory fixture can complete an aligned big-endian load from its backing
bytes. The Ultra 5 flash fixture has no data device at this address.
`BootFlash::step` reports the ASI, physical address and width, leaving
PC/nPC at `0xffff_ffff_f000_2144` / `0xffff_ffff_f000_2148` and preserving
`%o1`. Use `step_with_nvram` to execute that load against board data.
[UltraSPARC IIi User’s Manual, §15.10](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=268)

## Execute the NVRAM reset decision

Sun's Ultra 5/10 OBDiag device tree puts the EEPROM at `eeprom@14,0`.
PCIO configuration register `0x14` resets to PCI base `0xf1000000`,
with chip select 1 at offset zero. On the Ultra 5 PCI memory path this
locates chip offset `0x10` at physical `0x01ff_f100_0010`, the address
constructed independently by OBP. The service manual identifies the chip
as a 48T59. [Ultra 5/10 Product Notes, code example 2](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-4970-10/805-4970-10.pdf#page=16) · [PCIO reset BARs, table 36](https://www.artisantg.com/info/force_cpci_520_series_manual.pdf) · [Sun Ultra 5 Service Manual, table 9-1](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-7763-12/805-7763-12.pdf#page=171)

Pass an 8192-byte image to `ResetNvram::new`, then call
`flash.step_with_nvram(&mut cpu, &mut nvram)` for each step. The device
serves aligned big-endian reads in its SRAM region. The final 16 bytes
contain device registers and accesses there stop explicitly. The load
reads the supplied bytes at offsets `0x10..0x18` on every CPU reset.
The battery-backed SRAM has no fixed cold-boot marker value. [ST M48T59 data sheet, data retention and register map](https://media.digikey.com/pdf/Data%20Sheets/ST%20Microelectronics%20PDFS/M48T59%2CY_V.pdf)

| NVRAM bytes at offset `0x10` | CCR after CMP | Offset after BNE and its delay slot |
| --- | --- | --- |
| Eight zero bytes | `0x00` | `0x21c0` |
| ASCII `soft-rst` | `0x44` | `0x2154` |
| Four zero bytes followed by ASCII `-rst` | `0x04` | `0x21c0` |

In the third case ICC reports equality for the low 32 bits. OBP selects
XCC and branches on the full 64-bit value. With a matching marker it
checks a second value in `%g7`. The fixture starts that register at zero.
The zero-filled and matching-marker traces reach offset `0x15d4` after
22 and 31 instructions respectively. The next word, `0x87802058`,
writes `0x58` to `%asi` to begin MMU context initialization.

QEMU 10.2.2 independently reproduced all three comparisons. For that
CPU check, the debugger redirected `%o2` to seeded guest RAM for the
load. Helios executed the same words against the board address above.

## Clear the primary and secondary MMU contexts

At offset `0x15d4`, OBP sets the eight-bit address-space identifier register
to `0x58`. The two `stxa` instructions use that ASI with immediate offsets.
They clear separate 13-bit context IDs shared by the instruction and data
MMUs. Each store is followed by `membar #Sync`. The fixture completes each
data access synchronously. MMU translation is disabled throughout this run.
[UltraSPARC IIi User’s Manual, §15.9.3](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=257) · [SPARC V9, §5.1.8](https://icps.u-strasbg.fr/~loechner/enseignement/SPARC/v9.pdf#page=74)

| Flash offset | Instruction | Effect |
| --- | --- | --- |
| `0x15d4` | `wr %g0, 0x58, %asi` | Select the D-MMU register space |
| `0x15d8`, `0x15dc` | `stxa %g0, [8] %asi`, `membar #Sync` | Clear the primary context and order data accesses |
| `0x15e0`, `0x15e4` | `stxa %g0, [0x10] %asi`, `membar #Sync` | Clear the secondary context and order data accesses |
| `0x15e8` | `add %g0, %g0, %g7` | Clear `%g7` and advance to the PIL write |

Both traces end at PC/nPC `0xffff_ffff_f000_15ec` / `0xffff_ffff_f000_15f0`.
The zero-filled NVRAM run takes 28 instructions. The matching-marker run
with reset `%g7 = 0` takes 37. The next word is `wrpr 15, %pil`. A separate
guest program in Helios and QEMU seeded primary and secondary with `0x357`
and `0x6ac` and verified their independent readback and clearing.

The fixture initializes `%asi` and both context IDs to zero for reproducible
tests. User code may write `%asi`, and an access to a restricted ASI requires
privilege. Context stores with nonzero reserved bits stop explicitly.
Sun specifies additional synchronization before a new context is guaranteed
visible to instruction fetches. Translated fetches and TLB behavior remain
open. [UltraSPARC IIi context synchronization rules](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=257)

## Initialize interrupt and register-window state

OBP executes ten consecutive writes starting at flash offset `0x15ec`.

| Flash offsets | Registers written in order | Result |
| --- | --- | --- |
| `0x15ec–0x15f0` | PIL, PSTATE | PIL becomes 15. PSTATE changes from `0x35` to `0x05`, leaving privilege and alternate globals selected as the CPU exits RED_state. |
| `0x15f4–0x15f8` | TBA, SOFTINT | The trap base and software-interrupt bits are cleared. |
| `0x15fc–0x1604` | CLEANWIN, OTHERWIN, WSTATE | Set to 7, 0 and 0. |
| `0x1608–0x1610` | CWP, CANRESTORE, CANSAVE | Select window 0, with zero windows available to restore and six to save. |

After 38 instructions with zero-filled NVRAM, or 47 with `soft-rst` and
reset `%g7 = 0`, the next word is `0x8f902005` (`wrpr 5, %tl`).
QEMU 10.2.2 reached the same word on both paths using the NVRAM read
redirection described above. A separate seeded guest checked all ten
registers before and after the writes, including SOFTINT through ASR 22.

The fixture implements register readback and each of the four IIi global
register banks. A guest PSTATE write that enables interrupts, changes the
memory model, selects little-endian operation or requests 32-bit addressing
stops explicitly. Interrupt delivery, register-window traps and trap entry/return
remain open. The window counters, PIL and TBA start at zero for
repeatability where hardware reset values are unspecified.
[IIi software interrupts, table 11-9](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=163) · [IIi window registers, §14.2.1](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=223) · [IIi PSTATE and global banks, tables 14-12/13](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=235)

With the reconstructed image installed, run the seventeen local-ROM checks together.

```sh
cargo test --locked --offline --lib --test ultra5_first_fetch --test ultra5_first_instruction --test ultra5_reset_branch --test ultra5_reset_cwp --test ultra5_window_select --test ultra5_reset_probe --test ultra5_reset_contexts --test ultra5_reset_setup --test ultra5_first_serial_access --test ultra5_ecache --test ultra5_keyboard_timeout -- --ignored --nocapture
```
