# Firmware development baseline

Use **original Sun OBP firmware** for development and hardware-accuracy checks.
OpenBIOS is a separate compatibility test until it is proved working. Firmware
must not be patched to bypass missing hardware, and QEMU-specific interfaces
must not redefine the SS-5 model. A successful isolated probe is not a firmware
compatibility claim. Neither image below has reached a prompt in Helios.

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

With [bus faults and capture registers](fault-capture.md) implemented, the
unchanged original ROM completes **25373701 instructions**, with **two synchronous
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
implemented, so this is an `UnsupportedInstruction` host stop, with the CPU
error latch clear and CLI exit status 1. The next milestone is FPU register
state, transfers and trap behavior; POST completion and an OBP prompt remain
unproved.

Records under `local/probes/fault-capture-ss5/` retain configuration, CLI
console/stop, bounded library trace/source and matching empty serial transcripts.
The firmware hash is unchanged. The normal POST path no longer reaches the
previous warning-recovery MFSR stop; capture registers, asynchronous store errors
and module interrupts are validated separately by focused tests. This run also
executes cache diagnostic data/tag stores, without establishing working ordinary
cache hits/fills or real memory parity checking.

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
