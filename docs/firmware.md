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
Next work is the SBus/IOMMU control block, then TLB diagnostic behavior as
indicated by the original firmware. Neither probe establishes full timer
mode-transition hardware behavior or firmware compatibility.

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
