# Firmware development baseline

Use **original Sun OBP firmware** for development and hardware-accuracy checks.
OpenBIOS is a separate compatibility test until it is proved working. Firmware
must not be patched to bypass missing hardware, and QEMU-specific interfaces
must not redefine the SS-5 model. A successful isolated probe is not a firmware
compatibility claim. Original OBP now reaches a responsive serial monitor after
an initialization error; OpenBIOS has not reached a prompt. The current
[serial checkpoint](#serial-ok-prompt) distinguishes this from successful POST
or OS boot. Earlier checkpoints below are retained as historical evidence.

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
image is committed. User-requested hardware bring-up stops at this checkpoint.

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
