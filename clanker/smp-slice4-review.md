# Slice 4 review: CLI corrections and SMP desktop acceptance

2026-11-15. Reviewed changes on `feat/sun4m-smp-startup` based on `249d785`;
Heliacal `5c97099` on `feat/cpu-selection`. The user subsequently approved
integration of both reviewed branches. No issue closure is part of this review.
The library CPU/MMU/bus/scheduler code is unchanged in this slice.

## CLI corrections accepted

The earlier review reproduced two blockers: normal AUXIO2 power-off exited
status 1 with dual topology, and the CLI slowed the UP raw loop by 32.6% and
the synthetic UP SS-20 boot loop by 5.2%. Both are now resolved:

- `execute_scheduled` recognizes `SchedulerError::Cpu { fault, .. }` with
  `fault.is_power_off()` as successful `System powered off.`. Other CPU errors,
  DMA errors and `NoRunnable` remain failures. Independent synthetic guest
  fixtures pass for both processors on both profiles and the UP control.
  Tracked tests additionally check NVRAM output on successful power-off.
- UP retains its own loop; scheduling is cold and out of line. The author's
  24-pair actual-CLI measurements against clean `249d785` report
  0.477641 -> 0.484787 s for raw 10M steps / 1 MiB (+1.50%) and
  0.557410 -> 0.547797 s for synthetic SS-20 boot 5M / 16 MiB (-1.72%).
  The raw samples reproduce these medians. An independent four-pair raw rerun
  measured 0.487103 -> 0.483697 s. One baseline sample was an outlier; these
  support removal of the large regression, not a claimed speedup or a full
  Solaris boot timing.
- Guest AER admission, CPU-2 errors, invalid counts and option ordering have
  tracked coverage. This review strengthened
  `scheduled_guest_aer_counts_match_trace_and_secondary_error_is_tagged`:
  both CPUs must execute, numerical counter tuples must equal their tagged
  trace counts, and the combined instruction budget must be 100. Merely
  printing a CPU-2 label no longer satisfies the assertion.
- Help now distinguishes shared SMP deadlines from the UP time quantum.

Independent validation passes: 605 unique normal tests, 607 with
`perf-counters`, formatting, both Clippy configurations with `-D warnings`,
all four demos and five independent CLI fixture groups. Logs contain 606/608
successful lines because an isolated MXCC child reruns one test. An initial
sandboxed suite hit denied loopback sockets; both full suites subsequently
passed with local sockets permitted.

Evidence: `local/review-smp-slice4-2026-11-15/`, including
`review-cli-corrected-independent.log`, `cli-tests-corrected.log`,
`tests{,-perf}-corrected.log` and `cli-benchmark-corrected-independent.json`.
The larger supplied comparison is
`local/smp-slice4-2026-11-14/cli-benchmark-corrected.json`.

## Solaris evidence accepted within its scope

Reviewed both profiles' original-ROM module-info, workload, persistence and
UP serial logs, plus the boot harness's construction and scheduling:

- Original OBP and Solaris discover CPUs 0 and 2 on SS-20 and Axil.
- `psrinfo -v` reports both online; `pbind` succeeds and `mpstat` shows both
  at 100% user during bound shell loops.
- Both write and reread `2034242115 15 /helios-slice4-test` and
  `3975907619 262144 /helios-slice4-io` across clean halt/reboot.
- SS-5, SS-20 and Axil UP logs show one online CPU and clean halt.
- CPU-2 instruction and interrupt counts accompany the guest evidence.

Those serial boots were not repeated in this review. The graphical SS-20 boot
below was independently rerun on a fresh clone. The supplied loops are killed
before `dd`, so their logs do not establish simultaneous allocation/I/O stress.

## Independent SS-20 SMP Heliacal input

Used a disposable clone of `local/completion-2026-11-13/desktop.raw` and its
screen-console NVRAM, original 2.25R PROM, native SX, 128 MiB and two CPUs.
No firmware patch or direct core input injection. Heliacal ran on isolated
Xvfb with SDL X11/software and actual XTEST events via `xdotool` in the
existing `my-distrobox` container. Source images were preserved.

Graphical-console root login works and reports CPUs 0 and 2 online. Manually
starting Xsun/olwm/xterm gives a working terminal. After paced mouse startup
and a client click, typed input prints `HELIACAL-SMP-INPUT-OK`; read-only UFS
inspection confirms exactly `HELIACAL-SMP-PERSIST-OK\n` in
`/heliacal-smp-input-test`.

Capture/release succeeds: typing while released creates no
`/heliacal-smp-released`; after recapture, `/recap` contains exactly
`HELIACAL-SMP-RECAPTURE-OK\n`. Pause/resume also succeeds, followed by a typed
`HELIACAL-SMP-RESUME-OK\n` marker in `/resume`.

Typing `sync; /usr/sbin/halt` in Xterm completes filesystem sync and returns
to graphical OBP `ok`. The host F12 confirmation then exits Heliacal with
status 0. Markers and geometry files were independently reread after guest
halt; `verification.txt` records exact byte comparisons and marker absence.
`halted.png` records the guest's sync/termination and OBP prompt.

A paced title-bar drag moves the client from `484x316+105+125` to
`484x316+125+125`, independently confirmed by guest `xwininfo`. The later
geometry command scrolls one 13-pixel text row; the preserved 484x221 text
region has zero differences after the (+20,-13) translation. This is a
bounded movement/text check, not full resize/iconify acceptance.

No keyboard/core patch was needed. The previous screenshot does not establish
a keyboard-emulation defect. Two harness details matter:

1. Warm up Xsun's mouse path with 64 paced small movements, then verify that
   the guest pointer hits the terminal. A host capture click is consumed;
   send a subsequent guest click. Screen and guest coordinates differ.
2. Wait for guest-visible completion after automated typing. At 70 ms host
   pacing, pending keys can remain when `xdotool` exits. Releasing capture
   clears these pending keys by design. An initial review attempt released
   too early, truncated an `xwininfo` command and appended the next command
   to it. Repeating with guest completion checks passes. An immediate
   screenshot can also catch a partially rendered move; wait for settlement.

Exact startup commands, input actions, native PPMs, host screenshots and
read-only disk checks are under
`local/review-smp-slice4-2026-11-15/gui-ss20/`; the harness is `gui_session.py`
in its parent. `console-smp-input.png`, `x-input-settled.png`,
`release-accepted.png`, `recapture-accepted.png`, `drag-after-settled.png` and
`resume-accepted.png` record accepted phases. Earlier screenshots include
intermediate observations and do not establish acceptance.

## Axil evidence and independent window/focus completion

Reviewed the additional `local/smp-slice4-2026-11-14/heliacal-axil/` evidence
and the driver scripts. The CDE terminal accepts input, both CPUs are online,
capture/release works, and the CPU-bound loops remain active while two `dd`
processes create files. Subsequent `mpstat` samples show both CPUs at 100%
user; these samples follow the writes. This is filesystem allocation/I/O
under CPU load, not a dedicated malloc/mmap stress test. Clean halt, confirmed
F5 reset/reboot, checksum reread and host Quit are recorded. Independent
read-only UFS inspection confirms all six marker/data files and their hashes,
plus absence of the released-input marker.

The review then booted a private copy of that cleanly halted Axil disk with
original Axil 2.19 and TurboGX firmware, two CPUs and SDL X11/software.
All previously outstanding Axil window/focus checks pass without core or
frontend code changes:

- The mapped CDE terminal is `0x480002c`; `xwininfo -name Terminal` instead
  selects an unmapped 1x1 helper. Query the actual client ID. A first gesture
  did not move the window; the accepted calibrated title-bar gesture does.
- Move: `593x361+114+290` becomes `593x361+154+290`. The entire translated
  client (214,073 pixels) is identical.
- Maximize: the client becomes `1139x868+8+27`; a 564x110 known-text region
  has zero differences after the (-146,-263) translation.
- Restore: geometry returns to `593x361+154+290`. Three exposed desktop
  regions match exactly. Dtterm moves earlier rows into scrollback when
  shrinking; Shift+PageUp retrieves all eight numbered rows with zero
  differences. This is guest terminal behavior, not lost rendering.
- Iconify: every pixel in the vacated 603x390 frame matches the original
  wallpaper tile. Restore preserves the numbered text exactly. Full-frame
  differences are limited to the mouse pointer and blinking text caret.
- Expose: moving xclock over the terminal changes 16,281 pixels in the
  selected 159x103 text region; moving it away restores that region exactly.
  The two-row bevel changes with focus and is excluded from this text check.
- Focus loss: holding Shift produces uppercase `A`. Unmapping the host
  window releases capture; the host Shift release occurs while unmapped.
  Remapping/recapturing and typing writes exactly `focus-clean\n` to
  `/focus-clean`, proving the guest modifier was released.
- Typing `sync; /usr/sbin/halt` returns to graphical OBP after filesystem
  sync; confirmed host Quit exits 0. Markers and geometry are reread after halt.

Evidence: `local/review-smp-slice4-2026-11-15/gui-axil/`. `verify.py` asserts
the geometry, marker bytes and nine zero-difference pixel checks;
`verification.json`, `final-guest-files.json`, action logs, named PNGs and
compressed native PPMs preserve the results. `frontend-binary.sha256` records
the tested release binary. Production CLI/test hashes are identical to the
previous independently validated 605/607-test tree, so those checks were not
needlessly repeated for this documentation/evidence follow-up.

## Merge assessment and cleanup

Slice 4 is ready for incremental merge as **experimental one/two-CPU support**.
The CLI blockers and the last reported Axil GUI gate are resolved. Do not
expand this slice merely to remove the experimental label. The user approved
integration on 2026-11-15; issue #2 remains open. The concrete native-SX and
CGsix workloads above do
not establish arbitrary OS, all-window-manager or exhaustive SMP stress
coverage; dedicated user-space allocation/mapping stress remains unverified.

Both review GUI sessions cleanly halted and exited. Removed both private
2 GiB disk clones, the clean-baseline source/build copy and duplicate automatic
captures. Named native PPMs were losslessly compressed with round-trip checks,
and `verify.py` passes again against the compressed evidence. The entire review
directory is approximately 1.9 MiB. Source firmware/disks and the other AI's
evidence remain intact. See `cleanup-2026-11-15.json` in the review directory.

Only two single-CPU modules (processors 0 and 2) are emulated. Four CPUs on
these machines require dual-CPU-module support and remain outside slice 4.
Full POST/cache fidelity is still incomplete.
