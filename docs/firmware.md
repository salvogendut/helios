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

This identifies the next hardware block to implement. The service manual's
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
