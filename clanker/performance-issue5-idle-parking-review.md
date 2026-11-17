# Issue #5 idle-parking design review

Reviewed 2026-11-17 against Helios `e72304c` and Heliacal source `2d07819`.
The handoff is useful investigation material, but its execution contract is
not yet ready for a parking implementation. This review changes no production
code. Issue #5 remains open; no commit or merge was performed.

## 1. Separate idle classification from processor eligibility

The proposed requirement that no processor be runnable/eligible prevents the
intended UP case from parking. Processor 0 remains arbitration-enabled while
executing Solaris's idle loop. `ProcessorRunState::Ready` does not mean that
the guest has useful work; `ProcessorArbitration::Enabled` means that its
module can request the bus. See `src/topology.rs:148` and the eligibility
checks in `src/machine.rs:1336`.

Use a separate, non-architectural classification: the enabled processor is
at a proven idle boundary. Do not change AER or scheduler run-state semantics
to make the classifier pass. The first implementation should reject every
multi-CPU configuration, including configurations whose second CPU currently
has arbitration disabled. SMP parking needs a separate design and tests.

Hot-PC samples and ELF symbol names are not sufficient proof. Before adding
a recognizer, retain the exact instruction sequence, taken branches, delay
slots, memory reads/writes, and entry/exit state of one complete candidate
iteration. Select a canonical PC/nPC boundary where skipped iterations have
no required architectural effect. Audit register/window/PSR, MMU/TLB and PTE
effects, idle bookkeeping, and all conditions that leave the loop. Any state
excluded from differential comparison needs an explicit justification.

An image signature alone is insufficient if code, mappings or permissions
change. Define runtime revalidation/invalidation, including guest/DMA writes,
MMU changes, interrupts and reset. Observe executed instructions or use a
passive inspection path; do not introduce diagnostic fetches through the live
MMU. Unknown or unproved loops continue executing. Keep OBP excluded initially.

## 2. Specify a real timebase and deadline contract

There is no common absolute UP/SMP clock to reuse as currently proposed:

- `Machine::step()` advances the device bus with the CPU quantum plus PROM
  waits (`src/machine.rs:1480`), but does not advance `scheduler.virtual_ns`.
- `Machine::advance_ns()` advances the bus and the scheduler's external clock
  (`src/machine.rs:1165`). It does not service DMA transactions by itself.
- `SystemBus::advance_ns_fast()` retains up to a 2 us batch of deferred time
  in `device_ns`; the ordinary `SystemBus::advance_ns()` does not consume that
  deferred balance (`src/system_bus.rs:1104`). A deadline query based only on
  current device registers can therefore miss elapsed CPU time.

Specify which clock a deadline uses, how pending device time is included,
and exactly where it is serviced. Preserve the existing interrupt sampling
boundary and fractional CPU/timer phases. Merely flushing pending time during
an observational query can deliver an interrupt earlier than the interpreter.
Do not route normal UP execution through the scheduled path to obtain a clock.

Define and test the query's behavior for overdue events, simultaneous events,
no known deadline, overflow, reset and errors. Define when DMA/IRQ service is
required after advancing and before returning. Preserve the accepted SCSI
DMA-reset-held interrupt boundary from the device-service slice.

Choose a host/virtual-time policy before enabling sleep. One candidate for
the opt-in experiment is a documented 1:1 conversion only during a parked
interval, anchored at `(host_monotonic, virtual_time)`. Whatever policy is
chosen must specify early input, host oversleep, pause/resume and remaining
elapsed time after reaching the first deadline. Never catch up across several
interrupt deadlines without giving the CPU its required service opportunity.
Fast execution remains instruction-driven outside the parked interval; the
library accepts explicit virtual deltas and never reads the host clock.

## 3. Make wakeup validation executable

The example hint contains a generation, but `advance_idle_ns(max_ns)` does
not consume it. A documented generation without a validation mechanism does
not prevent stale advancement. Accept/revalidate a parking token, or provide
an equally explicit atomic ownership protocol. Define which mutations revoke
it and fail safely when a previously returned hint is stale.

Core state and frontend input have different owners. Heliacal currently puts
keyboard/mouse/serial input into a mutex-protected queue, uses a separate
command channel, polls the PTY through `SerialBridge::service`, and polls NAT
with host `Instant` values. Waiting only on the command channel will not wake
for all of these sources. A core generation cannot see unread host input.

Specify the full announce/check/wait protocol and notification coverage.
Drain/recheck commands and input before advancing virtual time after a wait.
Define whether timestamps mean host arrival or worker observation; current
PTY polling does not timestamp actual byte arrival. Until FD/network wakeups
are integrated, any bounded recheck interval must be explicit and measured.
Pending serial BREAK is another rejection/wakeup condition: its release is
currently driven by execution-outcome count, which does not progress while
parked (`../heliacal/src/emulator.rs:626`).

Required race fixtures include input before the hint, between hint and wait,
during wait, and during reset/replacement. Cover queued output/backpressure,
GUI typing/paste, PTY input, network progress and shutdown. Stale notifications
must not inject input into a replacement guest or lose a new guest's wakeup.

## 4. The PTY launch is reproducible; baseline scope still matters

The failed harness's log contains only the SDL startup line, with no
successful firmware/PTY startup message. It does not establish an `openpty`
failure. An independent diskless run using the installed Heliacal release
created the PTY, reached original SS-20 OBP, completed 20 Forth transactions,
and exited through the UI with status 0.

The working runner waits for a visible process-owned window, allows SDL
startup to settle, checks focus/keyboard command results, and retains launch
screenshots and the serial transcript. It uses the correct companion path
`idle.cfg.nvram.ss20`, seeded from a copy of `local/ss20-ttya.nvram`, rather
than `idle.cfg.ss20`. It opens the slave with `O_NOCTTY | O_NONBLOCK` and
verifies that terminal echo/canonical mode are off. Thus measured echoes come
from the guest. Each single-byte echo is followed by completion of `1 2 + .`
with `3` and another `ok`, so a local transport echo cannot satisfy the test.

Results for this installed binary, SS-20/one CPU/128 MiB/headless/no media,
under Xvfb/software rendering:

| Observation | Result |
| --- | ---: |
| PTY-write to matching guest echo, 20 trials, median | 0.807 ms |
| Same trials, nearest-rank p95 | 1.038 ms |
| Maximum observed | 1.607 ms |
| OBP idle worker CPU, 5.00 s window | 99.75% of one host CPU |
| Main thread CPU in that window | 0.40% |
| Process exit | 0 |

This resolves the launch/measurement blocker. It is an OBP PTY transport
measurement, not physical-key-to-visible-pixel latency, not a Solaris result,
and not a parking improvement. The installed binary's SHA-256 is retained;
its exact build inputs were not reconstructed, so these numbers are not a
matched baseline for `e72304c`. Rebuild and record both repository revisions,
features/profile, binary hash and media hashes for the actual off/on study.

Evidence: `local/review-issue5-idle-2026-11-17/measure.py` and
`checked-launch/{inputs.json,events.json,latency.json,cpu-sample.json,serial.log,heliacal.log}`.
The control-window capture was taken immediately after the serial prompt was
received and can lag the PTY output; use `serial.log` as the prompt evidence.
The earlier failed diagnostic attempt is not part of these results.

The ALU/memory/device figures in the original handoff are historical Slice A
numbers, not fresh measurements of this branch. A batch duration extrapolated
from median throughput is an estimate, not an input-latency upper bound.
Similarly, multiplying guest outcomes by 100 ns omits additional waits; the
189,798-step historical command response must remain a step count unless its
actual virtual-time delta is retained. OBP is a regression/control workload
while the first classifier targets Solaris. A fresh Solaris idle baseline is
therefore necessary before judging the intended CPU-use reduction.

## Work order for the next return

1. Revise the handoff to settle the four contracts above. Retain a complete
   candidate-loop trace and an independently justified safe boundary; if
   none is proven, do not implement a recognizer yet.
2. Implement only the time/deadline and diagnostic foundation first. Keep
   execution and all sleeps unchanged. Test UP quantum/fractional/PROM time,
   pending sub-2-us time, coincident deadlines, reset scopes and the held-reset
   IRQ boundary, plus the listed device phases. Include off-path throughput
   checks and observational-query snapshot equality.
3. Return that foundation for review before implementing an opt-in UP
   recognizer/parking token and the frontend wait protocol. SMP, OBP parking,
   JIT, WP/TRCR changes and guest firmware changes remain outside this slice.
4. Use a fresh linked build to record Solaris idle CPU and PTY latency before
   the on/off comparison. Retain raw timing, exact configurations, commands,
   source hashes and clean shutdown evidence; use disposable disk clones.

Full Rust validation was not repeated for this documentation/harness-only
review. The accepted production baseline remains the device-service slice's
727 normal / 730 perf-counter tests and required checks. The independent
diskless run is additional launch/latency evidence only. Own test processes,
temporary NVRAM/lock/PTY state and failed-run artifacts were cleaned; retained
evidence and source media are preserved.
