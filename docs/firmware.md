# Firmware development baseline

Use **original Sun OBP firmware** for development and hardware-accuracy checks.
OpenBIOS is a separate compatibility test until it is proved working. Firmware
must not be patched to bypass missing hardware, and QEMU-specific interfaces
must not redefine the SS-5 model. A successful isolated probe is not a firmware
compatibility claim. Original OBP now reaches responsive serial and graphical
monitors, with keyboard-entered Forth commands on the TurboGX console.
OpenBIOS has not reached a prompt. The latest [graphical checkpoint](#graphical-obp-console)
and [serial/timer/Ethernet checkpoint](#timer-and-ethernet-bring-up) distinguish
this from full hardware compatibility or OS boot. Earlier checkpoints below
are retained as historical evidence.

The selected machine is an SS5/85: 85 MHz microSPARC-II, 128 MiB RAM, CGsix,
SCSI disk/CD-ROM, Ethernet and serial. Pending devices remain absent; the
implemented subset and timing convention are documented separately.

## Original SS-5 OBP

The user supplied [andarazoroflove/sparc](https://github.com/andarazoroflove/sparc)
as the firmware archive. The selected file is
[ss5.bin at a6f20b0740cfb4440d36ac04d7ad67e0207a73d4](https://github.com/andarazoroflove/sparc/blob/a6f20b0740cfb4440d36ac04d7ad67e0207a73d4/ss5.bin).
The archive also contains a separate `ss5-170.bin`; the selected microSPARC-II
profile uses `ss5.bin`. Its internal machine name identifies SPARCstation 5.
The archive is a third-party source, not a manufacturer authenticity certificate.

| Property | Value |
| --- | --- |
| Image | Raw big-endian PROM, 262144 bytes (256 KiB) |
| Git blob | `98aa7fef3c66b720081950b09e7609e9b178213a` |
| SHA-256 | `e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e` |
| Local file | `local/firmware/sun-ss5.bin` |

```sh
python3 tools/fetch_firmware.py sun-ss5
cargo run --locked --offline -- boot local/firmware/sun-ss5.bin --steps 10000 --trace
```

The fetch helper verifies both the pinned SHA-256 and Git blob hash before
writing. It records provenance in a neighboring JSON file. Existing differing
files are refused; existing identical files can be reused. For an already
downloaded copy, pass `--source FILE`. No firmware is committed to this repository.

### Historical bring-up checkpoints

Initial probe, 2026-11-06, default 128 MiB, CPU/MMU reset, default NVRAM,
no serial input, 10000-step maximum, deterministic 100 ns successful-step quantum:

- Four instructions complete; there is no console output.
- PC=`0x0000bda0`, nPC=`0x0000bda4`, instruction=`0xc2804400`.
- The instruction reads physical `0x71f00000` through ASI `0x20`.
- That system reset/status register is unmapped. The data-access error with
  ET=0 latches CPU error state, and the CLI exits with status 1.

After reset/status and serial-initialization work, a second probe on 2026-11-06
with the same configuration produces:

- 304 completed instructions, zero traps, interrupts or software resets, and
  zero console bytes.
- PC=`0x0000bbb4`, nPC=`0x0000bbb8`, instruction=`0xd2a20400`.
- OBP writes `1` through ASI `0x20` to the system timer configuration register
  at physical `0x71d10010`, selecting the 64-bit processor user timer.
- This mode remains unsupported. The CLI exits with status 1 as an emulator
  stop, without a guest trap, CPU error latch or timer-configuration change.

The reset register and both SCC instances let OBP pass the earlier reset
and serial initialization accesses. WR14 DPLL source configuration is accepted
without enabling DPLL operation. The next hardware work is the processor's
64-bit timer mode at this checkpoint. Local records are retained under
`local/probes/reset-ss5/`; the initial records remain separate.

The next checkpoint implements that timer mode. The same 10000-step probe now
reaches its budget in the keyboard polling loop, with 10000 instructions, no
traps/interrupts/resets and no console output. The user timer advances while
Run=1; OBP samples it with LDDA. A longer run exposes subsequent hardware gaps:

```sh
cargo run --locked --offline -- boot local/firmware/sun-ss5.bin --steps 30000000
```

- At step 20000419, PC=`0x70001004`, nPC=`0x70001008`, instruction=`0xc0a04400`,
  OBP stores zero through ASI `0x20` to physical `0x10002000`. This unmapped
  microSPARC-II MID/SBus arbitration-enable register causes guest trap `0x29`.
  It is the **first missing hardware access**, before the final emulator stop.
- OBP prints a synchronous-trap diagnostic on TTYA, then requests a software
  reset at step 20035524. This is guest error handling, not a firmware prompt.
- The restarted path stops at step 20035688, PC=`0x0000be7c`, nPC=`0x0000be74`,
  instruction=`0xc0a200c0`: unsupported ASI `0x06` word store. microSPARC-II
  uses this space for TLB diagnostics. The CPU error latch is clear.
- Totals before the stop: 20035686 completed instructions (including the reset
  store), one synchronous trap, zero interrupts and one software reset.
  The CLI exits with status 1.

Records under `local/probes/user-timer-ss5/` retain the short trace, long-run
console and stop, and a bounded library probe with its source. That probe records
the first 500 steps, timer samples every million steps, exceptional outcomes and
128-step fault context, avoiding a multi-million-line trace. Its console matches
the CLI run. The firmware, RAM, default NVRAM and step quantum are unchanged.
This identified the SBus/IOMMU control block as the next work. Neither probe establishes full timer
mode-transition hardware behavior or firmware compatibility.

After adding SBus arbitration and the I/O configuration subset, the same
30000000-step probe completes **20004464 instructions**, with zero traps,
interrupts or resets and no console output. The previous write to `0x10002000`
now succeeds. At step 20004465, execution stops at PC=`0x7000ac40`,
nPC=`0x7000ac44`, instruction=`0xc0a401c0`: an unsupported ASI `0x0e` word
store during data-cache tag initialization. This is an emulator stop, not CPU
error state; the CLI exits with status 1.

That checkpoint identified the microSPARC-II cache diagnostic spaces, starting with the
data-cache tag access on this normal initialization path. The ASI `0x06` TLB
diagnostic limitation observed after the earlier guest reset remains a separate
gap. Records under `local/probes/io-control-ss5/` retain configuration, CLI
console/stop and the bounded library probe source/log. Firmware and machine
settings are unchanged. This is still not a successful firmware boot.

After adding cache diagnostic storage and indexed invalidation, the same
30000000-step probe completes **20044378 instructions**, one synchronous trap,
zero interrupts and one guest software reset. OBP performs 512 word stores to
each of ASIs `0c` and `0e`, completing both cache-tag initialization loops.
The probe counts no diagnostic data word stores, and PCR remains
`0x04004000` at the first fault (boot mode, translation and caches disabled).
The diagnostic data paths are covered separately by synthetic tests.

The first fault is step **20009111**, PC=`0x700075f8`, nPC=`0x700075fc`,
instruction=`0xe6a40400`: a physical-bypass word store of `0x02000000` to
`0x08000000`, exactly beyond the configured 128 MiB. The bus reports Unmapped,
and CPU trap `0x29` enters vector `0x70000290`. This occurs in OBP's memory
probing loop. The next investigation is the SS-5 bank map and unpopulated-memory
error behavior; this result does not justify increasing RAM or suppressing faults.

OBP emits 154 serial bytes reporting a synchronous trap and resets at step
20044216. The final stop is attempt **20044380**, PC=`0x0000be7c`,
nPC=`0x0000be74`, instruction=`0xc0a200c0`: unsupported ASI `06` word store
on the reset path. The CPU error latch remains clear; the CLI exits 1.
The TLB diagnostic limitation is still separate from the initial memory fault.

Records under `local/probes/cache-ss5/` retain the configuration, CLI console/stop,
bounded library trace/source, pre-fault registers and diagnostic store counts.
CLI and library console output match exactly. Firmware, RAM, NVRAM and step
quantum are unchanged. This establishes progress beyond tag initialization,
not a firmware prompt, working cache POST or OS compatibility.

At the DRAM/TLB milestone, with the SS-5 bank model and live diagnostic entries,
the same 30000000-step probe completes **20543176 instructions** with zero
traps, interrupts or resets and no serial output. `boot` and the library probe
now use `Machine::ss5`: four 32 MiB modules in slots 0–3, four empty slots.
Physical memory beyond 128 MiB completes as empty-bank accesses, so OBP passes
the previous memory-probe fault. The synthetic `sun4m` harness retains its old
strict contiguous mapping for small tests; it is no longer used for this probe.

The firmware also passes its accesses to TRCR (`ASI 04`, address `1000`) and
TLB diagnostic data/tags (`ASI 06`). The next stop is attempt **20543177**,
PC=`0x70005b68`, nPC=`0x70005b6c`, instruction=`0xe6a58400`: an ASI `20`
store of the test pattern `0xa5a5a5a5` to physical `0x10000000`. This attempts
to set IOCR.EN during an IOMMU register test. At that milestone DVMA translation
was not implemented, so the device rejected the write before mutation. The CPU error
latch remains clear and the CLI exits 1. This does not demonstrate any DMA
transfer or working firmware prompt.

Records under `local/probes/dram-tlb-ss5/` retain configuration, CLI console/stop
and bounded library trace/source. Console outputs match (both empty). The CLI
stderr record also contains the newly embedded `sun_ascii.txt` startup logo;
it is host output, separate from the guest serial transcript. The firmware
hash, 128 MiB capacity, default NVRAM, no input and 100 ns step quantum are unchanged.
See [DRAM and TLB diagnostics](dram-tlb.md) for functional limits, including
parity, fill-driven replacement timing, injected PTP walks and M-bit rewalks.

At the IOMMU translation/shared I/O TLB flush milestone, the unchanged
original ROM completed **21020690 instructions**, with **one synchronous
trap**, **zero interrupts**, **one software reset** and **125 serial bytes**.
The same 30000000-step budget, four 32 MiB DSIMMs, default NVRAM, no serial
input and 100 ns step quantum apply. No DMA device engine is running in this
probe; translation is separately exercised by library tests.

The first fault is attempt **20588909**, PC=`0x700063d0`, nPC=`0x700063d4`,
instruction=`0xc0850400`: an ASI `20` word load from unmapped SBus physical
address `0x7cc00000`, taken as data-access error trap `0x29`. OBP emits:

```text
ERROR  : Address= 00000300, exp= 00000816, obs= 00000016, xor= 00000800
UNUMBER: U0103
WARNING: Suspect Swift Module
```

The `0x800` difference is the SFSR SBus timeout bit (manual figure 5.13).
The preceding unmapped SBus read and current generic bus-error recording identify
timeout classification/capture as the next fault-status gap. This is a firmware
self-test warning, not a report of a failed physical module on the host.
OBP requests a software reset on attempt **20617884** and continues.

The final host stop is attempt **21020692**, PC=`0xffd27690`, nPC=`0xffd27694`,
instruction=`0xc8840400`: an ASI `20` word load from physical `0x10001050`
(MFSR), then an unsupported memory-fault capture register. The CPU
error latch is clear and the CLI exits 1. There is no OBP prompt or successful
POST/OS-boot claim. This identified SBus timeout status and memory/asynchronous
fault capture as the next work, preserving CPU/device DMA error separation.

Records under `local/probes/iommu-ss5/` retain configuration, CLI console/stop,
bounded library trace/source and the matching serial transcript. The firmware
hash is unchanged. [I/O controls](io-control.md) describes the implemented DMA
interface and remaining hardware limits.

At the [bus faults and capture registers](fault-capture.md) milestone, the
unchanged original ROM completed **25373701 instructions**, with **two synchronous
traps**, **zero interrupts**, **zero software resets** and **zero serial bytes**.
The 30000000-step budget, four 32 MiB DSIMMs, default NVRAM, no serial input and
100 ns step quantum are unchanged. The earlier `Suspect Swift Module` warning
and recovery reset no longer occur.

Both traps are diagnostic data-access errors (`0x29`):

| Attempt | PC / instruction | Physical access | Recorded cause |
| --- | --- | --- | --- |
| 20588909 | `0x700063d0` / `0xc0850400` | ASI `20` word load from `0x7cc00000` | SBus timeout, SFSR.TO |
| 20588995 | `0x700062b4` / `0xc0840400` | ASI `20` word load from `0x71400300` | NCR byte-only floppy aperture, SFSR.BE |

The final host stop is attempt **25373704**, PC=`0x70002454`, nPC=`0x70002458`,
instruction=`0xc11c0011`: `LDDF [%l0 + %l1], %f0` (register pair `%f0/%f1`).
Here `%l0=0x2000`, `%l1=0`, PCR=`0x04004000` and PSR=`0x044010e6` (EF set).
The effective and physical address is `0x2000`. FPU instructions are not yet
implemented at that checkpoint, so this was an `UnsupportedInstruction` host
stop, with the CPU error latch clear and CLI exit status 1. It identified FPU
register state, transfers and trap behavior as the next milestone. POST completion
and an OBP prompt remain unproved.

Records under `local/probes/fault-capture-ss5/` retain configuration, CLI
console/stop, bounded library trace/source and matching empty serial transcripts.
The firmware hash is unchanged. The normal POST path no longer reaches the
previous warning-recovery MFSR stop; capture registers, asynchronous store errors
and module interrupts are validated separately by focused tests. This run also
executes cache diagnostic data/tag stores, without establishing working ordinary
cache hits/fills or real memory parity checking.

At the [FPU state and transfers](fpu.md) milestone, the same original ROM
completed **25374200 instructions**, with **two synchronous traps**, **zero
interrupts**, **zero software resets** and **zero serial bytes**. The firmware
hash and all machine/probe settings above are unchanged. The two traps are the
same intentional bus-error diagnostics, at attempts 20588909 and 20588995.

This run completes **66 LDDF loads and 66 STDF stores**, advancing past the
earlier `LDDF` stop. Attempt **25374203** stops at PC=`0x70002728`,
nPC=`0x7000272c`, instruction=`0x81a80a22`: `FCMPs %f0, %f2`.
Both operands contain `0x01234567`; `%f1` and `%f3` contain `0xfedcba98`.
PCR=`0x04004000`, PSR=`0x044010e6` (EF set) and FSR=`0x00080000`.
Comparison execution was still unsupported; it did not update FCC, generate
a floating-point exception or latch CPU error. The CLI exited 1. This identified
IEEE comparisons and microSPARC-II deferred-exception state/queue behavior as
the next work.

Records under `local/probes/fpu-transfers-ss5/` retain configuration, CLI
console/stop and bounded library trace/source, including FPU registers and
instruction counts. The empty serial transcripts match. FSR transfers, moves,
branches and FPU traps are exercised by focused tests; this ROM run reaches the
comparison before executing those operations. Neither a completed POST nor a
firmware prompt has been demonstrated.

At the [comparisons and deferred exceptions](fpu.md) milestone, the unchanged ROM
completed **25374278 instructions**, with the same **two diagnostic bus traps**,
**zero interrupts**, **zero resets** and **zero serial bytes**. The firmware hash,
128 MiB configuration, default NVRAM, no input, 100 ns step quantum and
30000000-step budget remain unchanged.

The run completes two FCMPs, two FBfcc branches and eight FMOVs, in addition to
66 LDDF, 66 STDF, 12 LDF and one STF. Attempt **25374281** stops at
PC=`0x70002860`, nPC=`0x70002864`, instruction=`0x91a189a7`:
`FDIVs %f6, %f7, %f8`. `%f6=0x42c80000` (100.0), `%f7=0x40800000` (4.0)
and `%f8=0`. PCR=`0x04004000`, PSR=`0x044010e6` and FSR=`0x00080000`.
The FPU is in execution state with no deferred entry. Division is an
`UnsupportedInstruction` host stop, preserving the destination and leaving the
CPU error latch clear; the CLI exits 1.

Records under `local/probes/fpu-compare-ss5/` retain configuration, CLI
console/stop and bounded library trace/source, including FPU state and instruction
counts. The empty serial transcripts match. This run exercises successful
comparisons; deferred invalid exceptions and queue recovery are validated by
focused tests. This checkpoint identified division as the next FPU work.

At the [single/double division](fpu.md) milestone, the same original ROM completed
**25374279 instructions**, with **two diagnostic bus traps**, **zero interrupts**,
**zero resets** and **zero serial bytes**. The hash, 128 MiB machine, default NVRAM,
no serial input, 100 ns quantum and 30000000-step budget remain unchanged.

The FDIVs at `0x70002860` completes `100.0 / 4.0`, writing `%f8=0x41c80000`
(25.0) and leaving FSR=`0x00080000`. Attempt **25374282** stops at
PC=`0x70002864`, nPC=`0x70002868`, instruction=`0xa9a48833`:
`FADDs %f18, %f19, %f20`. `%f18=0x41a00000` (20.0), `%f19=0x40a00000`
(5.0) and `%f20=0`. PCR=`0x04004000`, PSR=`0x044010e6`; the FPU remains in
execution state with no deferred entry. The unsupported addition leaves its
destination and CPU error latch unchanged; the CLI exits 1.

`local/probes/fpu-division-ss5/` retains configuration, source, bounded library
trace, CLI stop and both matching empty serial transcripts. Instruction counts
are the previous checkpoint's counts plus one FDIVs. The run exercises an exact
single-precision result; double division, rounding and deferred exceptions are
covered by tests and the independent exact-fraction oracle (48000 guest cases,
seed `0x535335`). This checkpoint identified addition/subtraction as the next work.

At the [single/double addition and subtraction](fpu.md) milestone, original OBP
completed **25374281 instructions**, with the same **two diagnostic bus traps**,
**zero interrupts**, **zero resets** and **zero serial bytes**. The firmware hash,
128 MiB layout, default NVRAM, no input, 100 ns quantum and 30000000-step budget
are unchanged.

The FADDs at `0x70002864` computes `20.0 + 5.0`, leaving `%f20=0x41c80000`
(25.0) and FSR=`0x00080000`. A following LDF completes. Attempt **25374284**
stops at PC=`0x7000286c`, nPC=`0x70002870`, instruction=`0x9da3092d`:
`FMULs %f12, %f13, %f14`. Both source registers contain `0x40a00000` (5.0),
while `%f14=0`. PCR=`0x04004000`, PSR=`0x044010e6`; the FPU remains in
execution state with no deferred entry. Multiplication is an unsupported host
stop with CPU error clear; the CLI exits 1.

`local/probes/fpu-addition-ss5/` retains configuration, source, bounded library
trace, CLI stop and matching empty serial transcripts. FP counts are the prior
checkpoint plus one FADDs and one LDF. This firmware run has not exercised
subtraction or double arithmetic; focused regressions and the independent
fraction oracle cover them. Seed `0x535335`, with 2048 sampled operand pairs
per precision/operation plus boundary cases, checks 48000 divisions and 55168
each of addition and subtraction (158336 total). This identified multiplication
as the next observed blocker.

With [single/double multiplication](fpu.md) implemented, the unchanged original
ROM reaches the **30000000-step budget** at PC=`0xffd2f554`, nPC=`0xffd2f558`:
**29999968 instructions**, **16 synchronous traps**, **16 interrupts**, and
**one software reset**. Resetting store instructions are included in the instruction
count. The run uses the same 128 MiB layout, default NVRAM, no input and 100 ns
step quantum. It produces 71 serial bytes:

```text
ERROR  : No interrupt received, expected 0000001e
UNUMBER: U1507
```

The run now exercises both precisions of add/subtract/multiply/divide, comparison
and branches, including 12 deferred FPU exceptions and queue stores. The other
four synchronous traps are the two earlier diagnostic bus faults and two
misaligned FP transfers. It takes software interrupt levels 1–15, but the later
processor-timer diagnostic times out waiting for level 14 (TT=`0x1e`). After the
reported failure the firmware resets; one further level-14 interrupt occurs in
initialization. This is progress beyond the FPU stops, not successful POST.

`local/probes/fpu-multiplication-ss5/` records configuration, source, bounded
library trace, CLI stop and matching serial transcripts. The independent
arithmetic oracle passes 208464 guest cases, including 50128 multiplications
(seed `0x535335`, 2048 sampled operand pairs per precision/operation).

A separate **100000000-step-budget** CLI probe continues beyond that checkpoint,
prints configuration-checksum/default-NVRAM messages, then stops on
`UnsupportedMemory` at PC=`0xffd27690`, nPC=`0xffd27694`, word=`0xc8840400`.
The access is an ASI `0x20` read of the processor breakpoint register at
`0x10003018`. The CLI exits 1; its logs are retained alongside the 30M evidence.
Processor-timer timing and breakpoint-register behavior are the next blockers.
No firmware prompt has been demonstrated.

With the chip revision and virtual-address compare registers implemented, a
**200000000-step** run gets past the read at `0x10003018` and prints
`Probing CPU FMI,MB86904` on TTYA. It reaches the budget at PC=`0xffd16298`,
nPC=`0xffd162a0`, with **199998745 instructions, 17 synchronous traps,
1238 interrupts and one software reset**. The earlier processor-timer warning
and NVRAM checksum/default messages remain. This is ongoing firmware execution,
not a prompt or successful POST. Records are in `local/probes/chip-revision-ss5/`.

With both NCR auxiliary ports implemented, the caught byte-access faults at
`0x71900000` and `0x71910000` are gone. A 100-million-step CLI budget instead
stops at the floppy controller's DOR, physical `0x71400002`, before the budget
is exhausted. A library probe identifies attempt **80936336**, PC=`0xffd16898`,
nPC=`0xffd1689c`, instruction=`0xe0292000` (STB). This is an explicit unsupported
floppy access, with the CPU error latch clear. Serial output remains 225 bytes
through `Probing CPU FMI,MB86904`; no prompt yet. Records are under
`local/probes/auxiliary-ss5/`. This checkpoint supersedes the apparent prolonged
Forth initialization seen when the auxiliary ports were absent.

## Serial `ok` prompt

The unmodified original SS-5 ROM now reaches `ok` on SCC channel A (TTYA).
The probe uses `Machine::ss5`, 128 MiB RAM, default NVRAM, no initial input,
and the unchanged functional 100 ns step quantum. Firmware SHA-256 remains
`e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e`.
The final change adds the onboard NCR82077 initialization/positioning subset
with no attached floppy drive; hardware boot code has not been patched.

| Observation | First prompt | After Forth command |
| --- | ---: | ---: |
| Step | 93481624 | 93671422 |
| Completed instructions, including reset store | 93481339 | 93671135 |
| Synchronous traps | 17 | 17 |
| Interrupt entries | 268 | 270 |
| Software resets | 1 | 1 |
| Serial bytes | 344 | 360 |

Both observations have PC=`0xffd27424`, nPC=`0xffd27428`, with no CPU error
latch. After the first prompt, the library sends `1 2 + .` followed by carriage
return through `Serial::receive`, respecting receiver enable and FIFO capacity.
OBP echoes the command, prints `3`, and returns to `ok`:

```text
Data Access Error
Type  help  for more information
ok 1 2 + .
3
ok
```

**This is an error-recovery monitor prompt, not a completed normal boot.**
The earlier processor-timer diagnostic still reports `expected 0000001e`,
followed by a guest reset and configuration-checksum/default-NVRAM messages.
Later serial output announces SBus probing with `espdma` and `esp` labels.
At step 93294238, an ASI `0x20` byte read from physical `0x6e000000` raises
SBus timeout/data-access trap `0x29`: PC=`0xffd265f8`, nPC=`0xffd266ac`,
instruction=`0xc88c0400`. OBP handles it by entering the monitor. This trace
alone does not identify the complete cause of that probe failure. SCSI/DMA,
Ethernet, CGsix, completed POST and operating-system boot remain unfinished.

```sh
cargo run --release --locked --offline -- boot local/firmware/sun-ss5.bin --steps 100000000
```

`local/probes/obp-ok-ss5/` retains firmware/configuration metadata, bounded
library probe source and log, raw serial transcript, CLI output and a separate
NVRAM export. The proof run stops after the second prompt. No firmware or NVRAM
image is committed. This was the first requested stopping checkpoint; subsequent work continues below.

## AFX presence and the next MACIO access

On resumed bring-up, tracing identifies the `0x6e000000` read as OBP's named
`afx-slot-empty?` routine, not an ESP register. The board now reports an empty
AFX slot for the SBus CGsix configuration. OBP passes that check and reaches
physical `0x78400000`, the onboard MACIO SCSI DMA control/status register.
That missing word read raises trap `0x29` at step **93375626**, PC=`0xffd16718`,
nPC=`0xffef0000`, instruction=`0xc8012000`. The serial recovery transcript
still contains a `Data Access Error`; the fault has moved to the next device.
The 200-million-step CLI remains at its prompt loop; no clean boot is claimed.
Evidence is under `local/probes/afx-presence-ss5/`.

The subsequent DMA2 register implementation moves the failure to the actual
SCSI controller: step **93375980** takes CPU level 15 after a byte write to the
ESP command register at physical **0x7880000c**. The I/O controller captures
AFSR `0xa3820000`, AFAR `0x7880000c`. OBP reports `Level 15 Interrupt` and enters
its recovery monitor. This distinguishes successful DMA register probing from
an implemented SCSI controller or disk. Local records are under
`local/probes/macio-dma-ss5/`.

## ESP initialization and full RAM test

With the integrated NCR53C9X initialization subset, the unmodified original ROM
finishes creating onboard device nodes, correctly handles four empty SBus-slot
probes, and prints its normal machine banner:

```text
SPARCstation 5, No Keyboard
ROM Rev. 2.15, 128 MB memory installed, Serial #1.
Ethernet address 2:0:0:0:0:1, Host ID: 80000001.
```

The default NVRAM run then completes OBP's 128 MiB memory diagnostic without a
reported RAM error. It attempts its default diagnostic network boot:

```text
Boot device: /iommu/sbus/ledma@5,8400010/le@5,8c00000  File and args:
Data Access Error
Type  help  for more information
ok 1 2 + .
3
ok
```

The final data-access trap is at step **1009127039**, PC=`0xffd16718`, a word
read of physical **0x78400010**, the unimplemented Ethernet DMA CSR. The
`sd`, `st`, `le` and other printed node names alone do not establish working I/O.
The four empty-slot timeouts earlier in enumeration are handled by OBP and print
`Nothing there`; they do not abort device probing.

Using the same original-ROM SHA-256, default NVRAM, four 32 MiB DSIMMs, and
100 ns functional step time as the earlier checkpoint, a bounded library run
reaches its first prompt at **1009351805** steps. It sends `1 2 + .` through
TTYA only after that prompt, respecting receive FIFO capacity. The reply is
`1 2 + .\r\n3 \r\nok `, with the second prompt at **1009623777** steps:

- 1009614328 instructions, including the reset-requesting store;
- 21 synchronous traps, 9428 interrupt entries and one software reset;
- 3922 output bytes; PC=`0xffd1689c`, nPC=`0xffd168a0`, CPU error latch clear.

Local evidence is under `local/probes/esp-ram-test-ss5/`: probe source,
configuration metadata, raw serial output, trace/counts and a separate NVRAM
export. The probe stops at the verified second prompt. A plain CLI run needs a
larger budget than the early recovery checkpoint, for example:

```sh
cargo run --release --locked --offline -- boot local/firmware/sun-ss5.bin --steps 1100000000
```

The earlier timer POST warning (`expected 0000001e`) still occurs. This is a
completed device-enumeration/RAM-test checkpoint followed by recovery from an
unsupported network boot, not clean POST or working storage/networking. CGsix,
SCSI targets/data DMA and Ethernet remain unfinished.

The service manual's
[POST and serial diagnostics](https://docs.oracle.com/cd/E19127-01/sparc5.ws/801-6396-11/801-6396-11.pdf)
provide the bring-up workflow; register semantics still require chip-level
documentation. Future probes should retain hashes, configuration, trace,
console transcript and exact stopping condition.

## QEMU OpenBIOS comparison image

QEMU's pinned revision includes
[pc-bios/openbios-sparc32](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/pc-bios/openbios-sparc32).
Its [firmware README](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/pc-bios/README)
identifies OpenBIOS as GPL v2 replacement firmware built from SVN revision 1280.
The inspected MAME Sun driver has no SS-5 ROM entry; SS-10/SS-20 ROMs are not
substitutes for this target.

| Property | Value |
| --- | --- |
| Original format | ELF32 big-endian SPARC executable, 382048 bytes |
| ELF SHA-256 | `bf6e231eeea72b17f97055a20de1195005cf79cbd6c118b7ca89287b4a77fd1f` |
| Entry / load address | `0xffd00000` |
| Load segment | File offset `0x58`, file size `0x5d2ac`, memory size `0xcf000` |
| Prepared raw size | 847872 bytes, including zero-filled BSS |
| Raw SHA-256 | `acbfbc9a418f702fa6490f325dab89a8149525459b662fa2311684fa32046dd6` |
| Local files | `local/firmware/openbios-sparc32.elf`, `.bin`, `.json` |

```sh
python3 tools/fetch_firmware.py openbios
cargo run --locked --offline -- boot local/firmware/openbios-sparc32.bin --steps 10000 --trace
```

The helper checks the exact known ELF, copies its loadable bytes and zero-fills
the remainder of its memory segment. QEMU's PROM loader maps the segment from
`0xffd00000` to physical `0x70000000`; the resulting raw image matches that
placement. This is a conversion for one pinned image, not a generic ELF loader.
The original ELF is preserved. The remaining Helios PROM aperture is padded by
the normal ROM loader.

Initial probe with the same machine settings completes four instructions and
stops at PC=`0x00005f0c`, nPC=`0x00005f10`, instruction=`0xc4b145a0`: an unsupported
ASI `0x2d` halfword store. In QEMU this is the `fw_cfg` selector at physical
`0xd00000510`, outside microSPARC-II's 31-bit hardware space. The stop is an
emulator limitation, not a CPU error state, and there is no console output.
Implementing that interface is deferred compatibility work; original OBP
hardware bring-up takes priority.

## Timer and Ethernet bring-up

The original, unmodified SS5 OBP image now passes its processor-timer diagnostic
and the Ethernet driver's internal loopback test. Physical PROM reads include
NCR89C105's six SBus clocks per byte at the SS5/85's 21.25 MHz SBus clock.
Counter overflow skips zero, zero-limit timers do not interrupt, and a limit
read returns its pending flag before acknowledging it. These rules follow
Sun-4M section 5.3.2; [interrupts-timers.md](interrupts-timers.md) records the
conflict with the NCR timer prose.

The user-selected fresh IDPROM now reports MAC `08:00:20:c0:ff:ee`, hostid
`80c0ffee` and serial 12648430, with manufacture date zero and checksum `a9`.
Configuration SRAM starts blank, so OBP still reports an initial configuration
checksum warning and establishes its own defaults. Imported NVRAM is unchanged.

The 2026-11-06 headless probe uses 128 MiB, the same pinned original ROM and
its unchanged SHA-256, a 2,000,000,000-step budget, and no ROM patch or diagnostic
bypass. It completes device enumeration and the full RAM test, initializes the
Ethernet controller through real IOMMU DMA, completes internal loopback, and
emits 64-byte RARP requests with EtherType `8035` and opcode 3. No host network
backend or server replies are attached. After the second request, the probe
asserts serial BREAK for 1,000,000 steps, releases it, and waits for `ok` before
typing `1 2 + .` and CR through TTYA's FIFO.

| Event | Completed step |
| --- | ---: |
| First RARP frame | 995183053 |
| Second RARP frame / BREAK asserted | 1057759536 |
| BREAK released | 1058759536 |
| First serial `ok` | 1058977191 |
| Forth result `3` and second `ok` | 1059249163 |

Final totals: 1,059,239,040 instructions, 21 synchronous traps, 10,102 interrupt
entries and one guest software reset; 4,036 TTYA bytes. PC=`ffd1689c`,
nPC=`ffd168a0`, no CPU error state and no host stop. The serial transcript ends:

```text
ok 1 2 + .
3
ok
```

This is a monitor return via the normal serial abort path, after real Ethernet
initialization and packet transmission. The timer warning and unmapped Ethernet
fault from earlier checkpoints are gone. Clean POST is still unfinished: at
step 8711303, PC=`700044f8` reads the unimplemented parallel-port DMA CSR
`7c800000`, producing a diagnostic SBus timeout. OBP recovers and continues.
That is the next POST hardware gap, separate from Ethernet packet behavior.

Ignored `local/probes/timer-ethernet-ss5/` contains the probe, exact configuration,
console/step logs, both RARP frames and exported NVRAM. Synthetic regressions
cover timer timing/flags, DMA errors, packet ordering/CRC, loopback, IDPROM and
serial BREAK. No OS, storage target, CGsix display or network boot is claimed.

## Graphical OBP console

The TurboGX console milestone adds the original card's FCode ROM alongside the
SS-5 PROM above. The selected ROM is `sunw,501-2325.bin` from MAME's `turbogx`
set, archived in [retrobios at ae7feba5398fb1258b381d2a0a1c7779f7630d5f](https://github.com/abdess/retrobios/blob/ae7feba5398fb1258b381d2a0a1c7779f7630d5f/bios/Arcade/Arcade/turbogx.zip).
Its CRC32 `bbdc45f8` and SHA-1
`e4a51d78e199cd57f2fcb9d45b25dfae2bd537e4` match the pinned MAME ROM declaration.
The 32768-byte raw image has SHA-256
`3fb9a8dbe7c8621dd9a65246ac78d9353a492c41862c1dd61656fb03d1a16259`.

```sh
python3 tools/fetch_firmware.py turbogx
```

The helper fetches the pinned ZIP, reads only the named member, checks its size
and SHA-256, and writes `local/firmware/turbogx.bin` with a provenance JSON file.
`--source` accepts an already-downloaded ZIP. Existing different files are never
replaced. The archive is third-party provenance, not a manufacturer certificate;
no firmware is committed.

With fresh default NVRAM, 128 MiB RAM, the 1 MiB card at physical `0x30000000`
and a Type 5 US keyboard, the original OBP 2.15 ROM:

1. Identifies the keyboard and discovers `cgsix` in SBus slot 1.
2. Runs the option ROM, draws the color Sun banner and reports 128 MiB RAM,
   MAC `8:0:20:c0:ff:ee` and host ID `80c0ffee`.
3. Completes its memory test and sends RARP requests during automatic network boot.
4. Accepts Sun Stop-A, returns to the on-screen `ok` prompt, and evaluates
   keyboard-entered `1 2 + .` as `3` before another `ok`.

The initial 128 MiB proof queued Stop-A after the second RARP frame at step
1034794122 and captured the arithmetic response at step 1048000000. It recorded
1047989913 instructions (including the software-reset store), 3 synchronous
guest traps, 10084 interrupt entries and one software reset, with no host error.
The traps came from firmware probing empty SBus slots/mappings. Keyboard
presence changes the early firmware diagnostic path; this run does not establish
that the previously observed serial-only parallel-port POST gap is fixed.

Follow-up probes exercise the bounded keyboard FIFO and use OBP's own `setenv`
commands to disable automatic boot and diagnostics in an exported NVRAM image.
The final 128 MiB run completed this sequence at step 1061371506, with 1061361284
ordinary instruction completions, 3 synchronous traps, 10218 interrupt entries
and one software reset. Its snapshot shows the arithmetic result and both
accepted settings followed by `ok`.
The native window can reload that image with 128 MiB RAM and display `ok`
directly. The default machine configuration is not changed by these test images.
At that milestone, live desktop interaction confirmed the storage boundary:
`probe-scsi-all` stops on an unsupported ESP selection/transfer command at
physical `0x7880000c`, PC `ffd16898`. The window preserves the image and now
names that stop reason in its title; this is not a completed SCSI probe.
Ignored `local/probes/graphics-ss5/` contains the local probe sources, logs,
snapshots and NVRAM exports. See [graphics.md](graphics.md) for invocation,
register behavior and limitations. The following checkpoint supersedes the
SCSI discovery limitation. OS boot, mouse input and complete graphics
acceleration remain unverified or unsupported.

## SCSI target discovery

With the same original OBP 2.15 and TurboGX ROMs, 128 MiB RAM and Type 5 keyboard,
`probe-scsi-all` now completes through ESP selection and DMA2/IOMMU transfers.
The final run used the graphical milestone's NVRAM export, configured by OBP's
own `setenv auto-boot? false` and `setenv diag-switch? false` commands. It attached
a blank 4 MiB raw disk at ID 3 and a blank 256 KiB CD data image at ID 6. These
are synthetic discovery fixtures, not bootable media.

Keyboard input entered `probe-scsi-all` after step 100000000. The captured
display at step 182392204 contains:

```text
ok probe-scsi-all
/iommu@0,10000000/sbus@0,10001000/espdma@5,8400000/esp@5,8800000
Target 3
  Unit 0 Disk     HELIOS  SCSI DISK       0001
Target 6
  Unit 0 Removable Read Only device    HELIOS  SCSI CD-ROM     0001
ok
```

The firmware ran unmodified; no target list or prompt was injected. Missing IDs
complete with selection timeout. The source images remain byte-for-byte blank;
the backend opens them read-only. The probe stopped at the requested target-list
milestone without attempting an OS boot. Block reads, residual counts and error
recovery have separate firmware-free regressions; this run proves discovery.

Ignored `local/probes/scsi-ss5/` holds `probe.rs`, `final-run.log`, the two fixture
images and `probe-scsi-all.png`. Both the CLI and graphical frontend accept
`--disk PATH` and `--cdrom PATH`; see [SCSI storage](scsi.md) for formats, IDs,
supported commands and remaining limitations.

## Solaris CD bootloader output

The next milestone tests the user's original Solaris media with the same
128 MiB SS5/85, original OBP 2.15, TurboGX ROM, Type 5 keyboard and previously
exported NVRAM (`auto-boot?` and `diag-switch?` false). Only target 6 is mounted,
read-only. Ignored `local/media/` holds symlinks to the source files in the
user's `Downloads/Solaris Media` directory; no media are committed.

| Image | Bytes | SHA-256 |
| --- | --- | --- |
| `solaris_2.6_598_sparc.iso` | 565862400 | `17351669cb91a89562ed6bb23345879c4d18bd019a85085ad53f777e7b7a6b14` |
| `SOL_7_1199_SPARC.iso` | 647147520 | `d1709cf47fe560f780af1769732a52eca706c8a1a52367136f8a8a14dd276cbf` |

Both images have Sun disk-label magic `dabe` and ISO volume descriptors.
Their volume IDs are `SOL_2_6_598_SPARC_SMCC_SVR` and `SOL_7_1199_SPARC`.
The source hashes match before and after the tests.

At step 60000000 the probe queues real keyboard events for `ttya output`, then
`boot cdrom -v`. Firmware selects
`/iommu/sbus/espdma@5,8400000/esp@5,8800000/sd@6,0:d`. Both runs execute their
first boot-block instruction at PC `00004000`, step 116142551. After spinner
characters, the first completed bootloader output lines are:

| Image | Serial line | Stop step |
| --- | --- | --- |
| Solaris 2.6 5/98 | `Size: 243560+176918+41926 Bytes` | 149954625 |
| Solaris 7 11/99 | `Size: 255664+158146+39070 Bytes` | 148968160 |

The stop condition is the first newline after non-whitespace serial output
following boot-block entry. No firmware output or boot data are injected.
Neither run attempts kernel startup beyond that point. This demonstrates
loading/executing CD boot code and its first verbose output, not a complete OS
boot or installation.

The traces include INQUIRY, START STOP UNIT, REQUEST SENSE, MODE SENSE(6) and
READ(6), with 32 logical blocks transferred as 65536 bytes through the IOMMU.
These firmware/bootloader paths use native 2048-byte logical blocks and do not
issue MODE SELECT. Separate regressions verify 512-byte mode selection and its
capacity/read mapping; it is available when requested by guest software.
The newly implemented START STOP UNIT eliminates the earlier unsupported-opcode
response, while reset UNIT ATTENTION remains a real guest-visible condition.

Ignored `local/probes/cdrom-ss5/serial-probe.rs` reproduces the runs. Its
`sol26-serial.log` / `sol7-serial.log` contain controller and step traces;
`sol26-serial-serial.log` / `sol7-serial-serial.log` contain raw TTYA bytes,
including backspace-driven spinners. Both runs stop at the requested milestone.

## Solaris kernel startup

Continued tests use the same original ROMs, NVRAM, 128 MiB machine and read-only
target-6 images above, with no hard disk attached. The probe sends `ttya io`
through keyboard events at step 60000000 and `boot cdrom -v` through TTYA at
step 70000000. Both console directions then use serial. Ignored
`local/probes/solaris-boot/probe.rs` records raw serial and ESP/DMA traces.

Both kernels print their release banners, enumerate the IOMMU/SBus/ESP and
report the CD's `sd@6,0:b` partition mounted as UFS root. The versions are
SunOS 5.6 `Generic_105181-05` and SunOS 5.7 `Generic_106541-08`. Startup then
initializes both SCCs, CGsix and Ethernet. The CD driver requires PREVENT ALLOW
MEDIUM REMOVAL; returning ILLEGAL REQUEST previously caused `vfs_mountroot`
to panic. Solaris 7's optional PERSISTENT RESERVE IN remains unsupported with
normal SCSI ILLEGAL REQUEST sense, which this kernel tolerates.

The SCC drivers issue CRC reset and enable receive/transmit/external interrupts.
Those asynchronous paths now work through the real level-12 controller sources.
Solaris 2.6 reads E_TST_CSR while initializing Ethernet; the DMA model now
exposes its address and empty-cache status. Solaris 7 also reads `0x78400020`;
honoring the DMA2 bit-5 alias removes the previous unmapped-access kernel panic.
These changes follow the hardware interfaces; neither ROMs nor guest code are
patched. See [devices.md](devices.md) and [ethernet.md](ethernet.md).

Both kernels continue into startup programs, print `The system is coming up.
Please wait.`, and reach the installer's `Select a Language` screen on TTYA.
The first runs stopped as soon as that heading appeared: Solaris 2.6 at step
1522202105 (`sol26-dma`), Solaris 7 at step 1686400176 (`sol7-alias`). No language
selection or installation command was sent. This establishes CD boot through
the first installer screen, not installation or an installed-system boot; the
later installation and disk-boot result is recorded below.

Final-code repeats continued for 10 million steps after the heading and until
at least one million steps of serial silence. Both ended with the complete
menu and its input prompt:

```text
Select a Language

 0) English
 1) German
 2) Spanish
 3) French
 4) Italian
 5) Swedish

?
```

| Image | Final run | Stop step |
| --- | --- | --- |
| Solaris 2.6 | `sol26-final` | 1532202106 |
| Solaris 7 | `sol7-final` | 1696400177 |

Neither run had a host stop or kernel panic. The raw `*-serial.log` captures
end at `?`; the probe supplied no menu input. Both ISO and firmware hashes
still match the originals. All 403 core tests, four UI tests, fmt/Clippy checks,
release builds and four demos pass with this implementation.

## Installed Solaris disk boot

The local Solaris 7 writable installation image boots from the original SS-5 OBP
with saved NVRAM and no CD attached. The run checks and mounts the UFS root on
`sd@3,0`, configures `/dev` and `/devices`, attaches `cgsix`, `le`, the SBus
memory nodes and `audiocs`, completes first-boot identification, and reaches
`helios console login:`. A `root` login on `/dev/console`, with the root password
left empty during local verification, reaches a shell prompt. Audio I/O is not
verified. This image's `/etc/vfstab` references `c0t3d0`, so keep the disk at
SCSI ID 3; moving it changes the guest device names and breaks the installed
root/usr mounts.

The separate Solaris 2.6 installation uses a new 2 GiB image at
`local/disks/solaris26-install.raw`. Its blank disk was labeled with `format`
before `suninstall`; the installer selected End User System Support, the
`c0t3d0` disk, automatic layout, and manual reboot. The installed disk also
boots from the original OBP without the CD, mounts its UFS root on `sd@3,0`,
completes first-boot setup, reaches a `root` shell under SunOS 5.6, and returns
to the `ok` prompt after `halt`. Raw captures are
`local/probes/solaris-install/sol26-install-serial.log` and
`local/probes/solaris-install/sol26-boot-serial.log`.

Two missing paths surfaced during the Solaris 7 boot. Solaris issues ESP
command `0xc3` (select with ATN and stop plus DMA) to transfer one IDENTIFY byte
before sending a tag; the ESP now reports the message-out stop and accepts the
following TRANSFER INFORMATION command. Solaris's `audiocs` probe also performs
byte accesses to the CS4231 aperture at `0x6c000000`; the indirect register
subset now responds there instead of returning an SBus timeout. Raw serial
captures are `local/probes/solaris-install/sol7-csbyte-serial.log` and
`local/probes/solaris-install/sol7-login-serial.log`. The earlier ASI 47 stop
does not occur with the SS-5 PROM; it reproduces exactly at `PC=0x00026328`
when `local/firmware/ss10-20-v2.25r-rom` is booted in the SS-5 harness. That is
SPARCstation 10/20 firmware targeting a different machine and is not supported
by the SS-5 profile.

## Solaris X server startup

The installed Solaris 2.6 disk can start the server-side desktop login on
`display :0`. The X CGsix driver exercises the TEC register page, FBC absolute
line/relative rectangle registers and `FSQRTd`; Helios retains the TEC page,
draws the FBC line and rectangle primitives, and implements binary32/64 square
root. Solaris's mouse device is Sun Mouse Systems on SCC channel B; the core
emits its five-byte packets at 1200 baud and Heliacal forwards captured relative
motion and buttons. A headless probe reached `Starting Desktop Login on
display :0...` and produced a nonblank framebuffer, but did not interact with
the login screen. See `clanker/x-windows.md` for the local evidence and limits.

The default profile remains SS5/85, but the functional 100 ns instruction
quantum makes these kernels measure the CPU as 10 MHz and report an SBus clock
warning. Normal CPU/cache timing is not yet an 85 MHz timing model. No
external networking or audio streaming is established by this evidence.

## SPARCstation 20 and Axil 320 profiles

The SS-20/Axil machine profiles are bring-up targets for their original firmware
under the high sun4m address map. They do not yet reach a stable `ok` prompt.
Images remain under ignored `local/`:

| Image | Size | SHA-256 |
| --- | ---: | --- |
| `local/firmware/ss10-20-v2.25r-rom` | 524288 | `b4f15bfafa69edfcea446778bd14b61b76273e4e77ddebe3a9002af757b48ca8` |
| `local/firmware/axil320-2-19.bin` | 524288 | `bc231c32d817d09e1d8192da0475a342f88b3ac2ed64e6fb67ab61e797e77425` |

With `--machine ss20`, the SMCC SPARCstation 10/20 POST identifies the CPU and
reports CPU 1–3 absent. The ICACHE_TLB bit-pattern, all documented flush tests, the D-TLB cached
level-2 PTP test and the `TLB_HIT`, `TLB_MISS` and `TLB_PROBE` tests pass. The SX, DBRI and BPP placeholder windows let SBus enumeration reach the
serial monitor. The integer and FPU endianness tests pass
after implementing PSR.DE dual byte ordering. The next full-POST diagnostic is
the CPU/MBus module test:

```text
           SMCC SPARCstation 10/20 UP/MP POST version VRV3.45 (09/11/95)

CPU_#0       TI, STP1021PGA(1.x)	1Mb External cache

CPU_#1       ******* NOT installed *******
...
<<< CPU_00000000 on MBus Slot_00000000 >>>
 ERROR : Address = 40000000,
 exp[31:00] = 00000000, obs[31:00] = 0000001e, xor[31:00] = 0000001e
 U-NUMBER : Suspect Viking Module
Incorrect configuration checksum;
Setting NVRAM parameters to default values.
Setting diag-switch? NVRAM parameter to true
```

The PTP2/TLB diagnostics use a functional model that fills ASI 6 SEL 5/6 from
the level-1 table, permits a matching 256 KiB PTP2 to override a 16 MiB leaf,
mirrors fills into ASI 5/6 storage and installs ASI diagnostic writes as real
translations. Split I/D TLB behaviour is not fully modeled.

The blank default NVRAM is rejected after POST, so OBP rewrites defaults, sets
`diag-switch?` true and resets instead of settling at `ok` in the tested step
budget. With a valid NVRAM image, Stop-A (`Pause+A`) skips the long memory test
and reaches an interactive serial `ok` prompt after the SBus probe list. The
console becomes usable after OBP itself runs:

```text
ok setenv output-device ttya
ok setenv input-device ttya
ok setenv auto-boot? false
ok setenv diag-switch? false
ok
```

The generated image is kept under ignored `local/ss20-ttya.nvram`. It boots to
`ok` without a host-side rewrite:

```text
./target/release/helios boot local/firmware/ss10-20-v2.25r-rom \
  --machine ss20 --keyboard --nvram local/ss20-ttya.nvram --steps 300000000
```

SX, DBRI and BPP placeholder windows prevent the previous `Data Access Error`
during SBus enumeration. With a keyboard present and `diag-switch? false`, OBP
uses its abbreviated startup path and reaches serial `ok` without the full POST
cache-tag error. A headless boot still runs full POST and reports the remaining
MBus cache-tag diagnostic (`Address = 0x4000_0000`); this is not a full-POST
pass claim.

With `--machine axil320 --keyboard --cgsix-rom local/firmware/turbogx.bin`, the
Axil 320 ROM finds a CG6 in SBus slot 2 (`cgsix@2,0`), switches the console to
it and draws the machine banner there. Without a CG6 the ROM prints
`screen not found.` and falls back to tty. The default Axil IDPROM identity is
MAC `00:00:3c:13:de:ad`, serial `c0:ff:ee`, host ID `72c0ffee` (IDPROM machine
type `0x72`); the user's physical machine banner matches this identity.

The generated `local/axil-ttya.nvram` has `input-device ttya`,
`output-device ttya`, `auto-boot? false` and `diag-switch? false`. It was
created from the Axil OBP's graphical console and now boots to a clean serial
`ok` without the configuration-checksum reset:

```text
./target/release/helios boot local/firmware/axil320-2-19.bin \
  --machine axil320 --keyboard --cgsix-rom local/firmware/turbogx.bin \
  --nvram local/axil-ttya.nvram --steps 300000000
```

With a disk attached, OBP's `probe-scsi-all` now lists it:

```text
ok probe-scsi-all
/iommu@f,e0000000/sbus@f,e0001000/espdma@f,400000/esp@f,800000
Target 3
  Unit 0   Disk     HELIOS  SCSI DISK       0001
ok
```

The remaining work is the `/obio/zs@0,0` device-tree/console path, a valid
SS-20 NVRAM image with a serial console, and then the SuperSPARC-II I/D-cache
tag diagnostic clear/half semantics; see [machine profiles](machines.md). No
full SuperSPARC CPU/MBus, SX/VSIMM, DBRI or BPP claim is made.
