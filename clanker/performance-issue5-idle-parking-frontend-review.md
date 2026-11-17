# Issue #5: opt-in Heliacal parking review

Final re-review 2026-11-17 against Helios `e3d87e3` and the uncommitted Heliacal
`src/emulator.rs`, `src/main.rs`, `src/serial_pty.rs` and `src/settings.rs`
changes. The accepted core checkpoint is unchanged.
**Accepted as a default-off experimental implementation checkpoint. The
previous blocking behavior and Clippy findings are resolved.**
Keep parking default-off and issue #5 open. This review makes no production
changes, commits or pushes.

## Final re-review: accepted checkpoint

The actual worker now drains SCC output through the shared `drain_console`
helper before preparing the token. The cheap PC/nPC/PSR candidate check stays
before that extra drain. Both the normal batch and parking paths retain the
same PTY, transcript and embedded-console processing. The final independent
worker fixture observes:

```text
output-at-wait=[] final-serial="Z" steps=2048
```

All **14/14 independent protocol fixtures pass** after adapting only their
calls to the new `ParkContext` API. They cover wait-before-commit, short
deadlines, early/zero elapsed and oversleep, BREAK, queued and during-wait
commands/input, actual PTY input/backpressure, a localhost UDP reply during
the wait, inactive networking, reset-stale tokens and actual-worker output
draining. The observation hooks do not alter admission or commit decisions.

Independent validation of the submitted source also passes:

- **114/114 frontend tests**, including the five new parking source tests.
- Formatting and the exact prescribed Clippy command with `-D warnings`.
- SDL dummy/software smoke and release build.
- Disk-free original SS-20 OBP/SX display smoke with `--idle-parking`:
  55,758,848 outcomes, exit 0. This is a firmware non-parking control, not a
  fresh Solaris or desktop acceptance run.

The new `src/emulator/parking_tests.rs` is currently untracked; include it in
the checkpoint commit. The broader independent cases are retained below for
future promotion when extending this protocol. In particular, preserve the
actual-worker ordering case and during-wait arrival cases, not only tests
which call the output helper directly.

Evidence: `local/review-issue5-parking-frontend-2026-11-17/recheck2/`, including
`standard-validation.log/json`, `review-fixtures.log`, `release-smoke.log`,
source hashes/diff, the added source-test file, reproducible review fixtures
and reconstruction scripts. Production source hashes were unchanged throughout
review. Temporary build and source copies are removed after validation;
compact evidence is retained. No guest disks or live settings were used.

### Performance and documentation scope

The quoted **24.79% CPU and 11.36/37.64 ms PTY median/p95 are from the preceding
wait-first candidate**, before the final output-drain/context/test changes.
They were not rerun for this revision, and the installed binary hash now
differs from that measurement's manifest. They remain useful earlier evidence,
not a final matching off/on result or proof that the output change leaves
latency unchanged. The older off arm was also from an earlier binary.

The next performance slice should make a fresh matching off/on comparison
from identical disposable disk/NVRAM starts, with immutable per-run manifests
and transcripts, multiple idle windows, guest-verified input and clean halt.
Record wait/oversleep time, parked virtual time, token/proof cost, rejection
and wake reasons before tuning the remaining CPU use or latency tail. Keep
SMP and unknown loops excluded and do not relax the accepted core proof.

Before final merge, document the experimental flag in `--help` and human docs.
It is a boot-time-only setting; the production wait is bounded polling, not
notification-driven wakeup. Network service occurs in the outer loop before
the batch and after a wait, not inside `prepare_park`. The core checkpoint
remains accepted; this frontend acceptance does not complete issue #5 or claim
the 10–15% CPU target has been reached.

## Previous correction re-review (historical)

The prepare/wait/recheck/commit change fixes the original advance-before-wait
and short-deadline problems. Measured elapsed time is capped at the planned
budget, and early return/zero elapsed/oversleep behave as documented. Queued
commands remain stashed, GUI/PTY input cancels advancement, and pending or
active BREAK rejects admission. An actual localhost UDP reply during the wait
also cancels advancement; an inactive NAT engine does not prevent parking.

The new independent suite passes **13/14 tests**. The remaining failure runs
the actual worker, with a fully transmitted external SCC byte `Z` queued one
instruction before the accepted Solaris boundary. It observes:

```text
output-at-wait=[90] final-serial="Z" steps=2048
```

The worker begins the host wait while the byte is still in the core SCC output
queue, then publishes it afterward. The fixture explicitly disables SCC reset
loopback and verifies that the byte is queued before starting the worker. It
records a clone of the SCC at the wait call, leaving live state unchanged.
It sets the review abort flag only after making that observation so the
synthetic worker terminates; this is not a lost-byte claim.

### Remaining behavior correction: drain core output before parking

`host_work_pending` at `../heliacal/src/emulator.rs:672` checks the PTY's
buffers, but guest bytes are transferred from the core SCC into those buffers
only after the CPU batch at line 1089. The parking attempt at line 1061 comes
first. Therefore a guest that prints and returns to idle can incur a sleep
before its echo, response or prompt is delivered. The new PTY output check
does not cover that upstream queue.

Factor the existing SCC-to-PTY/internal-console drain into one helper and run
it before preparing a parking token as well as at the normal batch boundary.
Keep the cheap PC/nPC/PSR candidate check before this extra drain so active
execution does not acquire a per-instruction output-processing cost.
Preserve the transcript, console parser, waiting-for-console flag and redraw
behavior. Poll/flush the PTY before sleeping and let real output backpressure
reject parking. Do not consume output merely to check whether it exists and
then discard it. This can be fixed in the frontend without altering the
accepted core proof. Promote the worker-output fixture against that helper.

This establishes an avoidable delay of a requested park interval, potentially
longer under host oversleep. It does **not** establish that the delay explains
the entire observed 37.64 ms p95; that needs fresh measurement and wait/output
timestamps.

### Required validation correction: Clippy currently fails

An independent run of the prescribed Clippy command fails with five diagnostics:

- `commit_park`, line 739: 9 arguments, threshold 7.
- `commit_park_with`, line 766: 11 arguments, threshold 7.
- Line 761: redundant closure around `thread::sleep`.
- Lines 1061 and 1062: two collapsible `if` statements.

Group the borrowed host parking state into a small context rather than
duplicating the long parameter list, pass `thread::sleep` directly, and
simplify the conditional flow. Rerun the exact required command with
`-D warnings`; the supplied corrected validation log contains tests and build
results, but no successful Clippy run. Do not describe the candidate as Clippy
clean until this check passes.

### Promote the protocol tests and complete the small handoff items

The production frontend still has 109 tests, including only the two new
setting/CLI cases. None of the corrected parking protocol cases is tracked.
Promote the new clock/wait seam coverage: bounded elapsed time, short deadlines,
commands before/during wait, GUI/PTY input, BREAK, output backpressure, network
arrival and stale/reset plans. Include the corrected output-delivery case.
Retain the previously requested worker replacement/epoch checks, and document
the experimental flag in `--help` and human docs, including its boot-time-only
application and exact single-CPU SS-20/Solaris scope.

The new helper checks NAT progress **after** waiting. Before waiting it relies
on the outer loop's earlier network service; `prepare_park` itself has no NAT
argument or service call. Describe this accurately as bounded host polling,
or add a pre-wait network readiness/service check before issuing the token.
Do not claim a notification-based early wake: the production wait is still a
bounded `thread::sleep`. Its after-wait cancellation now preserves guest time.

### Corrected measurements and retained evidence

The corrected raw ticks and timestamps reproduce **24.7852%** worker CPU in
one 5.003 s window, and **11.3617 / 37.6368 ms** median/p95 over 20 guest-verified
PTY transactions. The corrected installed binary matches the latest retained
`inputs.json`. The previous off result is 99.1706% and 11.1566 / 12.9559 ms;
it was not rerun against this corrected binary. The on p95 is below the
proposed 50 ms target in this small sample; the CPU percentage remains above
the proposed 10–15% target. Neither is grounds to relax the correctness gates.

Both measurement scripts still overwrite their shared config, manifest and
transcript paths. Use the separate run directories and fresh clones requested
below for the next matching comparison. Record actual wait/oversleep durations
and input/output timestamps to investigate the latency tail. Do not attribute
the whole tail to the console-output finding without evidence.

New review evidence is under
`local/review-issue5-parking-frontend-2026-11-17/recheck/`:

- `parking_recheck.rs`, `instrumentation.patch`, `review-fixtures.log`:
  13 passing cases and the one actual-worker output failure. Instrumentation
  only seeds the synthetic worker and observes the point just before waiting.
- `standard-validation.log/json`: **109 tests, fmt and SDL smoke pass;
  Clippy fails as listed above.** Core production source is unchanged.
- `release-smoke.log`: independent release build and disk-free original OBP
  SS-20/SX display smoke with parking enabled pass (55,928,832 outcomes,
  exit 0). This is a non-parking firmware control, not a fresh Solaris run.
- `candidate-hashes.json`, `heliacal.diff`, `revisions.txt`: exact reviewed
  source, verified unchanged in the production checkout throughout review.

Return these frontend corrections uncommitted. The accepted core token and
its earlier validation remain accepted. The initial review below is retained
as history; its clock-ordering and BREAK failures are now resolved.

## Initial review (historical)

The real Solaris core admission milestone is supported by the retained
`local/issue5-parking-real-2026-11-17/run.log`: 20 admissions, 20 zero-time
advances, preserved CPU/MMU/RAM, and deadlines about 3.2 ms away. The separate
`headless.log` and overwritten `real-idle.json` describe a later 5/5 probe.
Neither probe advances a positive interval or tests the frontend wait protocol.

## 1. Commit parked time after a defined host wait

`../heliacal/src/emulator.rs:558` calls `advance_parked` immediately. The
worker at line 936 then sleeps for that already-committed virtual interval,
but only if the advance did not reach a service boundary. Consequently:

- A normal opportunity commits 2,000,000 ns before any host wait occurs.
  An early host event cannot reduce or revoke that interval.
- A deadline within the 2 ms budget advances immediately, with no host wait
  at all. The deterministic 1 ms timer fixture advances from 100 ns to
  1,000,000 ns and records zero waits.
- The code reads no monotonic elapsed time around this wait. Host oversleep,
  early notification and the remaining interval after a deadline have no
  explicit conversion policy.

This does not satisfy the clock-policy requirement in the
[design review](performance-issue5-idle-parking-review.md#2-specify-a-real-timebase-and-deadline-contract).
Core clipping is correct; it cannot decide when host time should be committed.
Reaching a future service boundary must not itself erase the wait preceding
that boundary.

Refactor the frontend into prepare/wait/recheck/commit phases with an injectable
monotonic clock and waiter. A suitable first policy is 1:1 only while parked:
anchor host and virtual time, wait at most the earlier of the 2 ms polling cap
and the first core deadline, then derive an explicit virtual delta from the
observed elapsed time. A deadline already due requires normal service without
sleep. At the first reached deadline, return to normal CPU/device service;
never advance across several events to catch up after host oversleep. Document
whether excess elapsed time is discarded or retained, and reset any anchor on
pause, reset, replacement and fallback to normal stepping.

## 2. Host work needs its own admission and wake protocol

The core token cannot inspect the command receiver, GUI input mutex, PTY
buffers or host network readiness. Those are polled outside the CPU batch.
The parking call after a CPU step does not recheck any of them.

Review-only hooks insert work at that exact point, after outer polling and
before the parking call. Both a queued `Command::Pause` and a queued embedded
serial byte remain pending while the worker commits 2 ms and requests a 2 ms
sleep. The same byte already delivered into an interrupt-enabled SCC prevents
parking, proving that the missing guard is at the frontend boundary.

Add a host-work check before waiting and after waking, covering commands,
abort/shutdown, GUI serial/keyboard/paste/mouse queues, pending keyboard
delivery, PTY input/output and host network service. Preserve commands received
while waiting for dispatch; do not consume them as mere wake notifications.
Do not hold the input mutex during a wait. Use notification/generation checks
for the queues, or an explicitly documented bounded polling policy for sources
without a wake FD. The 2 ms bound applies to a requested polling interval, not
to total command round-trip latency or to host scheduler oversleep.

Stage/check host events before committing virtual time. Pause, shutdown and
replacement must cancel the opportunity. Specify serial/network delivery
ordering at the observed wake time. A conservative first version may cancel
the opportunity and resume normal stepping when host work is found, provided
the treatment of elapsed host time is documented. Do not deliver an event into
the core and then attempt to consume the now-stale token.

Add deterministic cases for work already queued, arrival between the final
check and wait, arrival during the wait, and reset/replacement with an old
notification. Exercise both GUI and PTY input, network progress, output
backpressure and shutdown. The existing receiver/input epochs must keep old
guest input out of the replacement machine.

## 3. Reject pending and active serial BREAK

`SerialBridge::service` releases BREAK after one million execution outcomes
(`../heliacal/src/emulator.rs:646`). Parked time does not increment that count.
The new call has no `break_pending` or `break_active` check.

Both states reproduce admission and a 2 ms advance in the actual worker.
The active case has an enabled receiver and a held BREAK with no SCC interrupt
pending; an acknowledged or masked external-status interrupt does not make
the held input pulse disappear. Thus the core IRQ check cannot replace the
frontend guard. Repeated parking can greatly extend the outcome-count-based
hold and postpone subsequent serial bytes.

For this slice, reject parking while either bridge flag is set. Preserve the
accepted BREAK delivery/release implementation and its backpressure retry;
changing BREAK to a different timing model would be separate work.

## Independent fixtures and validation

Evidence is in `local/review-issue5-parking-frontend-2026-11-17/`:

- `idle_review.rs`: eight deterministic worker fixtures, using the accepted
  reconstructed Solaris loop from `tests/support/mod.rs` and no guest disks.
- `instrumentation.patch`: test-only hooks in an isolated Heliacal copy. They
  seed one instruction before the boundary, inject host work after polling,
  and record the actual wait call instead of sleeping. They do not replace
  the worker's admission, advance, wait condition or command/input logic.
- `review-fixtures.log`: **2 pass / 6 fail** on the candidate. The passing
  controls are parking disabled and an IRQ-producing byte already in the SCC.
  Failures cover advance-before-wait, the short deadline, queued Pause, queued
  GUI serial input, pending BREAK and active BREAK. Promote the semantic cases
  using the new clock/wait test interface; the hooks are review instrumentation,
  not a production implementation.
- `candidate-hashes.json`, `heliacal.diff` and `revisions.txt`: reviewed inputs.
- `standard-validation.log`: independent 109/109 existing frontend tests,
  formatting, Clippy with `-D warnings`, and SDL dummy/software smoke all pass.
- `release-smoke.log`: release build and disk-free original SS-20 OBP/SX
  display smoke with `--idle-parking` pass (57,554,944 outcomes, exit 0).
  This exercises the non-parking firmware path, not an OS or desktop workload.

The standard tests contain two new setting/CLI tests but no tests of the new
worker parking behavior. Passing them does not resolve the six review failures.
Core source is unchanged; the accepted 781/784 unique core test results are
not represented as a fresh full core rerun in this review.

## Measurement and documentation follow-up

The retained raw thread ticks and transaction timestamps reproduce the reported
arithmetic:

| Headless Solaris observation | Off | On |
| --- | ---: | ---: |
| Worker CPU, one approximately 5 s window | 99.1706% | 38.5855% |
| PTY command round trip, 20 trials, median | 11.1566 ms | 14.2655 ms |
| Same trials, nearest-rank p95 | 12.9559 ms | 19.7242 ms |

This is useful preliminary evidence. The latency increase is about 3.1 ms
at the median and 6.8 ms at p95. The worker target remains unmet, as the handoff
already says. Fix timing/input correctness before tuning token cost or sleep
length to improve the CPU percentage.

The off/on scripts write the same `inputs.json`, config, serial transcript,
screenshots and launch log. Only the event, CPU and latency JSON files are
retained separately for both arms. The cited `run-noop-final.log` is absent;
the available `events-noop.json` does record exit 0. The current installed
binary hash also differs from the single retained manifest. These facts limit
independent reconstruction of the matching runs; they do not invalidate the
tick/timestamp arithmetic above.

After the correction, use separate immutable run directories with both source
revisions/diffs, features/build settings, binary and initial disk/NVRAM hashes,
config, full serial transcript, phase markers and exit status. Start each arm
from its own fresh disposable clone. Retain multiple idle windows and guest-
verified input trials, parked virtual/host durations, wait/oversleep counts,
wake reasons and proof-rejection categories. Include early input, serial BREAK,
pause/resume, reset/replacement and shutdown. Keep OBP, other profiles and SMP
as non-parking controls; do not claim desktop or physical input acceptance from
this headless result.

Also document `--idle-parking` in `--help` and the human docs, its default-off
and exact SS-20/UP/Solaris scope, time policy, and when configuration takes
effect. Currently the worker reads the setting only on successful construction
of a new machine; it is not a live toggle merely because `hardware_eq` excludes
it. Explicit boot-time-only behavior is sufficient for this experimental slice.

Return the corrected frontend uncommitted with these checks. Do not change the
accepted core proof, relax admission, enable by default or broaden to SMP as
part of this correction.
