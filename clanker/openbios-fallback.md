# OpenBIOS fallback — issue #8

Work branch: `feat/openbios-fallback` in Helios and Heliacal, from Helios
`7636eaa` / Heliacal `1f23c78`. Original OBP remains the accuracy baseline.

## Ownership and selection

`src/firmware.rs` and `src/firmware/image.rs` own the shared selection contract,
bounded ELF loader, firmware kind, fresh CHRP environment and opt-in fw_cfg
stream. An explicit PROM always wins, including errors. With no PROM, use
`HELIOS_OPENBIOS` or the documented search for `openbios-helios-sparc32.elf`.
No automatic download, bundled third-party firmware or live-config rewrite.
The CLI reports the chosen image; Heliacal also shows it in About.

Heliacal appends `.openbios` to the existing flat/named-machine NVRAM path.
The worker loads firmware once for each replacement and keeps the running
machine if construction fails. Explicit original images retain their existing
NVRAM paths. Tests cover switching both ways without overwriting OBP state.

The core firmware interface is absent from normal constructors. SS-5 admits
only its exact service page through ASI 2d and explicit 4 KiB PTE mappings;
other mappings retain the 31-bit hardware semantics. The relocated boot fetch
alias exists only with that service installed. Wide/atomic accesses cannot
partially consume or alter the stream.

## Source port

Upstream: `https://github.com/openbios/openbios`, commit
`e5ac46dd24e6216c36aa80462af25457e7029440`, the gitlink in documented QEMU
`f71afb409a87c4ecd0f9d63a5963eb645dfab780`. The project website supplied by the
user returned HTTP 403 here; official source was available.

The stock image exposed actual firmware assumptions rather than missing Sun
registers. The tracked source patch addresses:

- Empty delay functions, endless retries for an absent floppy, ESP reset
  without NOP and incorrect interrupt/status sequencing. Failed selections
  previously fabricated disks at every target.
- Missing SBus arbitration before DMA, default CD slice `a` instead of the
  Sun boot slice `d`, and a fictitious SS-20 APC node which made NetBSD panic.
- CPU MIDs and physical `reg` tuples (8/A), secondary AER admission after
  publication of startup context, and 85/75 MHz nominal clocks.
- Historical TurboGX `intr`, sun4m SBus PIL translation, and enough virtual
  I/O space for the 1 MiB framebuffer plus device registers. The linker bounds
  the 2 MiB virtual-I/O reservation below OFMEM's virtual ceiling.
- `fb8-install`'s missing/private `color!` path leaked four arguments and later
  dereferenced address 4. Discarding the whole failed invocation fixes the
  stack error without modifying the original FCode ROM.
- Legacy `pack` lacked Sun's trailing NUL. Reusing the numeric buffer turned
  width 1152 into 115280000 and height 900 into 900280000, then the core
  correctly rejected the unsupported CGsix resolution. Each version-1 FCode
  table now binds the legacy convention without changing ordinary `pack`.
  The independent FirmWorks primitive is linked in `docs/references.md`.
  `tools/openbios/checks.fs` exercises shortening, empty and maximal strings,
  terminators/sentinels, and the unchanged default token binding.
- Keyboard initialization only mapped the SCC in stock OpenBIOS, leaving its
  receiver disabled. The port programs 1200 8N1 and uses a one-character
  lookahead so releases, idle and reset/layout responses cannot block a poll
  or trigger an empty FIFO read. Hardware keyboard/SCC code is unchanged.

`tools/build_openbios.py` clones the pinned commit, applies the patch, builds
from a fresh directory, and records patch/image/compiler provenance beside
the ELF. Upstream incremental make does **not** reliably rebuild after linker
script changes. Use a fresh work/output directory. GCC 16 needs GNU C11 and
warning-tolerant compilation for this older source; Rust checks stay strict.
Timestamps prevent a claim of bit-for-bit reproducibility.

Initial acceptance image: `local/firmware/openbios-helios-sparc32.elf`, SHA-256
`fda89b11311f24d14e6d4ef6f0086a0d5e2bb56bc2a01ab4041392635330d0c1`;
patch SHA-256 `bfb342b2dbd1a8f1c5bea8aff387bf6226cf129a61c72c669d4c4165e57e9343`.
The neighboring JSON records the pinned source and GCC 16.2.1 cross compiler.

## Acceptance evidence

Private evidence directory: `local/openbios-2026-11-16/`.

- Core full validation: 683 unique normal tests and 700 with all features;
  one isolated MXCC child adds an extra successful raw test-log line.
  Formatting, both Clippy configurations with `-D warnings`, and all four
  demos pass (`core-validation.log`).
- Heliacal: 80 tests, formatting, Clippy, release rebuild, dummy/software SDL
  smoke (`frontend-final.log`). Original-OBP display smokes pass for SS-5,
  two-CPU SS-20/SX and two-CPU Axil/CGsix (`obp-*-smoke.log`). These are display
  checkpoints, not repeat OS boot claims.
- OpenBIOS serial Forth and `1 2 + .` pass on all profiles. Capacity checks
  include SS-5 256 MiB and two-CPU Axil 512 MiB. The shared validator rejects
  below-32-MiB and sparse populations before attaching media.
- NetBSD 11.1 installed CHD: SS-20/128 MiB single-user shell, write, clean
  unmount/power-off, reopen and checksum `3147149250 24 /openbios-test`.
  `netbsd-final`, `netbsd-reread`, and `netbsd-dual-max` logs retain these checks.
  At 512 MiB with two CPUs configured the supplied GENERIC kernel still reports
  `hw.ncpu=1`; CPU 2 does not execute. This is not a NetBSD SMP acceptance.
- Real Heliacal headless run: automatic selection with empty PROM, PTY shell
  input, persisted checksum, embedded serial screenshot, clean guest halt,
  host Quit status 0 and removal of the owned PTY link. The final port, including
  keyboard setup, passed again in `ui-headless-keyboard/`.
- Initial Heliacal graphical runs: SS-5/1 CPU, SS-20/2 CPUs and Axil/2 CPUs,
  each at 128 MiB with TurboGX and empty PROM. All show the Forth prompt and
  captured `1 2 + . cr` producing `3`; release and host Quit return status 0.
  Reviewed screenshots: `ui-{ss5,ss20,axil}-keyboard/6.png`. This is firmware
  input/display acceptance, not graphical OS or secondary-CPU execution proof.
- The tracked interpreter checks pass (`forth-checks-final.*`): each assertion
  returned `ok`; the failure marker appears only in its definition's echoed
  input. This run preceded the independent keyboard-only port change.
- Solaris 2.6 CD, final per-module topology: two-CPU Axil/128 MiB reaches a
  single-user shell; `psrinfo -v` reports processors 0/2 online at 75 MHz.
  `sync; halt` succeeds. Counts: CPU 0 2,965,469,530 outcomes, CPU 2 920,658,791;
  total 3,885,614,498 executed instructions, 478,254 synchronous traps and
  35,569 interrupt entries. See `solaris-cd-final.*`. This run used the port
  after the per-MID register fix, before the final legacy-string and keyboard
  changes; its provenance is retained in `solaris-tested-firmware.json`.
- NetBSD CD reaches its microroot media-selection prompt and accepts `cdrom`,
  then asks for `/dev/cd0a` (`netbsd-cd-final.*`). No new installation is claimed.

## Boundaries

Native SX has no replacement-firmware driver; Heliacal rejects graphical SX
with OpenBIOS before allocation. Sparse DSIMM populations remain supported
hardware, but require original OBP because this port describes contiguous RAM.
Axil uses the compatible SS-20 device tree and retains its own IDPROM identity.
OpenBIOS halt and abort callbacks power off instead of returning to a monitor.
No general desktop, NetBSD SMP, network boot or full installation claim follows
from these checkpoints. See human-facing compatibility notes in
`docs/firmware.md` for the accepted display paths.

All writable guest tests use disposable clones. The user's NetBSD CHD, source
Solaris disks, firmware and live Heliacal catalog are not test destinations.
Completed-run clones, intermediate firmware/source/build trees, old failure
captures and temporary editing scripts are removed. Compact accepted logs,
test harness sources, configurations and final screenshots remain. The installed
fallback ELF/JSON and pre-existing stock comparison firmware remain available.

## Blank-startup follow-up

The user's 256 MiB SS-5 configuration was correct. Both consoles were slow,
not stalled: with a read-only Solaris disk attachment, serial first output
arrived at 43.6 s and the prompt at 78.6 s; graphical output was still blank at
180 s and subsequently accepted keyboard input. The active saved record named
`SS5-headless` had `headless=false`, so its prompt belonged on the framebuffer.
The live configuration, its NVRAM and source media were not edited.

A sampled guest PC of `0x6134` identified upstream `entry.S`'s whole-RAM clear:
five instructions per four-byte word before console initialization. The port
now uses eight aligned doubleword stores per 64-byte block, preserving the
entire original range, RAM-end/config-port/CPU-ID registers, the primary-only
condition and the optional 16 MiB kernel-preservation start. There is no host
skip or change to Sun hardware. Independent checks dirty every RAM word, run
the actual firmware to `clear_done`, and verify every word is zero on cold
start and reset: SS-5 32/256 MiB, SS-20/Axil 512 MiB (8/8 passes).

Matched core first-serial instruction counts: 364,450,000 -> 91,820,000;
measured wall time 36.59 -> 10.35 s, with other acceptance processes running.
The 256 MiB clear itself takes 62,914,560 loop instructions rather than
335,544,320. The full frontend run reaches its first serial bytes at 11.2 s
and its Forth prompt at 47.3 s. CGsix shows the prompt before any injected key
in the 180 s capture; `1 2 + . cr` returns `3` afterward. Both Quit with status 0.
TurboGX still takes minutes to initialize; this is not a general speed fix.

Heliacal now shows an OpenBIOS startup notice while awaiting selected-console
output. It is presentation-only, cleared by serial bytes in headless mode or
nonuniform decoded pixels in graphics mode, and latched off so later guest
screen clears do not redisplay it. Firmware test patterns can count as first
visible pixels before the final prompt. The notice respects pause, is reset on
cold replacement, and adds no continuous decoding once output is established.

Current installed ELF SHA-256:
`084fa665e089edb0a00532f15bcc5fe383401f23f78ef1fd88dc8221bf161616`;
patch SHA-256 `bb783061e40a556f95ba77deac7cd9d5ef72153c3914c6513d53fc266a101a8f`.
Heliacal: 81 tests, formatting, Clippy, release build and SDL smoke pass.
Original SS-5 OBP display smoke also passes (98,969,600 steps). Core Rust code
is unchanged from the 683/700-test validation above. Earlier OS/SMP acceptance
used the previous firmware builds; this follow-up verifies the clear's exact
memory/register result and both console paths, not a fresh OS installation.

Evidence: `local/openbios-startup-review/`, including `profile*.log`,
`ram-clear-final.log`, `frontend-validation.log`, `obp-smoke.log`, and private
`{serial,graphical}{,-optimized}/` captures/timing. Retained independent harness
sources are `startup-profile.rs`, `ram-clear.rs` and `measure.py`; `clear_done`
PC `0x617c` in the RAM fixture belongs to this exact build. Temporary source,
build trees, probe executables and test NVRAM are removed after validation.

## Native-search follow-up

The [startup optimization](openbios-startup-optimization.md) supersedes the
previous timing and installed-image details above. It reduces the measured
256 MiB serial prompt to 15.2 s and first CGsix banner pixels to 21.4 s, with
a fresh image and repeated firmware/OS acceptance.
