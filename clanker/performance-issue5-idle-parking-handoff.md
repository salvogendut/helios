# Issue #5: idle-parking design and measurement handoff

Status: reviewed but not implementation-ready. The controlling review is
[`performance-issue5-idle-parking-review.md`](performance-issue5-idle-parking-review.md);
do not implement the proposed interface below until its eligibility, timebase
and wakeup-token corrections are settled. No parking implementation, timer
policy, idle classifier, JIT change or frontend sleeping has been added. Issue
#5 remains open. This note records the original evidence and proposal.

Review follow-up: [idle-parking design review](performance-issue5-idle-parking-review.md)
identifies required eligibility, clock/deadline and wakeup-contract corrections.
An independent diskless frontend run has now resolved the PTY launch blocker;
Solaris latency and a freshly linked baseline remain outstanding.

## Objective and non-goals

Stop a running, genuinely idle guest from occupying a host CPU while preserving
guest-visible device time, timer delivery and event ordering. The frontend owns
host waiting; the library remains headless and usable without a UI. Idle
recognition must report explicit eligibility, not a blanket "repeated PC means
idle" rule.

Non-goals in the first slice:

- JIT/decoded-cache changes; the decoded cache remains opt-in and disabled by
  default.
- Changes to WP/TRCR replacement or MMU walk semantics.
- Parking while a CPU is runnable, a device event is due, or the guest is in an
  unknown/unsafe wait.
- Treating parked time as retired instructions or as a CPU execution outcome.
- Hardcoding Solaris, SS-20 or SS-5 absolute addresses into a generic detector.

## Baseline measurements

The historical no-parking frontend baseline is the corrected Slice A trace
(`clanker/performance-issue5-slice-a.md`): on the diskless OBP prompt, the
machine worker used **99.39% of one host CPU**, the main UI thread **0.33%**, and
a paused worker **0.25%**. The trace had 28,367 samples, zero lost samples,
52.72 s elapsed and 1,002 timeline snapshots. This is the target baseline for
parking, not a claim that a particular host will reproduce the exact number.

Historical Slice A active-work throughput, fixed bare-metal benchmarks:

| Workload | Median |
| --- | ---: |
| ALU | 42.69 ns/step |
| Memory | 54.85 ns/step |
| Full device ALU | 88.42 ns/step |
| MMU fast-load overhead | 11.07 ns |

Refresh these on the reviewed source/build before an off/on comparison; they
are not fresh measurements of `e72304c`.

### Input latency: historical evidence and resolved launch blocker

A historical OBP probe on the same firmware recorded the first prompt at step
93,481,624 and the reply to an eight-byte Forth command at step 93,671,422 in
`local/probes/obp-ok-ss5/configuration.json` and `library-trace.log`. That is
**189,798 guest steps** from input
injection to the next `ok` prompt. This is an OBP command round trip, not a
single-key end-to-end frontend measurement, and it must not be presented as the
parking acceptance number.

Heliacal's worker currently runs 2,048 `step_machine` calls per batch and
checks its command channel only between batches (`heliacal/src/emulator.rs:556`,
`:690`). At those historical fixed-work rates, the batch duration estimate
is roughly **0.09–0.18 ms** for ALU-to-full-device work; this is not an input
latency upper bound. UI/event-loop latency has not been measured end to end.

A fresh PTY measurement was attempted with
`local/issue5-idle-2026-11-17/measure.py`; Heliacal did not create its serial
PTY within 30 s and the script stopped with `RuntimeError: no PTY`. A later
headless boot attempt reached the control window but still did not complete the
measurement. The independent review subsequently completed a checked launch,
20 guest-verified PTY transactions and a clean exit using the installed binary:
0.807 ms median / 1.038 ms p95 single-byte guest echo. See the review for raw
evidence and scope. The launch blocker is resolved; a fresh linked-build
baseline and Solaris shell latency are still required before parking results.

## Observed idle loops

### OBP

The SS-5 OBP prompt settles at `pc=0xffd27424`, `npc=0xffd27428` in
`local/probes/obp-ok-ss5/library-trace.log`. The prompt is a serial-polling
loop. It is identified for measurement, but the initial parking slice must
**not** park it: the plan excludes PROM polling, and OBP's input path is
required for reset, settings and recovery. A later opt-in slice could consider
it after a separate event/wake audit.

### Solaris 2.6 serial idle

The SS-20 serial-idle sample attributes 98,494 of 100,000 sampled PCs to
`idle` (starts at `f006fa08`) and `generic_idle_cpu` (starts at `f0071708`);
these addresses were checked against the guest ELF. The SS-5 idle sample shows
a different set of hot PCs, including `f006fa54`, `f006fa64`, `f011b3fc`,
`f006fa34`, `f011b40c`, `f006fa38`, `f006fa40`, `f006fa60`, `f011b408`,
`f006fa2c`, `f011b3f8`, `f011b400`, `f00f1620`, `f006fa24`, `f006fa58` and
`f006fa5c`. The `f011b...`/`f00f...` paths were not assigned reliable symbols.

The SS-20 idle window executed 204,796,545 instructions, took 2,057 interrupts
and 1,398 synchronous traps in 204,800,000 steps. The SS-5 baseline recorded
2,372 interrupts and 32,475 traps in its idle window. These deltas prohibit
treating every idle sample as a retired instruction or treating parked time as
execution.

### Why repeated PCs are insufficient

A PC ring can be entered by ordinary user spin loops, kernel polling, masked
interrupt waits, active DMA completion loops and PROM input polling. Eligibility
must also check at least:

- supervisor mode and valid PC/nPC alignment;
- PSR.ET, PIL and pending interrupt levels;
- scheduler runnable/eligible state for every CPU;
- no due device deadline, active DMA/SCSI/network transaction or pending
  interrupt that needs CPU service;
- no reset/pause/shutdown request;
- a stable, multi-PC loop over a warmup window, with distinct PCs and no
  instructions with observable side effects in the candidate cluster;
- known image/profile signature or a conservative unclassified fallback.

Unknown firmware or OS versions fall back to execution.

## Wakeup sources to preserve

The parking contract covers every source that can make an idle machine
runnable. The current device advance path is in `SystemBus::advance_devices_ns`
and already advances devices without executing a CPU.

| Source | Evidence / deadline owner | Required wake behavior |
| --- | --- | --- |
| System/processor/user timers | `src/devices/timer.rs`, 500 ns tick phase | advance to the next counter overflow; deliver timer pending/IRQ |
| RTC/NVRAM clock | `src/devices/nvram.rs`, NVRAM seconds and fractional phase | preserve RTC seconds and read/stop semantics |
| CGsix retrace/blink | `src/devices/cgsix.rs`, frame/blink counter | advance scanout generation; wake if guest or frontend needs retrace |
| CG14/SX retrace | `src/devices/sx.rs`, `retrace_phase_ns` | advance retrace phase and pending IRQ |
| Keyboard/mouse | `src/devices/keyboard.rs`, `src/devices/mouse.rs` | byte arrival and queued input wake the worker |
| SCC serial A/B | `src/devices/serial.rs` | receive/transmit progress and console input |
| Ethernet/NAT | `src/devices/ethernet.rs`, `src/net/` | RX/TX, DMA and host network progress |
| ESP/SCSI and DMA2 | `src/devices/esp.rs`, SCSI DMA | command, selection timeout, DMA request/completion |
| Floppy | `src/devices/floppy.rs` | its functional deadline |
| SMP IPI/peer wake | scheduler, `SystemBus::interrupt_levels_for` | no CPU may be parked while another CPU needs it; IPI wakes |
| Reset/pause/quit/frontend input | Heliacal control and serial input paths | immediate wake; no device time is skipped |

The existing `Machine::advance_ns` API advances all device clocks and the
scheduler's external virtual time without executing a CPU. It is the base for
a parked interval, but it does not yet expose a query for the earliest due
event. A deadline query/earliest-event API is required before parking; a
bounded recheck must be used where a device has no exact deadline.

## Virtual-time contract

1. The core owns `virtual_ns`, device state and event ordering. The frontend
   owns the host thread sleep/condvar/channel implementation.
2. Parking must advance device time by an explicit virtual interval. It must
   not set timers to a stopped state and must not freeze the RTC, retrace,
   serial, DMA or network progress.
3. No interval may cross a due event. Advance to the earlier of the requested
   interval and the next event, service the event, then return control. If the
   event raises an interrupt or makes a CPU runnable, the core reports that and
   the frontend must not resume sleeping.
4. A parked CPU retains its PC/nPC, window state, PSR and MMU state. Parking is
   not a CPU step, trap, interrupt or retired instruction. On wake, execution
   continues at the same boundary.
5. Device time advanced while parked is observable in counters, RTC, serial
   and timer state. Acceptance is event-boundary equivalence, not equality of
   retired-instruction counts during idle.
6. The host wait duration is a frontend policy. It must be derived from the
   core's virtual deadline with a bounded conversion/recheck, because fast mode
   is not a real-time guest clock. Early input must cancel the wait without
   advancing virtual time past the input timestamp.
7. The core must never sleep, block on a host primitive, or depend on a UI
   thread. The frontend must never call a host sleep while the core reports an
   unserviced due event or a runnable CPU.
8. The announce/check/wait sequence must be race-safe: input, network
   progress, pause/reset and shutdown signal the worker; generation/version
   checks prevent a lost wake between the eligibility query and the wait.

## Proposed core/frontend interface

Names are placeholders; semantics are the review subject.

```rust
/// A conservative, generation-checked parking opportunity.
pub struct IdleHint {
    pub eligible: bool,
    pub virtual_now_ns: u64,
    pub next_event_ns: Option<u64>,
    pub wake_reasons: WakeReasons,
    pub generation: u64,
}

impl Machine {
    /// Does not block, sleep or mutate guest execution state.
    pub fn idle_hint(&mut self) -> IdleHint;

    /// Advance device time by an explicit virtual interval while the CPU is
    /// parked. Returns the actual advancement and any newly ready wake reason.
    pub fn advance_idle_ns(&mut self, max_ns: u64) -> IdleAdvance;
}
```

`idle_hint` is queried only after the core has classified the current loop and
confirmed no runnable/eligible CPU or due event. `advance_idle_ns` must stop at
the earliest device event and report whether an interrupt, runnable CPU,
serial/network input, reset/pause or shutdown needs frontend action. The core
returns; it never performs the host wait. Heliacal converts the hint into its
own condvar/channel deadline, wakes the worker on input/control events, then
calls `advance_idle_ns` for the elapsed virtual interval and re-queries the
hint before sleeping again.

For SMP, any eligible/runnable processor disables parking for the whole machine.
An IPI, peer wake or shared-device event must break the wait. The first
implementation should be single-CPU and opt-in; SMP parking needs its own
acceptance tests.

## Classification policy for the first implementation

- Opt-in only; disabled by default and disabled in all existing demos.
- Start with a known Solaris kernel idle signature plus validated
  supervisor/PSR/PIL/PC/nPC and scheduler/device guards. Keep the signature
  data in a small profile registry, not scattered constants.
- Require warmup and multiple distinct PCs; do not classify a single repeated
  PC.
- Reject ordinary user loops, masked-interrupt waits, active DMA, SCSI,
  Ethernet/NAT and PROM polling.
- Fall back to execution for unknown firmware/OS images.
- Record accepted/rejected/near-miss candidates and wake reasons in opt-in
  diagnostics, without affecting guest behavior.

## Measurement and acceptance plan

Required before a performance claim:

1. Idle CPU: the same frontend sampling method as Slice A, at OBP and Solaris
   idle, with parking off/on. Report worker, main and paused percentages and
   retained raw traces.
2. Input latency: direct end-to-end PTY timing at the OBP prompt and in a
   Solaris shell. Report median/p95 over repeated trials, injection timestamp,
   first response byte and input queue/worker timing. Reuse the review's
   checked launch setup and record the exact linked build.
3. Active work: fixed ALU, memory, full-device and MMU numbers with parking
   disabled, plus a representative boot/login/idle workload; no boot slowdown
   beyond baseline spread.
4. Differential on/off tests at event boundaries: timers, RTC, retrace,
   serial, Ethernet/NAT, ESP/SCSI/DMA, reset/pause/quit and SMP IPI.
5. Negative fixtures: ordinary spin loop, masked interrupt wait, active DMA,
   PROM polling and unclassified kernel loop must not park.
6. Sustained Solaris idle/login/X and clean halt on SS-5, SS-20 and Axil; no
   stopped timer, stuck input, missed serial/network event or lost wake.

Proposed target remains **below 10–15% of one CPU while parked**, input latency
**below 50 ms**, and no measurable boot regression beyond baseline spread. A
parking result must report parked time and wake reasons; these are goals, not
results already achieved.

## Open decisions before implementation

- Exact host-wait/virtual-time conversion and whether fast mode should pace or
  only preserve event ordering.
- Where the earliest-event query lives and how devices without an exact
  deadline are bounded.
- Whether `Machine::advance_ns` is sufficient or a dedicated parked-advance
  method is needed to guarantee "stop before event" and service atomicity.
- How input and network progress signal a sleeping worker without adding a
  polling thread or changing channel semantics.
- Whether the initial classifier may park only Solaris `idle`, or also the
  generic `generic_idle_cpu` path; OBP remains excluded.
- Scope of the first slice: UP only, with SMP left behind its own gate.

## References

- `clanker/performance-plan.md`: issue #5 plan, idle evidence, wake sources and
  proposed targets.
- `clanker/performance-issue5-slice-a.md`: corrected host CPU baseline.
- `clanker/performance-issue5-device-service.md`: accepted device-service
  guards and timing context.
- `local/performance-2026-11-13/ss5-baseline.log`,
  `local/performance-2026-11-13/ss20-baseline.log`, associated reports and
  `ss5-unix.elf`/`ss20-desktop.elf`.
- `local/probes/obp-ok-ss5/library-trace.log`,
  `local/probes/obp-ok-ss5/configuration.json`.
- `local/issue5-idle-2026-11-17/measure.py`, `idle.cfg` and the failed
  measurement logs: not yet a successful frontend latency baseline.
- `src/machine.rs` (`advance_ns`, scheduler and step paths),
  `src/system_bus.rs` (`advance_devices_ns`), `src/devices/timer.rs`, and the
  device modules listed above.
- `heliacal/src/emulator.rs:556`, `:690`, `:887`: current worker batch and
  input/network polling boundaries.
