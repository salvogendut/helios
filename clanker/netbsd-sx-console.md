# NetBSD 11.1 SX console

Developed on `fix/netbsd-sx-console`, based on merged Helios `34276c7`.
Heliacal source stays on `main` at `d81d21c`; its release build uses the
updated sibling core.

## Reproduction and cause

The user's named machine selects SS-20, one CPU, 256 MiB, native SX, fast
clocks, original OBP 2.25R and the installed NetBSD CHD at SCSI ID 3.
The private reproduction used copies of the disk and graphical NVRAM;
it did not change the live catalog or attach the source disk for writing.

The exact stop reproduced after 438,326,773 steps:

```text
PC=f002b7d8 nPC=f002b7dc SPARC instruction=d622400a
physical address=f80000020
CPU R11 / SX instruction=92421408
SX ROP_CONTROL=3c
Unsupported("SX select instruction")
```

The SPARC instruction is the driver's store into SX's instruction queue.
The data is ROP_L with A=R8, B=R8, destination R40, length three. NetBSD's
`cg14_invert` uses the operation to draw/erase its text cursor. The error
does not originate in SDL, Wayland or OpenGL.

Adding the missing ROP forms exposed a second problem: `SX_CONTROL_STATUS`
writes copied zero into MT. NetBSD's `sx_wait` then spun at `bus_space_read_4`
PC `f002e36c` waiting for an empty queue, despite every SX command having
completed synchronously.

## Changes and sources

- ROP_B and ROP_L share the staged ROP_M execution path. R2 chooses the
  high/low truth-table nibble per byte or word, starting at bit 31. ROP_M's
  per-bit behavior is preserved. Both inputs and the selector are observed
  before overlapping destinations change.
- Every source/destination range is checked before mutation. Byte vectors
  beyond eight registers stop explicitly: behavior beyond the 32-bit mask
  span is not established by this patch.
- MT is always set and BZ clear at observable synchronous queue boundaries;
  guest control writes cannot fabricate queued work. GO remains writable.

Sources: Sun *Pixel Processing in a Memory Controller*, section 4.1.7/table 1;
NetBSD `sxreg.h` 1.22, `cgfourteen.c` 1.102 and `sxvar.h` 1.5 at tree
`bc1ab70174b4c8f3c69457e0ab39d4cf3cf77147`. Those source files are unchanged
relative to their checkout revision. The pinned QEMU sun4m comparison only
installs an unimplemented SX window, so it is not an execution oracle.
Human sources and limits are in `docs/references.md` and `docs/graphics.md`.

## Validation

`tests/sx_rop.rs` adds seven tests: the captured command; high-bit-first byte
masks; all 32 byte selectors; destination aliasing R2; all 256 truth tables
in all three modes with overlapping vectors; range failures and doubleword
queue rollback; queue status across control writes, commands and resets.
Existing packed-outline/SX regressions remain green.

Full core validation: 658 unique default tests and 676 with CHD/perf-counters,
formatting, both Clippy configurations, all four demos and release CLI build.
Heliacal: 75 tests, formatting, Clippy, release rebuild and SDL smoke pass.
Its isolated SS-20/SX/256 MiB disk-free firmware smoke passes after 54,814,720
steps; this separate smoke is not an OS-boot claim.

The corrected private guest reaches graphical root login and accepts Sun
keyboard input. `uname -a` reports NetBSD 11.1 GENERIC; `seq 1 60` scrolls
the console and leaves readable rows 22..60 plus the shell prompt.
After two further lines, the retained rows 24..60 match at a 44-pixel shift
with zero differences across the full text width. The comparison uses the
centered text grid (Y=10, 22-pixel font rows); an initial crop from Y=0
included unscrolled border pixels, which are recorded separately.
Kernel, shell and persisted marker/I/O checksums match the installation:
`3544516170 5707996`, `1436774852 227568`, `296033067 21`, and
`1204609605 262144`. Its root filesystem reports clean. The original NetBSD
GENERIC kernel and disk content are used, without firmware/kernel patches
or disabled console acceleration. NetBSD X11, SMP and other graphics devices
are outside this checkpoint. Earlier timing/RTC and entropy setup limits
remain separate.

`HELIOS-NETBSD-SX-OK` is visible after the scrolling test. `sync` and `halt`
complete disk sync, filesystem unmount and swap shutdown, returning to the
graphical OBP prompt. The private harness then exits successfully.

Evidence is under ignored `local/netbsd-sx-2026-11-16/`: private harness,
before/after/fixed logs, framebuffer captures, kernel symbols, source hashes,
focused and full core/frontend check logs. Boot times include manual waits
and concurrent validation and are not performance measurements.

Cleanup retains the compact logs, lossless PNG evidence, hashes, NVRAM and
harness source. The disposable CHD, redundant captures and private runner
executables are removed; `cleanup.json` lists the removed paths and hashes.
The user's `local/disks/netbsd11-ss20-scsi3.chd` and saved installation NVRAM
remain preserved for testing.
