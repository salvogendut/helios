# Slice 4: original firmware and Solaris SMP

2026-11-14. Issue [#2](https://github.com/salvogendut/helios/issues/2).
Helios branch `feat/sun4m-smp-startup`, based on merged `4e6ab81` (slice-3
implementation `262aeb2`). Heliacal branch `feat/cpu-selection`, commit
`5c97099` from `95b6e10`, adds the requested experimental CPU selector. Read the final
branch tips before working. Do not redo the scheduler or frontend selection.

## Starting evidence

The unchanged merged core has now run original OBP on **two actual CPU
owners** on both profiles. These are disk-free serial tests, with a keyboard
present and existing `diag-switch?=false` NVRAM copied into memory. They do
not establish full POST or Solaris SMP support.

| Profile | Observed banner | CPU 0 outcomes | CPU 2 outcomes | Prompt |
| --- | --- | ---: | ---: | --- |
| SS-20 | `SPARCstation 20 MP (2 X SuperSPARC-II)` | 142,489,780 | 3,820,221 | `1 2 + .` prints `3`, returns to `<#0> ok` |
| Axil 320 | `Axil-320 MP (2 X 390Z55)` | 126,310,618 | 22,739,383 | same |

Both ROMs enable CPU 2 through guest AER writes, execute it from reset,
perform **one** software reset during startup, then finish initialization
and disable its arbitration at the prompt. SS-20's reset occurred at CPU-0
PC `0x000279a0`; Axil's at `0x00010378`. The reset is an observed startup
transition, not an error to suppress or a reason to stop the harness.
An additional one-CPU SS-20 scheduled run also reached a responsive `ok`.

The harness maintains its own counts across reset. Above counts include all
scheduler outcomes, not just retired instructions. SS-20 totals are
146,308,889 executed instructions, 4 synchronous traps, 1,107 interrupt
entries and 1 software reset. Axil totals are 149,049,047 / 7 / 946 / 1.
Core scheduler counters reset on whole-machine reset, so their final snapshot
alone must not be labeled as the entire boot's totals.

Reproduction and raw evidence are ignored under
`local/smp-slice4-2026-11-14/`:

- `Cargo.toml`, `probe.rs`, `Cargo.lock`: bounded headless probe. From the
  Helios root run `cargo run --release --offline --manifest-path
  local/smp-slice4-2026-11-14/Cargo.toml -- ss20 2` (or `axil 2`). It attaches
  no disks, does not set CPU entry points or force AER, and saves to new files.
- `ss20-2cpu.log`, `axil-2cpu.log`: first CPU-2 events, AER transitions,
  final ring, counts and PCs; corresponding `.serial` and `.nvram` files.
- `ss20-1cpu.*`: one-CPU scheduled comparison, not a speed benchmark.
- `ss20.log` / `axil.log`: initial exploratory runs stopped at the first
  software reset. They are superseded by the complete `*-2cpu` runs.
- `input-sha256.txt`: original ROM and NVRAM hashes. No source media changed.

This probe observes arbitration transitions, not every MMIO operation. Add a
bounded trace around the real Solaris failure as necessary; do not interpret
the current small trace as a complete startup protocol audit.

## Assignment for the other AI

1. Preserve the one/two **single-CPU module** topology: CPUs 0/2, MIDs 8/A.
   One worker executes `step_scheduled` for a dual machine; `step` stays the
   single-CPU compatibility path. Never call `step` once for each CPU or
   advance devices twice. Preserve independent deadlines, fault atomicity,
   guest AER behavior and the board-owned IOMMU. Read `AGENTS.md`, `README.md`,
   `HACKING.md`, `docs/references.md`, `docs/machines.md`, `docs/reset.md` and
   the accepted slice-3 review before edits.
2. Add a validated headless `--cpus 1|2` option to the production CLI if needed
   for reproducibility; default one, reject two on SS-5 and all unsupported
   counts. Tag CPU errors and execution reports correctly. Heliacal already
   constructs the topology and dispatches the appropriate stepping API.
3. Repeat the two-CPU firmware baseline, capture `module-info` and firmware
   CPU-node identities, then boot **Solaris 2.6 on disposable disk clones**.
   Source disks are `local/completion-2026-11-13/desktop.raw` (SS-20),
   `axil.raw` (Axil), and `ss5-final.raw` (UP regression). Preserve them and
   all disks in `local/heliacal-machines/`. Use new per-run NVRAM/output paths.
4. Follow Solaris's real `prom_startcpu` and secondary handshake. Use the
   pinned Solaris source in `smp-plan.md` as a debugging reference; match it
   to the installed binary. At the first error, timeout or unexpected reset,
   save CPU ID, PC/nPC, instruction, trap/error kind, both CPUs' MMU/interrupt/
   arbitration state, shared time and a short ASI/MMIO/memory ring. Distinguish
   normal reset from repeated reset, absent CPU, firmware park, IPI wait and
   host error. Fix the demonstrated hardware gap with an independent focused
   regression, then repeat. No firmware/kernel patches, injected CPU nodes,
   `fw_cfg`, direct kernel-entry jumps or silent ignored unsupported accesses.
5. Return for review when **both profiles** reach a shell with two CPUs online
   (`psrinfo -v` plus startup messages). Exercise work bound to each processor
   where guest tools permit; verify execution/interrupt progress on each.
   Exercise mapping/allocation and disk I/O, write a checksum marker, `sync`,
   halt and reboot to reread it. Repeat all three UP boots. If blocked, return
   the first concrete failure and minimal reproduction, not another long
   sequence of uninstrumented boots.

Use incremental serial polling with overlap, bounded batches and absolute
per-byte input pacing. Counts and budgets must state whether they mean
scheduled outcomes, retired instructions or virtual elapsed time. Keep logs
and small traces; remove completed disposable disk clones after acceptance,
retaining source images and useful evidence.

## Frontend and four-CPU scope

Heliacal now has `cpus = 1|2` in flat/named TOML, `--cpus`, F9 selection,
cold-replacement detection, tagged errors and CPU-0/CPU-2 Tinker diagnostics.
Existing files default to one. Synthetic worker tests prove both execute,
pause/resume and failed replacement preserve state, and cold replacement can
return to one CPU. Its two-CPU setting remains explicitly experimental until
Solaris/desktop acceptance. Do not claim guest support from display smoke.

The user confirmed that Axil accepts the same MBus modules as SS-20: both can
have **four hardware CPUs via two dual-CPU modules**. Sun's service manual
table 12-1 lists dual modules; its installation guide requires compatible
pairs. Current Helios supports only single-CPU modules and rejects four.
A future dual-module slice must determine module-specific CPU/MXCC ownership,
identities for processors 1/3, firmware support and validated module pairings.
Do not implement that by cloning CPU 0 four times or raising the UI limit.

After Solaris SMP passes, validate actual Heliacal keyboard/mouse,
capture/release, pause/resume, reset and shutdown, native SS-20 SX and Axil
CGsix desktop operation during disk activity. Run the complete core checks
in normal and `perf-counters` configurations, all four demos, and Heliacal
fmt/Clippy/tests/smoke/release against the exact core. Compare UP throughput
with the accepted baseline before merging. Keep issue #2 open until those
acceptance gates pass. Return reviewable changes; do not merge independently.

## Checks on this handoff

The core behavior is unchanged from `4e6ab81`. Repeated checks pass: 601
unique normal tests, 603 with `perf-counters` (602/604 summary passes including
the isolated MXCC child), formatting, both Clippy configurations and all four
demos. The first sandboxed test run stopped at the two loopback network tests
with `Operation not permitted`; the full rerun with socket access passed.

Heliacal `5c97099` passes 34 tests, formatting, Clippy, release build and SDL
smoke. An actual SDL window saved the two-CPU setting, discarded a later
one-CPU draft and quit with exit 0; the F9 layout was inspected. Disk-free
real-firmware display smoke passes on SS-5/one CPU, SS-20/two/native SX and
Axil/two/CGsix, using temporary configurations and NVRAM. Eight invalid CLI
combinations fail before worker launch. Logs and screenshots are under
`local/smp-slice4-2026-11-14/heliacal/`. Source ROM/NVRAM hashes were rechecked
unchanged. No disk was attached. Full UP Solaris regressions are the accepted
slice-3 evidence, not newly rerun boot results from this documentation/frontend
slice. The next implementation needs fresh UP and SMP guest acceptance.
