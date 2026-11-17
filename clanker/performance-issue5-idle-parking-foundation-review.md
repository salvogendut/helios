# Issue #5 time/deadline foundation review

Reviewed 2026-11-17 on the uncommitted foundation above `e72304c`, then
independently rechecked after its four corrections. **The corrected foundation
is accepted and ready to commit.** There are no blocking findings for this
scope. Production code is unchanged by the reviewer; no commit was made.
Issue #5 remains open, and parking implementation is a separate slice.

## Correction review accepted

All ten independent fixtures were promoted byte-for-byte into
`tests/timebase_deadlines.rs` and pass in both independently rerun full suites.
The fixes address the original failures:

- CGsix uses the visible-line blanking origin and preserves acknowledgement
  and frame-wrap behavior.
- Keyboard command peeking and mouse packet-capacity checks distinguish
  blocked input from immediately serviceable work, preserving baud/protocol
  execution. A further fixture covers a disabled SCC receiver.
- ESP includes reset-pulse expiry alongside its other deadlines.
- Explicit scheduler ownership is acquired in `step_scheduled` and cleared
  by whole-machine/software reset. Construction and direct-only execution do
  not acquire it. The direct step hot path is unchanged; the existing rule
  against mixing step APIs between resets still applies.

Independent final validation: **746 normal executions / 745 unique names**,
**749 perf-counter executions / 748 unique names**, formatting, both all-target
Clippy configurations with `-D warnings`, and all four demos. Retained logs,
source hashes and cleanup records are under
`local/review-issue5-foundation-2026-11-17/accepted/`.

The corrected frontend binary/input hashes and successful OBP/Solaris run
records also check out. The earlier independent full-state loop proof below
remains evidence for the conditional boundary; the corrections introduce no
new CPU/MMU execution semantics. This recheck did not repeat that guest boot.

The fixed-work results are small and mixed. The pinned two-run medians show
+0.62% ALU / +1.59% memory; the separate four-run ABBA medians show +2.26% ALU /
-1.12% memory. These samples do not establish a throughput improvement or
prove zero overhead. They are recorded as foundation costs, with the direct
execution path preserved, rather than as a performance gain.

Commit this accepted foundation separately before the next implementation
slice. Next, settle the core token validation, first-event clipping and service
contract described below. Host sleeping and an enabled recognizer are not part
of this acceptance.

## Original findings (resolved)

The following reproductions and original validation results describe the
pre-correction candidate, retained to explain the promoted fixtures.

### 1. CGsix predicts the wrong blanking boundary

`src/devices/cgsix.rs:195` uses `FRAME_UNITS - VBLANK_UNITS` as the blanking
start. `VBLANK_UNITS` already represents the 900 visible lines. The existing
advance path latches blanking at that position, not at the 43-line blanking
duration. The new query and the running device consequently disagree:

- From reset, the query returns **599,616 ns**, but the actual pending bit
  changes at **12,550,090 ns**.
- At logical time 1,000,000 ns, the query returns **13,149,705 ns** (frame
  end), crossing that actual interrupt boundary.
- After acknowledgement inside blanking, the next frame end is correct,
  but the following blanking prediction is wrong again.

Use the same phase origin as the established device execution, retaining
integer ceiling conversion and the pending/acknowledgement policy. The three
CGsix fixtures check initial timing, no skipped interrupt, and rearming across
frame wrap. They use the documented line count and pixel rate independently
of the query's calculation.

### 2. Back-pressured input is not immediately serviceable

`Keyboard::next_deadline_ns` (`src/devices/keyboard.rs:88`) returns zero for
every pending command. A pending layout request can be blocked by the keyboard
FIFO. Repeated zero-time advances then make no progress. In the fixture, after
draining the startup ID and filling the keyboard queue, the query returns
**25,000,002 ns (now)** instead of the next byte boundary, **33,333,334 ns**.

`Mouse::next_deadline_ns` (`src/devices/mouse.rs:56`) returns zero whenever
unpacketized motion remains, even when its packet queue cannot accept more.
For a 1,000-pixel motion with a full packet queue, zero-time advancement leaves
the entire bus snapshot unchanged. The next byte is due at **8,333,333 ns**.

Distinguish work that can actually progress now from work waiting on a byte
interval or receiver/queue space. Report the baud boundary for the reproduced
cases; retain explicit handling for a disabled/full SCC receiver. Do not fix
this by changing packet generation, baud timing or keyboard protocol behavior.
A deadline-driven caller must not become stuck repeatedly servicing zero time.

### 3. ESP reset-pulse expiry is missing

`Esp::next_deadline_ns` (`src/devices/esp.rs:172`) covers selection timeout and
active work, but omits `reset_ns`. With CCF explicitly set to zero (factor 8)
and reset IRQ notification disabled, command availability changes after
**26,000 ns**. The query instead returns the RTC deadline at **1,000,000,000 ns**.

The fixture verifies that an ordinary ESP command fails at 25,999 ns and
succeeds one nanosecond later. Include the outstanding reset-pulse deadline
alongside the other ESP conditions, without changing reset timing or IRQ
semantics. This is distinct from the already-tested held DMA-reset boundary.

### 4. Clock equality does not establish scheduler ownership

`Machine::timebase` (`src/machine.rs:1198`) infers scheduled execution from
`scheduler.virtual_ns == bus.logical_ns`. Both clocks coincide at construction,
after reset and after explicit advancement before any CPU step.

For a fresh UP bare-metal machine, `advance_ns(5_000)` followed by `timebase()`
reports `next_cpu_ready_ns = Some(5_000)` although the scheduled path has never
been used. This contradicts the public `Timebase` field contract and can turn
`next_ns` into a spurious immediate CPU deadline.

Use explicit execution-path ownership, or a query API whose caller explicitly
requests scheduler deadlines. Define the pre-first-step and reset cases.
Preserve the normal direct-step hot path; do not switch UP to scheduled stepping
to repair an observational query.

## Original independent fixtures and validation

The promotable fixtures are in:

`local/review-issue5-foundation-2026-11-17/tests/deadlines.rs`

Run them from the repository root with the usual Rust environment:

```sh
cargo test --locked --offline \
  --manifest-path local/review-issue5-foundation-2026-11-17/Cargo.toml \
  --test deadlines -- --nocapture
```

Original candidate: **3/10 pass**. The seven failures were the three CGsix cases,
two input-backpressure cases, ESP pulse expiry and scheduler ownership above.
Passing controls cover RTC phase/stop, user-timer fractional overflow, and UP
CPU time including physical SS-5 PROM waits. The tests mutate only their
synthetic machines; no guest image is required.

Independent full validation also passes:

- 734 normal test executions / **733 unique names**.
- 737 perf-counter test executions / **736 unique names**.
- Formatting, both all-target Clippy configurations with `-D warnings`, and
  all four demos.

These standard tests do not cover the reproduced deadline errors. Logs and
source fingerprints are retained in `local/review-issue5-foundation-2026-11-17/`
as `fixtures.log`, `validation.log.gz`, `validation-summary.json` and
`reviewed-sources.json`.

## Solaris boundary evidence independently strengthened

The supplied million-step trace and counts check out: 999,981 executed
outcomes, 9 synchronous traps, 10 interrupts, and 70,868 entries at
`f006fa24/f006fa28`. Those entries are not a claim of 70,868 uninterrupted
complete iterations. The original four boundary records contain PC/nPC, PSR
and the active register view; they are not full CPU/MMU snapshots.

An additional independent run booted a disposable Solaris disk and copied
NVRAM, reached a root shell, settled for 100M steps, and checked **four complete
14-instruction iterations** at that boundary. Each instruction was verified
from `Step::Executed`; no extra live-MMU reads were introduced.

Each adjacent pair of full diagnostic snapshots had:

- Equal complete CPU state, including inactive windows, MMU/TLB, FPU and MXCC
  as represented by the existing architectural comparator.
- Equal RAM/DSIMM fingerprints and standalone IOMMU state.
- Exactly 1,400 ns logical advancement; differences limited to device time,
  deferred time and the corresponding device/video state.

The three observed loads were:

| Instruction PC | Effective virtual address | Loaded value |
| --- | --- | --- |
| `f006fa24` | `f025f8ec` | `00000001` |
| `f006fa34` | `f026dea0` | `00000000` |
| `f006fa54` | `f025f9e8` | `f0071708` |

The guest then completed `sync`, `halt`, and returned to OBP `ok`; the checker
exited 0. Evidence is `guest-state.log`, `boundary.serial`, `guest-inputs.json`
and the retained harness `src/main.rs` in the review directory.

This supports the proposed **conditional** boundary for the observed SS-20
Solaris kernel. It does not establish safety after memory/code/mapping changes,
for another image/profile, or for SMP. A future token still needs to validate
the read dependencies and revoke eligibility on relevant changes. Architectural
TLB/cache state is compared according to the accepted snapshot contract;
non-architectural host caches/counters are excluded by that contract.

## Baseline checks and next scope

The retained frontend binary/input hashes, 20-trial medians/p95s, process exit
statuses and fixed-work medians agree with the handoff. The Solaris measurement
is command-level PTY latency; its `write_ns` timestamp is currently sampled
just after `os.write`. Move that timestamp before the write in the next run,
and retain both if distinguishing enqueue cost. Do not relabel this as physical
keyboard or display latency. The recorded frontend process exit does not itself
prove a guest clean halt; the independent boundary run above supplies a clean
halt checkpoint separately.

The original baseline scripts and source media were left untouched. The review
preserves compact evidence while removing its own build directory, disk clone,
temporary NVRAM copies and runtime files. See `cleanup.json`.

Original work order and next scope:

1. Completed: fix the four query issues, promote the independent fixtures and
   add ownership/reset and disabled-receiver coverage.
2. Completed: update the deadline contract/docs, rerun required validation and
   report active-work measurements. The corrected foundation is accepted.
3. Next: propose a dedicated token-validated parked
   advance that clips to the first event and returns for DMA/IRQ/CPU service.
   Ordinary `advance_ns` remains an explicit interval API, not that contract.

Keep RTC phase in the time accounting even when the future frontend chooses
not to wake a CPU solely for an RTC second. Separate future timed transitions
from already-latched, possibly masked status; a masked pending bit must not
silently become the reason every graphics guest is permanently unparkable.
An X/desktop baseline belongs to graphical-parking acceptance; the first UP
serial-only opt-in prototype can use the validated serial workload. SMP and
OBP parking remain separate work.
