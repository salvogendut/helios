# Issue #5: time/deadline foundations and Solaris idle-loop proof

Status: corrected foundation independently accepted, ready to commit. **No sleeps, parking
token, active idle recognizer, JIT change, WP/TRCR change or frontend wait
policy has been added.** Issue #5 remains open. The controlling review handoff
is
[`performance-issue5-idle-parking-foundation-review.md`](performance-issue5-idle-parking-foundation-review.md);
the earlier design review remains
[`performance-issue5-idle-parking-review.md`](performance-issue5-idle-parking-review.md).

The four query corrections identified by the independent review are applied
and its ten fixtures are promoted below. Full standard validation passes independently, and
an additional disposable Solaris run confirms complete CPU/MMU/FPU and RAM
invariance across four observed iterations followed by clean halt. Parking
implementation remains outside this slice.

Worked from Helios `e72304c` plus the uncommitted source changes below, and
Heliacal `2d07819`. The source media, firmware and accepted NVRAM images were
not modified.

## Scope

This slice establishes one absolute logical device clock and a read-only
deadline query, including the CPU fast path's deferred device balance. It also
retains one million executed steps of Solaris SS-20 serial-idle execution and
identifies a conditional PC/nPC loop boundary. Acceptance covers these
foundations; token validation, parked advancement and host waiting remain
separate work.

## Clock and deadline foundation

The scheduler's eligible/runnable state is deliberately separate from any idle
proof. In the Solaris trace below, processor 0 remains arbitration-enabled and
the scheduler can still report it ready while the guest executes the idle loop.
`Timebase.next_cpu_ready_ns` is a deadline, not an idle classification, and
`SchedulerError::NoRunnable` is not used as an idle signal. No run-state or AER
semantics were changed.

The public query is `Machine::timebase()` (`src/machine.rs:1200`):

```rust
pub struct Timebase {
    pub now_ns: u64,
    pub deferred_ns: u64,
    pub next_device_ns: Option<u64>,
    pub next_cpu_ready_ns: Option<u64>,
    pub next_ns: Option<u64>,
}
```

- `now_ns` is `SystemBus::logical_ns`, the total time submitted by direct and
  scheduled execution. It includes the CPU fast path's deferred `device_ns`
  balance. `deferred_ns` is the portion not yet applied to device registers.
- `next_device_ns` is the earliest known device event at or after `now_ns`.
  A deadline already covered by deferred time is returned as `now_ns` rather
  than as a past time. Queries do not flush, acknowledge or mutate anything.
- `next_cpu_ready_ns` is the scheduler's earliest eligible deadline only while
  the scheduler has explicitly claimed execution-path ownership. The direct
  `step` path and fresh/reset machines report `None`; clock equality is not
  ownership and the scheduler deadline is not an idle-classification result.
- `next_ns` is the minimum of the two categories.

`Machine::virtual_ns()` exposes the same logical clock. `SystemBus` also exposes
`logical_ns()`, `deferred_ns()`, `flush_device_time()` and
`next_device_deadline_ns()` (`src/system_bus.rs:1137`). `advance_ns` now applies
deferred CPU-path time before its new interval, so device ordering cannot move
backwards. `advance_seconds` does the same and advances the logical clock with
saturating arithmetic. `SystemBusState` carries `logical_ns` for differential
snapshots.

Deadline coverage was added to the existing device models without changing
their execution paths:

| Device/time owner | Query |
| --- | --- |
| timers and processor/user-timer banks | `src/devices/timer.rs:217` |
| CGsix retrace/blink | `src/devices/cgsix.rs:187` |
| SX/CG14 retrace | `src/devices/sx.rs:165` |
| Ethernet poll/active transfer | `src/devices/ethernet.rs:243` |
| ESP selection/reset pulse/active phase | `src/devices/esp.rs:172` |
| floppy polling/seek completion | `src/devices/floppy.rs:120` |
| keyboard/mouse byte/backpressure queues | `src/devices/keyboard.rs:94`, `src/devices/mouse.rs:56` |
| RTC second boundary | `src/devices/nvram.rs` clock state + `Sun4mDevices` phase |

The 2 microsecond CPU coalescing balance is included in the conversion from a
relative device deadline to an absolute one. It is not itself treated as an
event. The query remains read-only even when a timer or DMA condition is
already pending. DMA/IRQ service is not performed by the observation itself;
the caller must reach the normal boundary and give the CPU its service
opportunity.

No parking token, generation check or frontend wait protocol is present in this
slice. The speculative `IdleHint`/`advance_idle_ns` interface from the earlier
handoff is therefore not implemented or validated here.

## Review corrections applied

1. **CGsix blanking.** The query now uses `VBLANK_UNITS` (the 900 visible
   lines) as the blanking origin, matching the existing `advance_ns` latch.
   It returns the frame-end/blink boundary after an in-blanking
   acknowledgement and the next blanking boundary after the next frame. The
   three independent CGsix fixtures pass.
2. **Blocked keyboard/mouse input.** A blocked layout command no longer
   reports zero time. Keyboard queue backpressure reports the next 1200-baud
   byte interval, and mouse motion reports the next byte interval while its
   packet queue cannot accept another five-byte packet. Read-only SCC peeking
   distinguishes a serviceable command from a blocked one without changing
   packet/baud/protocol behavior.
3. **ESP reset pulse.** The outstanding `reset_ns` duration is included in
   `Esp::next_deadline_ns`, including when reset IRQ notification is disabled.
   The 26,000 ns pulse expiry fixture passes without changing reset timing or
   IRQ semantics.
4. **Scheduler ownership.** `Machine` now records explicit
   `step_scheduled` ownership. A fresh, reset or direct-`step` machine reports
   `next_cpu_ready_ns = None`; clock equality is not ownership. The direct
   `step` hot path is unchanged.

The ten independent fixtures from
`local/review-issue5-foundation-2026-11-17/tests/deadlines.rs` are promoted as
`tests/timebase_deadlines.rs`; all **10/10 pass**. Two additional focused tests
cover scheduler-ownership reset/direct behavior and a keyboard queue whose SCC
receiver cannot currently accept a byte.

## Tests added

`tests/timebase.rs` covers:

1. A timer deadline after a 100 ns fast-path step includes the deferred
   balance; repeated queries leave the diagnostic snapshot unchanged; advancing
   to the event leaves the timer pending and then acknowledged by the normal
   boundary.
2. Explicit `advance_ns` flushes earlier deferred time before the new interval
   and preserves the 500 ns timer phase/count.
3. Both SS-20/Axil scheduled CPUs share one absolute timebase through 1,500
   cycles each, including fractional 75 MHz phases; every query matches
   `SchedulerStats::virtual_ns` and the returned `ScheduledStep`.
4. The direct UP path reports logical time without a scheduler deadline.
5. Reset flushes pending time and clears the logical clock.
6. `tests/device_service_review.rs` adds a query before the held SCSI DMA-reset
   serial fixture and verifies that the query leaves the snapshot unchanged and
   the subsequent CPU boundary identical to the reference unconditionally
   serviced machine.
7. `tests/timebase_deadlines.rs` promotes the review's ten independent
   fixtures: CGsix blanking/rearming, keyboard and mouse backpressure, ESP
   reset-pulse expiry, RTC phase/stop, user-timer fractions and UP PROM time.
8. Ownership, reset and blocked-SCC focused tests are in `tests/timebase.rs`.

The existing differential test now accepts `bus.logical` as a timing change.

## Solaris idle-loop trace

The trace was captured with the original SS-20 OBP, Solaris 2.6 disk and
`ss20-ttya.nvram` on the current foundation source. The guest reached `login:`,
logged in as root, ran `uname -a`, printed
`HELIOS-IDLE-TRACE`, settled for 100M steps, and then executed 1,000,000 traced
steps. The trace contains 999,981 executed outcomes, 9 synchronous traps and 10
interrupt entries. The stable loop was entered 70,868 times.

Kernel symbol evidence: `local/performance-2026-11-13/ss5-unix.elf`
(`sha256 9bf149089c59e702cefaf775b8b9d569845d98b205b09524975beee7c3ff0c98`)
places `idle` at `f006fa08` and `generic_idle_cpu` at `f0071708`.

Complete observed stable iteration, starting at the branch-target boundary:

| step | PC | nPC | instruction | disassembly |
| ---: | ---: | ---: | ---: | --- |
| 9 | `f006fa24` | `f006fa28` | `d00220ec` | `ld [%o0+0xec], %o0` |
| 10 | `f006fa28` | `f006fa2c` | `80a22001` | `cmp %o0, 1` |
| 11 | `f006fa2c` | `f006fa30` | `3280000f` | `bne,a f006fa68` (not taken) |
| 12 | `f006fa34` | `f006fa38` | `d0062058` | `ld [%i0+0x58], %o0` |
| 13 | `f006fa38` | `f006fa3c` | `80a22000` | `cmp %o0, 0` |
| 14 | `f006fa3c` | `f006fa40` | `02800006` | `be f006fa54` (taken) |
| 15 | `f006fa40` | `f006fa54` | `213c097e` | `sethi ..., %l0` (delay) |
| 16 | `f006fa54` | `f006fa58` | `e00421e8` | `ld [%l0+0x1e8], %l0` |
| 17 | `f006fa58` | `f006fa5c` | `9fc40000` | `call %l0` |
| 18 | `f006fa5c` | `f0071708` | `01000000` | `nop` (delay) |
| 19 | `f0071708` | `f007170c` | `81c3e008` | `retl` |
| 20 | `f007170c` | `f006fa60` | `01000000` | `nop` |
| 21 | `f006fa60` | `f006fa64` | `10bffff1` | `ba f006fa24` |
| 22 | `f006fa64` | `f006fa24` | `113c097e` | `sethi ..., %o0` (delay) |

The stable loop boundary is therefore **`f006fa24` / `f006fa28`**. The
annulled `f006fa30` load is not executed on this path. The `f006fa44` call and
the `f006fa68` exit path are not executed. The boundary loop contains no store
instruction; it reads three guest addresses and calls the loaded
`generic_idle_cpu` pointer, which is `retl`/`nop`. The idle-entry counter store
at `f006fa18` is outside the boundary and is not part of the skipped
iteration. The outer function has already performed it before reaching the
loop.

The boundary is conditional, not a generic idle proof. It requires the observed
`PSR 404000e0`, the two loaded idle/status values to keep selecting this path,
and the `%l0` call target to remain the `retl` helper. A future recognizer must
revoke eligibility on guest/DMA writes, instruction/code changes, MMU/TLB/PTE
changes, interrupts and reset; the loads in the loop mean that memory and
translation changes cannot be ignored merely because the code bytes are stable.

Four consecutive boundary snapshots are byte-identical:
`local/issue5-idle-2026-11-17/trace/ss20-current.boundary.txt`. The full trace
is `ss20-current.trace.tsv` (1,000,000 rows, 50 MiB) and the disassembly/loop
summary is `idle-loop-summary.txt`; both are under
`local/issue5-idle-2026-11-17/trace/`.

This is **not yet an active recognizer**. The boundary is safe only under the
observed conditions: this kernel image, PSR `404000e0`, the two global/status
loads selecting the stable idle path, the `%l0` call target, and no intervening
code/mapping/PTE/guest-memory change requiring invalidation. OBP remains
excluded. A later implementation must validate a parking token and wake on
interrupts, input/PTY/network/control events, memory/MMU changes and reset; the
trace itself does not provide those mechanisms.

## Fresh frontend baseline

The final Heliacal binary was linked against this foundation source and built in
the project container with release/thin-LTO/one-codegen-unit settings:

- Heliacal source: `2d07819`
- Nucleus: current uncommitted Helios foundation on `e72304c`
- Features: `chd`, `host-icmp`
- Binary: `/var/home/salvogendut/Dev/heliacal/target/release/heliacal`
- SHA-256: `9f838d337814236f246f20c50c96358442f257f0d5ff7fa58fcc5174ff6d0a94`
- Build log: `local/issue5-idle-2026-11-17/frontend/frontend-build-corrected.log`

SS-20/one CPU/128 MiB/headless/no parking, with original OBP, Solaris 2.6 disk
copy and TTYA NVRAM:

| Measurement | Result |
| --- | ---: |
| Guest-verified OBP PTY echo, 20 trials, median | 0.892 ms |
| Same OBP trials, nearest-rank p95 | 1.128 ms |
| Solaris shell command round trip, 20 trials, median | 23.503 ms |
| Same Solaris trials, nearest-rank p95 | 28.312 ms |
| Solaris idle `helios-machine` worker | 99.34% of one host CPU |
| Solaris idle main `heliacal` thread | 0.80% of one host CPU |
| Process exit | 0 |

The Solaris latency is a `echo HZnnnn` command-level response, not a physical
key-to-pixel measurement; the token is required twice (terminal echo plus guest
output) before timing stops. OBP PTY timing uses the reviewed single-byte plus
Forth `1 2 + .` guest verification. Raw evidence and source-input hashes are
under `local/issue5-idle-2026-11-17/frontend/`, including `cpu-sample.json`,
`latency.json`, `obp-current/latency.json`, `serial.log`, `events.json`,
`inputs.json` and the corrected-run logs.

The frontend is still unparked: the worker remains at roughly one full CPU.
Solaris X/desktop idle and a parking off/on comparison remain future work.

## Active-work comparison

A matched pre-foundation worktree at `e72304c` was rebuilt after the ownership
correction and compared with the corrected tree using pinned-host, alternating
runs (CPU 2, 5,000,000 iterations): two corrected and two baseline runs per
workload.

| Workload | `e72304c` median | Corrected median | Delta |
| --- | ---: | ---: | ---: |
| ALU | 42.405 ns/step | 42.67 ns/step | +0.62% |
| Memory | 53.87 ns/step | 54.725 ns/step | +1.59% |

The ownership correction added no direct-step write. The pinned memory ranges
do not overlap (baseline 53.67–54.07; corrected 54.55–54.90 ns/step), but there
are only two samples per variant. A separate four-sample ABBA comparison gives
+2.26% ALU and -1.12% memory, showing that the observed differences are mixed.
These measurements do not prove zero overhead or a throughput improvement.
The direct `step` path remains unchanged by the ownership correction. Raw logs:
`local/issue5-idle-2026-11-17/perf/corrected-pinned/` and `perf/corrected-abba/`.

## Standard validation

All commands used the pinned nightly toolchain and `/tmp/heliacal-cargo`.

- `cargo fmt --all --check`: pass
- `cargo clippy --locked --offline --all-targets -- -D warnings`: pass
- `cargo clippy --locked --offline --all-targets --features perf-counters -- -D warnings`: pass
- `cargo test --locked --offline`: **746 successful executions (745 unique names), 0 failed**
- `cargo test --locked --offline --features perf-counters`: **749 successful executions (748 unique names), 0 failed**
- `demo`, `trap-demo`, `serial-demo`, `timer-demo`: all pass
- Heliacal release build: pass
- Fresh OBP and Solaris frontend runs: pass, status 0

Logs are under `local/issue5-idle-2026-11-17/validation/` (`test-*-final.log`,
`review-fixtures-corrected.log`, `demo-*-final.log`). Source media hashes
remain unchanged:

```
e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e  local/firmware/sun-ss5.bin
b4f15bfafa69edfcea446778bd14b61b76273e4e77ddebe3a9002af757b48ca8  local/firmware/ss10-20-v2.25r-rom
3fb9a8dbe7c8621dd9a65246ac78d9353a492c41862c1dd61656fb03d1a16259  local/firmware/turbogx.bin
ce3af8221c588383c0abfe6a7b271ec22ae6cb85bb04469d5beda9c1c90af794  local/completion-2026-11-13/desktop.raw
9ee9e7eb4448a2b873e43dfc6c6710dea1eb579d2f25ecf6a6f5b4e288dd38ea  local/ss20-ttya.nvram
```

## Questions carried into the parking design

1. The explicit `step_scheduled` ownership rule is accepted for the documented
   mutually exclusive step APIs, including fresh/reset machines and explicit
   advancement before the first step. Preserve that rule in parked advancement.
2. Are any device deadline owners missing from the table, or should the RTC
   second boundary be excluded from a future wake set because it does not raise
   an interrupt?
3. Does the idle-loop boundary evidence satisfy the required invalidation audit
   for guest memory, MMU/PTE and code-pointer changes, or is a stronger
   passive trace required before a recognizer is proposed?
4. Does the next slice need a dedicated parked-advance API that stops before
   the first event, services DMA/IRQ, and returns a wake reason, instead of
   using explicit `advance_ns` deltas?
5. The Solaris frontend baseline is serial-only. Should the next review require
   an X/desktop-idle baseline before any off/on parking comparison?
