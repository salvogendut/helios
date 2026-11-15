# Clock, serial boot and CDE follow-up

2026-11-15, `fix/profile-clock-timing` (Helios) and
`fix/serial-boot-clock-followup` (Heliacal). The unrelated IRIS/M3d2 text in the
user report was explicitly withdrawn. These changes follow the merged headless
console feature; they do not extend that feature's issue scope.

## CDE stop: incompatible framebuffer ROM

Captured SS-5 error: PC `eeee3364`, nPC `eeee3368`, instruction `c2492000`
(`ldsb [%g4], %g1`), physical address `30ba8400`. With the CGsix base at
`30000000`, this is offset `ba8400`, within TurboGX+ VRAM but beyond the
implemented 1 MiB TurboGX aperture `800000..900000`.

The live configuration selected `sun-tgx+-vesa.bin`, SHA-256
`a07094d79d78d77c1fe6e4e05dab6e33aecf01cdba9ef903a3f46a337970f1e1`,
CRC-32 `fc7be95e`. The supported `turbogx.bin` has SHA-256
`3fb9a8dbe7c8621dd9a65246ac78d9353a492c41862c1dd61656fb03d1a16259`,
CRC-32 `bbdc45f8`. Sun's hardware guide table 1-1 and pinned MAME distinguish
1 MiB TurboGX/BT458 from 4 MiB TurboGX+/BT467. Increasing the allocation alone
would not implement the larger card's drawing geometry, modes and DAC.

`Cgsix::validate_rom` rejects the two identified Plus CRCs (`fc7be95e` and
MAME's `525a58db`) before allocation. Other 32 KiB ROMs remain accepted for
research, without claiming their compatibility. Both core constructors and
Heliacal settings validation use the check. The captured address range now
reports missing VRAM rather than a misleading narrow-register error. Tests
use independent synthetic CRC fixtures, never bundled firmware bytes.

## Axil 512 MiB /usr failure

The selected target-3 disk was `local/disks/solaris26-install.raw`, an SS-5
installation. Its slice-6 symlink and path_to_inst used the SS-5 SBus path.
The exact configuration reproduced `/dev/dsk/c0t3d0s6 no such device` and a
missing `/usr/sbin/fsck` through the real Heliacal PTY. The RAM capacity and
serial transport were not the cause.

Recovered a disposable copy through Solaris 2.6 CD single-user mode, rebuilding
/dev and /devices with devlinks/disks/tapes/ports and installing the new
path_to_inst, following docs/scsi.md. The original source was never attached
writable by this task. The recovered copy is retained as the user deliverable:
`local/disks/solaris26-axil320.raw`. Its slice-6 link now selects
`iommu@f,e0000000/sbus@f,e0001000/espdma@f,400000/esp@f,800000/sd@3,0:g`.

The final rebuilt Heliacal run used a further disposable copy, 512 MiB, one
CPU and no framebuffer. It reached root, printed uname/psrinfo, mounted /usr
and `HELIACAL-AXIL-CLOCK-ACCEPTED`, and cleanly halted; host Quit returned 0.
No unknown-keyboard/layout warnings remain in that final serial trace.

With the real catalog unlocked, backed it up and atomically changed only the
TurboGX ROM paths and Axil target-3 path. Named labels, other preferences and
NVRAM were preserved. The user ran the original SS-5 disk concurrently earlier
in the investigation, so its initial and final mtimes are not an unchanged-file
claim; every emulator invocation owned by this task used its own disk clone.

## Profile clocks and timer phase

The user subsequently requested an Advanced toggle because nominal rates
slowed boot. `CpuClockMode::Fast` is now the default 100 ns quantum in the
core and Heliacal. `reset_with_clock_mode(Hardware)` cold-resets both CPUs
into nominal rates; the CLI exposes `--accurate-clocks`. Heliacal stores
`accurate_cpu_clocks` per named machine, defaults false for old records, gates
it behind Tinker/Advanced, and uses Save plus confirmed F5 replacement.
The timing observations below used hardware mode: one nominal CPU cycle per
completed outcome, SS5/85 = 85 MHz; SS-20/Axil = the selected 75 MHz modules.
Fractional nanoseconds are retained independently for each CPU. PROM waits
remain separate, tied SMP deadlines share device service, and errors consume
no CPU cycle. Whole/software reset resets all fractions; local reset affects
only its module. The bare-metal/sun4m diagnostic harness retains 100 ns.

Removed a redundant timer accumulator which could delay an MMIO-visible tick
across fractional batches (505 ns then 495 ns). The timer owns its 500 ns
phase. SS-5 TRCR now reports read-only SBSP=2/MEMSP=1 pins (`01200000`), selecting
CPU/4 SBus and 85 MHz memory timing per microSPARC-II figure 5.15/tables 29/30.
No TLB WP/replacement policy was changed.

Original OBP observations: SS-5 reports 85.0 MHz and SBus divide 4 / 21.2 MHz;
SS-20 reports both modules at 75 MHz. Axil 2.19 reports 74 MHz because its
integer `ms-factor * 40 / 20000` truncates before scaling by 1,000,000. The
observed ms-factor is 37499, an estimate of 74.998 MHz. The underlying timebase
is exactly 75 MHz. No PROM instructions or reported frequencies were patched.
Higher virtual frequency means more interpreted instructions per timed guest
wait. This is not a host-throughput or pipeline/cache-timing improvement.

## Keyboard and mouse reports

Changed timing exposed an existing keyboard workaround: every host SCC
receiver epoch could enqueue a fresh `ff 04 7f` identity. Captured Solaris
traffic showed the unsolicited `ff`, followed by its explicit reset command,
then a second `ff`. Solaris treated that second prefix as an unknown keyboard
ID and fell back to Type 3. Removed the host-register-triggered response;
power-on and keyboard command 1 still identify the keyboard. The tracked
regression now verifies silent receiver reconfiguration followed by exactly
one reset response. This supersedes the receiver-resend workaround recorded
in clanker/ss20-axil.md, not the SCC epoch API used by host serial transport.

Solaris 2.6 zs_async.c identifies zs3 as the mouse channel. Its ring-buffer
warning reflects software receive-buffer exhaustion, distinct from hardware
silo overflow and the CDE VRAM failure. Existing mouse timing/range regressions
pass. No mouse protocol change or blanket claim that this warning is fixed
has been made in this follow-up. The final hardware-clock SS-5 boot and
paced mouse session added no new zs3 warning to /var/adm/messages; older
warnings remain in that log and were not erased.

## Validation and local evidence

- 636 unique normal / 638 perf-counters tests; raw logs contain one additional
  isolated MXCC child test. Formatting, both Clippy configurations and all
  four demos pass after the keyboard change.
- Heliacal: 69 tests, formatting, Clippy, release rebuild and SDL smoke pass.
- Real-firmware display smoke passed (236,267,520 steps) before the final
  keyboard correction. Final original-firmware/OS runs are recorded separately.
- The first corrected-ROM SS-5 graphical run reached CDE, showed terminal and
  clock windows, accepted `echo CDE-OK`, and cleanly halted. It exposed the
  keyboard warning described above; final rebuild revalidation passed below.
- Final Axil PTY acceptance completed as described above. Final SS-20 SMP
  reached root with CPUs 0 and 2 online at 75 MHz, /usr mounted, and
  HELIOS-CLOCK-SMP-ACCEPTED; clean halt returned to `<#2> ok` after
  8,505,500,000 outcomes / 776.3 seconds. No keyboard/layout warning occurred.
  Final hardware-rate SS-5 CDE revalidation also passed: no keyboard/layout
  warning, visible FINAL-CDE-OK terminal output, paced mouse motion and clean
  halt to `ok`. Fast-mode boots and toggle UI checks follow below.

Evidence root: `local/serial-clock-review-2026-11-15/`. Contains core/frontend
checks, original-FCode traces, screenshots, isolated harnesses, config backup
and recovery transcript. Completed scratch clones, local probe build artifacts
and downloaded reference copies were removed; the recovered Axil disk and
compact evidence are retained.


## Requested fast/accurate toggle and final fast-mode regressions

After seeing the boot cost, the user requested that nominal clocks be an
Advanced option. Fast mode is the default in both core and frontend. The
Advanced row is Tinker-gated, saved per machine, and requires confirmed cold
replacement; no running guest's calibration is changed in place. The CLI
accepts --accurate-clocks and reports the selected timebase. Existing machine
records default false. Nine core clock tests cover both policies, reset/RAM
retention and independent SMP clocks; CLI and frontend regressions cover
flag parsing, actual core construction, draft changes and named persistence.

Final suites: 636/638 unique core tests (normal/perf), 69 frontend tests, both
Clippy configurations, formatting, four demos, SDL smoke and release builds.
Fast-mode SS-20 SMP: root, both CPUs at 10 MHz, /usr, marker, clean halt;
2,152,700,000 outcomes / 207.9 s. Fast-mode Axil 512 MiB: root, 9 MHz OBP
calibration, /usr, marker, clean halt; 1,449,500,000 outcomes / 135.9 s.
These are acceptance timings, not a controlled performance benchmark.
Both fast-mode scratch disks were removed after the guests halted.

The final SDL menu exercise confirmed SS-5 `module-info` at 10 → 85 → 10 MHz
after changing the saved Advanced option and confirming each F5 replacement.
The restored fast-mode SS-5 then reached graphical CDE, accepted
`echo FAST-CDE-OK` and paced mouse motion, and halted with `syncing file
systems... done`, `Program terminated` and `ok`. Host Quit exited successfully.
Evidence is in `ss5-cde/clock-*.png` and `ss5-cde/fast-cde-*.png`.
The SS-5 disposable disk and unused test NVRAM names were removed after exit;
private test displays were stopped. Source media and the retained recovered
Axil image were not attached writable in these final acceptance runs.
