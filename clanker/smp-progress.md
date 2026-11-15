# Solaris SMP bring-up progress

Branch: `feat/sun4m-smp-startup`, based on merged slice-3 `4e6ab81`.
Implementation and review: 2026-11-14 through 2026-11-15. The user approved
incremental integration on 2026-11-15 with two-CPU support still experimental.
Issue #2 remains open; this document records the accepted workload and limits.

## Core/CLI changes

The two-CPU scheduler, independent CPU/MMU/MXCC ownership, shared device clock,
arbitration and reset behavior from slice 3 are unchanged. No firmware or
kernel patch, device-tree injection, `fw_cfg`, direct kernel jump or silent
unsupported-access conversion was added.

`helios boot` now accepts `--cpus 1|2`:

- Default remains one CPU.
- SS-5 rejects two CPUs; SS-20/Axil accept two single-CPU MBus modules.
- Counts other than 1 or 2 are rejected before machine construction.
- One worker uses `step_scheduled` for dual topology; `step` remains the
  single-CPU compatibility path.
- Trace/report records carry `cpu=0` or `cpu=2`; final reports print tagged PC
  and per-CPU instruction/trap/interrupt/reset counters.
- `tests/cli.rs` covers successful SS-20 dual boot, one-CPU default, SS-5
  rejection, missing/malformed/0/3/4 counts, argument order, real guest AER
  admission, tagged CPU-2 counters/errors and guest power-off from CPU 0 or
  CPU 2 on both profiles. Power-off still saves NVRAM on the successful exit.

The scheduled CLI path recognizes `SchedulerError::Cpu { fault, .. }` with
`fault.is_power_off()` before converting errors to strings. Guest power-off
through CPU 0 or CPU 2 now exits 0 with `System powered off.`; non-power-off CPU
faults, DMA errors and `NoRunnable` remain failures. The review's five CLI
fixture groups pass (`review-cli-corrected.log`).

The production CLI was also run directly against both ROMs:

- SS-20: `SPARCstation 20 MP (2 X SuperSPARC-II), Keyboard Present`, 150M
  step budget, CPU0 counters `(146178634, 4, 1141, 1)`, CPU2 counters
  `(3820218, 0, 3, 0)`.
- Axil: `Axil-320 MP (2 X 390Z55), Keyboard Present`, 160M step budget,
  CPU0 counters `(137259558, 7, 1052, 1)`, CPU2 counters
  `(22739380, 0, 3, 0)`.

## Firmware evidence

Fresh headless runs used the original ROM and initialized NVRAM with a
keyboard present, no disk injection, and the guest-controlled AER startup.
`module-info` at the `ok` prompt reports:

```text
SS-20: MBus 10 MHz, SBus 5 MHz
       CPU#0 10 MHz SuperSPARC-II / SuperCache 1.x
       CPU#2 10 MHz SuperSPARC-II / SuperCache 1.x
Axil:  MBus 9 MHz, SBus 4 MHz
       CPU#0 9 MHz SuperSPARC / SuperCache
       CPU#2 9 MHz SuperSPARC / SuperCache
```

The supplied OBP probe evidence remains reproduced by the unchanged core:
SS-20 reaches the arithmetic prompt with 142,489,780 CPU0 and 3,820,221 CPU2
outcomes; Axil reaches it with 126,310,618 / 22,739,383 outcomes. Both ROMs
enable CPU 2 through guest AER writes, run it from reset, perform one software
reset during startup, and park it at the prompt. Logs:
`local/smp-slice4-2026-11-14/ss20-2cpu.log` and `axil-2cpu.log`;
current module-info runs: `boot/ss20-module-info.serial` and
`boot/axil-module-info.serial`.

## Solaris 2.6 SMP

Disposable clones of the accepted SS-20 (`desktop.raw`) and Axil
(`axil.raw`) disks booted with two CPUs. No disk or NVRAM source was modified.

```text
SS-20: SunOS helios 5.6 ... sun4m sparc SUNW,SPARCstation-20
Axil:  SunOS helios 5.6 ... sun4m sparc sun4m
psrinfo -v:
  0 on-line; 2 on-line
```

The final SS-20 workload boot recorded per-CPU scheduler outcomes:

```text
CPU0 outcomes 1190755603, kinds [1190146873 executed, 581100 traps, 27629 interrupts, 1 reset]
CPU2 outcomes  813044397, kinds [ 812484182 executed, 535946 traps, 24269 interrupts, 0 resets]
```

The final Axil read/reboot recorded:

```text
CPU0 outcomes 1121109427, kinds [1120510865, 572022, 26539, 1]
CPU2 outcomes  781190573, kinds [ 780592955, 572758, 24860, 0]
```

`pbind`-bound guest shell loops drove both processors to 100% user in `mpstat`
on both profiles. The guest also performed `dd` disk reads/writes, created a
checksum marker and a 256 KiB checksummed file, ran `sync`, halted cleanly and
rebooted on the same clone. Both profiles read back:

```text
2034242115      15       /helios-slice4-test
3975907619      262144   /helios-slice4-io
```

Evidence: `local/smp-slice4-2026-11-14/boot/{ss20,axil}-final*.{log,serial}`,
`ss20-work*/`, `ss20-module-info.*`, `axil-module-info.*`.

## UP regressions

Fresh single-CPU clones reached root, printed `uname -a` and
`HELIOS-BOOT-ACCEPTED`, and halted cleanly:

- SS-5: 1,332,600,000 outcomes, one online processor.
- SS-20: 1,255,900,000 outcomes, one online processor.
- Axil: 1,267,600,000 outcomes, one online processor.

Evidence: `local/smp-slice4-2026-11-14/boot/{ss5,ss20,axil}-up.*`.

## Tests, frontend and performance

- Core: 605 unique normal tests and 607 with `perf-counters`, all pass; formatting,
  both Clippy configurations and all four demos pass. The 16 tracked CLI tests
  include both-profile scheduled power-off, guest AER/count/error and count
  validation cases.
- Heliacal `feat/cpu-selection` `5c97099`: formatting, Clippy, 34 tests and
  release build pass against this core. The two-CPU selection remains labeled
  experimental. Disk-free real-firmware display smoke passed for SS-20/two
  CPUs/native SX and Axil/two CPUs/CGsix (`heliacal-*-smp-smoke.log`). An
  interactive SS-20 two-CPU/native-SX boot from a disposable disk reached the
  graphical Solaris console login and desktop (`heliacal-ss20-smp-boot.png`),
  confirming the two-CPU worker and graphical boot path. A second capture click
  and status check reached `CAPTURED`, but typing into the visible
  `HELIACAL-INPUT` terminal produced no guest text in this Xvfb/SDL3 session.
  This setup observation was not evidence of a keyboard-device defect.
  The independent 2026-11-15 review subsequently verified graphical-console
  login, Xterm typing, capture/release, pause/resume, a 20-pixel window drag,
  clean guest halt and host Quit on a fresh two-CPU SS-20 clone, without any
  keyboard/core patch. Exact typed markers were checked from the disk after
  halt. Automated input must wait for guest completion before releasing
  capture, which clears queued keys. See [the review](smp-slice4-review.md).
  A fresh Axil/CGsix clone then reached CDE with both CPUs online; root login,
  CDE Terminal typing, capture/release/recapture, concurrent `pbind` CPU 0/2
  loops plus `dd` allocation/I/O, checksum markers, clean guest halt, confirmed
  F5 reset/reboot and marker reread all passed. The bound CPU loops remained
  active while `dd` created 1 MiB and 256 KiB zero-filled files. Subsequent
  `mpstat` samples show both CPUs at 100% user; they were taken after `dd`
  completed, not during the writes. This tests filesystem allocation/I/O
  under CPU load, not a dedicated user-space malloc/mmap stress program.
  Hashes are recorded in `heliacal-axil/verification-concurrent.txt`.
  The independent review reread all six marker/data files and confirmed the
  hashes and absence of the released-input marker. See `heliacal-axil/`.
  The independent follow-up now also accepts Axil move, maximize/restore,
  expose, iconify/restore and held-Shift focus-loss behavior, using the mapped
  CDE client ID, guest geometry, exact text/desktop pixel assertions and a
  lowercase marker after recapture. The review guest cleanly halts and host
  Quit exits 0. See `local/review-smp-slice4-2026-11-15/gui-axil/` and
  [the review](smp-slice4-review.md) for the assertions and cleanup record.
- Actual CLI path paired against clean `249d785` after the hot-path split
  (`taskset -c 0`, 24 runs each, identical raw loop and arguments):
  - `run` 10M/1 MiB: baseline 0.477641 s, current 0.484787 s (+1.50%).
  - `boot --machine ss20` 5M/16 MiB: baseline 0.557410 s, current 0.547797 s
    (-1.72%).
  The earlier 33%/5% regression is removed by giving the scheduled function
  a cold out-of-line path; the remaining differences are host-noise-limited,
  not claims of a speedup. Samples: `cli-benchmark-corrected.json`.
  Library-only benchmarks are not used as CLI acceptance evidence.

## Limits / review notes

- Only CPUs 0 and 2 via two single-CPU modules are supported. Four CPUs require
  dual-CPU modules and remain unimplemented.
- The SS-20/Axil high-PROM hardware timing model remains the existing limited
  accounting; this work did not invent SS-5 constants for MBus.
- The concrete SS-20/native-SX and Axil/CGsix SMP desktop checkpoints are
  accepted, including the previously outstanding Axil window/focus checks.
  They do not establish exhaustive window-manager or SMP stress coverage;
  dedicated user-space allocation/mapping stress remains unverified. Keep
  the one/two-CPU feature experimental.
- Full POST/cache accuracy remains incomplete. Slice 4 is recommended for
  incremental merge; issue #2 has not been closed by this review.
