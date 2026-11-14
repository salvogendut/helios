# SMP slice 3: deterministic scheduling and synthetic SMP

Corrected implementation submission on `feat/sun4m-smp`, based on accepted
slice-2C checkpoint `6b11d3e`. The independent review clock, software-reset
IOMMU and guest-integration findings are addressed below. Production
construction and firmware boots remain single-CPU. Two-CPU execution is
available only through the explicit synthetic constructor; no
firmware/device-tree patch, CPU-count UI, original-OBP secondary startup or
two-CPU OS experiment is included. The corrected slice has passed independent
review; see [the acceptance record](smp-slice3-review.md) for final boundary
fixes, validation and a matched Solaris measurement.

## Ownership and API

- CPU 0 remains `Machine::cpu`. CPU 2 is an independently constructed
  `ProcessorModuleState` stored in `Machine::secondary_cpu`, with its own
  `Cpu`, MMU, FPU, windows, initiator identity and MXCC. It is never cloned,
  mirrored through CPU 0, or swapped through a temporary CPU-0 slot.
- `Machine::with_profile_processors(profile, ram, prom, nvram, processors)`
  constructs one or two modules. SS-5 rejects two; SS-20/Axil accept one or two.
  `with_profile` continues to select one CPU. `secondary_cpu`,
  `secondary_cpu_mut`, `processor_cpu` and `processor_cpu_mut` expose the
  authoritative storage. `secondary_cpu_with_bus_mut` provides disjoint
  access for direct MMU diagnostics.
- Construction installs CPU 2 presence, its MXCC window and its interrupt/
  timer bank. Processor 2 starts arbitration-disabled after construction and
  reset. No hotplug path exists.
- `Machine::set_processor_entry` and `Cpu::set_pc_npc` are synthetic-test setup
  helpers. They do not add a guest startup protocol.
- The default `Machine::step` path remains the single-CPU/CPU-0 compatibility
  path. Two-CPU callers use `step_scheduled`/`run_scheduled`.

## Scheduling contract

- One host thread executes one processor event at a time. Each eligible
  processor has an independent next-ready deadline. The scheduler selects the
  earliest deadline and breaks ties by hardware processor number (0 before 2).
- Arbitration eligibility means present and AER-enabled. A selected CPU with
  a latched error reports its existing tagged stop without executing again.
  Arbitration denial retires no instruction, trap or error. CPU 2 is denied
  until guest/host AER bit 2 is set; an interrupt never grants arbitration.
  Admission clamps expired readiness to the current clock without cancelling
  a pending instruction/PROM wait; module reset begins at the current clock.
- Pending Ethernet/SCSI service runs once per distinct deadline, before the CPU
  outcome. Devices advance to that deadline. Tied CPUs see one service
  snapshot; device-time service is not multiplied by CPU count.
- On success, only the selected CPU is charged `100 ns + its physical PROM
  wait`; its next deadline moves to that completion. The shared clock then
  advances to the earliest next eligible deadline, never past another CPU's
  earlier opportunity. A slow PROM read therefore does not stall the other
  CPU. Error stops do not advance time. A completed reset store stops both
  processors and charges its quantum/wait to retained RTC/serial state before
  resetting the board and scheduler. It counts as an instruction and returns
  `Step::SoftwareReset`.
- `ScheduledStep.virtual_ns` is the shared clock at return, normally the
  earliest next eligible deadline, not necessarily the selected instruction's
  completion. Interrupt sampling uses the selected processor's bank.
  `SchedulerError::Cpu` carries the tagged fault; `Device` carries DMA service
  failure and `NoRunnable` replaces an infinite spin.
- `SchedulerStats`/`ProcessorCounters` count instructions, synchronous traps,
  interrupt entries and errors per hardware ID and in aggregate. They are
  reset by whole-machine or guest software reset. `run_scheduled(budget)`
  executes a bounded number of successful processor events.
- Whole-machine reset resets both owned CPUs/MXCCs, shared devices, AER,
  standalone IOMMU and scheduler once, preserving topology/module identities.
  Guest software reset resets both CPUs, shared devices, the standalone IOMMU
  and the scheduler, so pre-reset DMA translations cannot survive.
  `reset_processor_module(0|2)` resets only that CPU/MMU/MXCC, moves that CPU's
  readiness to current shared time and preserves the other CPU, devices, AER
  and IOMMU.
- Atomic translation and fault handling remain inside one CPU scheduling
  boundary. Shared RAM is the coherence model; no host threads, cache timing,
  JIT or idle parking were added.

## Tracked regressions

`tests/smp_scheduler.rs` covers, on both SS-20 and Axil where applicable:

1. Explicit two-module construction, SS-5 rejection, CPU-2 identity and
   one-CPU defaults.
2. Both hardware IDs make 1,000 instructions each with repeatable tagged
   steps; separate registers, FPU and PC/nPC; aggregate/per-CPU counts and
   shared virtual time replay. Adding CPU 2 preserves the 100,000 ns cadence.
3. Arbitration denial executes/counts nothing; equal per-CPU work preserves
   virtual time, system timer and processor-timer readbacks.
4. A real guest `sta` AER write through ASI `0x2f` admits CPU 2 only at the
   next scheduler boundary.
5. A real guest CPU-0 `sta` software IPI reaches CPU 2's bank. CPU 2's guest
   handler acknowledges the bank, flushes its local TLB, publishes completion
   through shared RAM and returns; the stale pre-IPI mapping and fresh
   post-handler mapping are both checked. CPU 2 continues post-return work.
6. Shared RAM publication from CPU 0 is observed by CPU 2.
7. Contended `SWAP` and `LDSTUB` locks are shared and both CPUs make lock
   progress without starvation or partial lock values.
8. CPU 0 and CPU 2 TLB entries are independent; a local context flush updates
   only the issuing CPU while the other retains the old translation.
9. A nonzero PROM wait delays only the issuing CPU; CPU 2 gets successive
   scheduler opportunities while CPU 0 waits.
10. CPU-2 synchronous traps and host-stop errors are tagged; CPU 0 fault state
    remains independent. A guest reset store is tagged and clears the
    standalone IOMMU translation cache.
11. Software reset through both `step` and `step_scheduled` clears standalone
    DMA translations; power-on reset clears the cache and module-local reset
    preserves it.
12. Equal virtual time advances CGsix retrace state identically with one or
    two CPUs. SCC register/output state is shared identically; the SCC remains
    event-driven without a free-running baud clock, so elapsed-time progress
    is not claimed.
13. Module reset preserves the other CPU; whole reset clears both.

The final independent review adds `tests/smp_scheduler_boundaries.rs` for
host/guest re-grants during an outstanding wait, retained RTC time and CPU-2
reset attribution, and scheduled distinct contexts with a pending standalone
DMA flush. These complement the guest shootdown protocol.

Expected state is derived from the SPARC/guest program and shared-RAM/timer
rules, not from scheduler internals.

## Performance

The fixed one-CPU release benchmark is a bare-metal `ba 0; nop` loop,
20,000,000 `Machine::step` calls, built with the same release/LTO settings
before and after the change (`/tmp/opencode/slice3-baseline`).

| Run | Before edit median | After edit median |
| --- | --- | --- |
| Wall | 1.077 s | 0.998 s |
| ns/step | 53.844 | 49.887 |
| Range (ns/step) | 53.496-55.409 | 49.187-50.749 |

No fixed-work regression was observed in this host-noise-limited run. A
second matched synthetic guest phase (1,048,576-iteration shared-RAM
load/add/store loop, 20,000,000 `Machine::step` calls, same release build)
against a clean `6b11d3e` worktree measured 52.454 ns/step before versus
50.456 ns/step after (medians). Logs:
`local/smp-slice3-2026-11-14/up-{baseline,post-change}.log` and
`guest-phase-{base,post}.log`. This does not promise throughput scaling with
two CPUs.

## Submitted validation

Corrected submission before the final review fixes, 2026-11-14 (logs under
`local/smp-slice3-2026-11-14/`):

- `cargo fmt --all --check`: pass.
- Normal core tests: 598 unique tests, 0 failures (599 successful summaries
  include the duplicated isolated MXCC child).
- `--features perf-counters`: 600 unique tests, 0 failures (601 summaries).
- Independent review fixtures: 5/5 pass (`review-fixtures.log`).
- Clippy `--all-targets -- -D warnings`: pass normally and with
  `perf-counters`.
- Demos `demo`, `trap-demo --trace`, `serial-demo`, `timer-demo --trace`:
  pass.
- Heliacal via `distrobox enter my-distrobox`: fmt, Clippy, 29 tests and
  release build pass against this core.
- Fresh disposable single-CPU boots to root, `uname -a`,
  `HELIOS-BOOT-ACCEPTED`, `sync` and clean OBP halt:
  - SS-5: `SUNW,SPARCstation-5`, 1,324,300,000 steps.
  - SS-20: `SUNW,SPARCstation-20`, 1,245,700,000 steps.
  - Axil: `sun4m`, 1,273,900,000 steps; first run wrote
    `/helios-slice3-test` (`2085311958 15`) and retained `/helios-disk-test`
    (`3034423527 17`); a second boot read both values and halted cleanly.
  - Fresh final clones were used under `/tmp/opencode/slice3-boots-corrected`;
    the checked source media hashes and mtimes are retained under
    `boot/source-hashes.txt`.
- Source disk/NVRAM images were not modified; only reflink clones under
  `/tmp/opencode/slice3-boots` were writable.
- Synthetic trace: `synthetic-trace.log` shows alternating processor IDs
  0/2, PC/nPC, event category, per-CPU counts and the shared clock; an
  independent replay produces an identical trace. The first event can report
  the pre-advance shared clock when the other CPU already owns the earliest
  next deadline.

Final review validation: **601 normal / 603 feature core tests**, seven
independent fixtures, formatting, both Clippy configurations, all demos and
Heliacal's 29 tests/release rebuild pass. The independent matched SS-20 Solaris
idle comparison against `6b11d3e` measures 14.730 versus 14.753 seconds median
per 204.8M-step window, with identical guest event counts, serial output and
final NVRAM. Details, limits and evidence paths are in the acceptance record.

## Review notes / limits

- `Machine::step` remains CPU-0-only by design; mixing it with
  `step_scheduled` is not a supported timing model.
- SS-20/Axil physical PROM wait in the existing bus is only accounted for the
  SS-5 low PROM range; the controlled wait regression deliberately uses that
  existing synthetic window. The scheduler uses the same `finish_cpu_step`
  accounting for both CPUs, and the high-PROM hardware timing gap remains
  explicitly documented rather than inventing a constant.
- The SCC model is event-driven and has no free-running baud clock, so the
  equal-time regression covers CGsix retrace and system/processor counters;
  SCC state remains shared and unaffected by CPU count.
- No production two-CPU boot, original-OBP secondary startup or frontend CPU
  count control is claimed. Issue #2 remains open.
