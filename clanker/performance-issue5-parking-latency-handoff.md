# Issue #5: locate the idle-parking command-response delay

Status: CWP checkpoint committed as Helios `3a1cf6f` and Heliacal `e8e2cfc`.
The bounded 40 ms frontend input-grace prototype is accepted for the
experimental, default-off parking checkpoint. Latency improvement, bounded
no-echo recovery and settled-idle savings are independently checked. Continuous
input can occupy one host CPU; this is the explicit responsiveness tradeoff.
Complete the documentation and small pause-error cleanup below before commit;
no further bound tuning or guest benchmark round is required for this slice.
Keep issue #5 open and parking off by default.

## Accepted baseline and remaining question

The correctly composed CWP-0–7 candidate reduces idle worker CPU from
24.29%/42.88% to 7.60%/8.40% in the two retained phases. Command-output
medians rise from 27.83/22.32 ms to 33.77/33.36 ms; p95 rises about 1.7/2.0 ms.
This cost is accepted for an explicitly enabled experimental checkpoint,
not resolved or approved for default-on use. See the
[CWP review](performance-issue5-cwp-admission-review.md).

The command benchmark timestamps immediately after a host PTY write, the
first complete echoed marker, the command's output marker, and its prompt.
The corrected audit finds first-echo medians around 31.6/31.3 ms on the
candidate versus 26.2/20.6 ms on the baseline; echo-to-output medians are
around 2 ms. This locates most of the observed difference before complete
echo reception, but does not identify the responsible stage. The 2 ms cap
bounds one requested host wait, not the entire guest/serial round trip.

## Checkpoint housekeeping complete

The CWP-0/7 frontend fixtures are promoted into the existing test module;
frontend suites pass **115 default / 119 profiling**. They cover wait/commit,
queued input, BREAK and ordinary/profile state equality. README/configuration
docs now describe `idle_parking = true` / `--idle-parking`, default-off
single-CPU SS-20 scope and the observed CPU/latency tradeoff. Core docs are
also reconciled. This accepted checkpoint is committed separately from
the unfinished latency attribution; no additional proof round is needed.

Keep the checkpoint separate from new diagnostic instrumentation. Preserve
the old incorrect-build evidence under its historical label. The corrected
comparison uses updated core in both arms: old frontend hash `bd12d051...`
versus both-patch hash `60d9c383...`. Pin the actual checkpoint revisions and
both source manifests/lockfiles for the next experiment, rather than relying
on a deleted temporary path or a filename alone.

## Bounded diagnostic task

Use fixed RTC, identical NVRAM/media seeds and disposable SCSI-ID-named clones.
Retain the current frontend/core baseline and a control with the previous
CWP-0 frontend predicate; change no guest or device semantics. One diagnostic
pair covering both idle regimes and 20 short command trials per regime is
enough for initial attribution. Include a small isolated character-echo probe
with verified cleanup of the guest command line to distinguish typing from
a full command. Record absolute guest virtual endpoints and host phase times.

Add bounded, default-off diagnostics using the existing `parking-profile`
isolation. Correlate bytes/sequence IDs through:

- Host write start/end and the worker's first PTY input observation.
- Each successful SCC receive acceptance, with host and guest virtual time.
- Relevant serial interrupt/service outcomes and any waits between accepted
  input and guest output. Use returned execution outcomes; never fetch code
  through the live MMU merely to trace it.
- Guest SCC output production, worker console/PTY drain and host receipt.
- Each park request, actual host wait, committed virtual interval and the
  existing reason a candidate/commit is rejected.

Keep clocks and units distinct. Worker elapsed intervals are not CPU time;
host timestamps cannot substitute for guest virtual time. Queued commands
and input must remain stashed/delivered exactly once. Do not infer worker
observation time from a PTY buffer length or host receipt. Bound trace volume
and avoid per-instruction formatting/full-transcript scanning.

Report which measured interval accounts for the extra 6–11 ms, separating
host input wait, byte delivery, guest service/timer time and output delivery.
If the trace does not settle causality, report the remaining ambiguity once;
do not guess a timer or tty-driver explanation. Diagnostic timings explain
ordering only: retain the accepted unprofiled measurements as the performance
baseline, and corroborate any later optimization with matched unprofiled runs.

## Scope limits and deliverable

Keep the 2 ms cap, wait-before-commit, elapsed-time clipping, full proof and
snapshot checks, device deadlines, serial/BREAK/input/network wakeups, default-
off setting and SMP exclusion unchanged. No cooldown, baud/timer changes,
forced guest progress, busy-spin interval, broader idle recognizer or relaxed
predicate belongs in this assignment. Only propose a narrow remedy after the
trace identifies a concrete cause.

Return a short stage-timing table, retained diagnostic patch/build recipe,
source/binary/lockfile hashes, raw bounded trace, clean guest halt and the
next proposed action. Keep diagnostics compiled out of normal builds; run
relevant equality/protocol checks. Preserve source media/live catalog, remove
scratch builds/clones, and stop for review without committing instrumentation.


## Initial bounded result

The frontend CWP fixtures were promoted into `parking_tests`; default/
profiling frontend counts are **115/119**, and the user docs now state the
default-off SS-20/UP scope, 69–80% idle CPU savings and 6–11 ms command
response tradeoff.

The corrected four-pair unprofiled comparison (core+frontend patches,
instrumentation absent) measured 24.29/42.88% idle CPU on baseline versus
7.60/8.40% on the candidate. Command medians were 27.83/22.32 ms versus
33.77/33.36 ms; fixed work was 5.366/5.335 s versus 5.217/5.180 s.

A bounded 20-trial-per-idle-phase first-byte trace found the extra delay
**before the first nonempty host read following command submission**:

| Window | Baseline first-byte median / p95 | Candidate first-byte median / p95 |
| --- | ---: | ---: |
| Pre-command | 25.15 / 37.05 ms | 30.46 / 32.62 ms |
| Later | 19.33 / 21.45 ms | 30.62 / 33.52 ms |

The p95 values above are nearest-rank from 20 trials; the earlier retained
JSON fields accidentally held the maxima. They are corrected here without a
guest rerun.

The interval from complete echoed marker to command output remains short
(about 1.6 ms baseline and 2.3–2.4 ms candidate), and the prompt follows.
This does **not** rule out delayed production, buffering or drain of the
initial echoed byte. Those stages occur before the first host receipt too.

Worker PTY input observation, SCC acceptance, guest service and output-drain
timestamps were not captured in this bounded pass. The responsible stage
before first echo therefore remains unresolved; do not attribute it to the
tty driver, timers or parking from these host-only results. No policy change
is proposed.

Evidence: `latency-diagnostic-baseline/`, `latency-diagnostic-candidate/`,
`latency-first-byte.json`; corrected binary hashes retained. Temporary source/
build copies were removed. No commit.

## Independent initial-pass review and next action

Both recorded binary hashes match the corrected builds, source input hashes
match, and both guests completed sync/halt to OBP with exit 0. The reported
first-byte and complete-echo medians recompute exactly from the 20 per-window
trials. `first_byte_ns` is the timestamp of the first **nonempty host read**,
not a guest-output production timestamp or worker-input observation. Retain
that distinction; the host-only probe does not finish the stage-attribution
assignment, nor provide the requested isolated character-echo experiment.

The original `p95` fields in `latency-first-byte.json` contained each 20-trial
maximum. They now use the existing nearest-rank convention, sorted index
`ceil(0.95 * n) - 1` (18 for n=20). Correct first-byte p95 values are
37.050/21.455 ms baseline and 32.622/33.522 ms candidate. Medians are unchanged;
all 16 median/p95 pairs independently recompute from the raw timestamps.
This reporting correction required no guest rerun and does not revise the
accepted four-pair performance table.

The checkpoint is now committed; both production source hashes match the
accepted review. The next work is the already specified bounded correlation
of host write, worker PTY
observation, SCC acceptance, guest output production, output drain and host
receipt, including waits and both time domains. Do not run another host-only
benchmark as a substitute, and do not change timing policy until the missing
stages are measured. Preserve the bounded patch/build recipe and report the
remaining uncertainty once if causality is still unresolved.

Independent arithmetic, test-log counts and source hashes are retained at
`local/review-issue5-parking-latency-initial-2026-11-18/audit.json`. This review
changed documentation only, reran formatting checks, created no temporary
builds or guest clones, and made no commits.


## First worker-stage trace: historical endpoint/correlation review

One diagnostic pair is retained: two windows of 20 trials per arm, fixed RTC,
disposable media, and recorded build hashes. Both guests halt cleanly with exit
0; the scratch disks and `/tmp/opencode/trace` are removed. The complete accepted
SCC input streams match all 40 commands per arm, and none of the four command
traces reaches its 4096-event cap. The accepted production source hashes remain
unchanged. These checks pass; the claimed production-stage attribution does not.

### The two output timestamps measure queue transfer

In both retained patches, `guest_output` is recorded in `drain_console` **after
`serial.take_output(Channel::A)`**. It measures frontend collection from the
SCC output queue. The actual production site is the SCC `Port::send` enqueue
in `src/devices/serial.rs`; the patch does not timestamp that site. Therefore
the purported SCC-acceptance-to-production interval still includes any time
the byte spends in the core output queue awaiting collection.

Likewise, `output_drain` is recorded after `SerialPty::send`, which only appends
to the frontend output queue. The successful OS write is in
`SerialPty::poll`. The approximately 2 microseconds between the existing
timestamps measure collection/enqueue overhead, not guest production to host
publication. Output queue residence has **not** been excluded as a cause.

### PTY coverage and trial association are incomplete

`pty_observation` logs a queue-length increase around `SerialBridge::service`
polling, with an empty byte field. Reads performed by `host_work_pending` are
unlogged. Three of the four command traces consequently start with SCC
acceptances and no preceding observation event.

Every field in `stage-trace.json` can be reproduced by choosing the first event
of each kind after the host write, without correlating command/byte identity.
This creates **6/40 inconsistent baseline rows and 3/40 candidate rows**:

- Baseline trial 0 assigns PTY observation at 36.526 ms, after its SCC
  acceptance at 1.483 ms. Candidate trials 0 and 20 have the same missing-read
  problem: assigned observations at 35.238/41.340 ms follow acceptances at
  2.396/1.082 ms. Those observation events belong to subsequent commands.
- Baseline trials 1, 8, 22, 24 and 29, and candidate trial 1, select a leftover
  prompt space as their first output, before the new command's SCC acceptance.
  The harness permits a bare `#` to finish a trial before its trailing space.

Prompt median acceptance does not prove that no host wait overlapped arrival;
short waits and waits after acceptance remain unmeasured. Preserve that limit.

### What the retained byte streams do support

The review matched each unique `echo HZnnnn\r\n` in the collected output stream
to its first accepted command byte, rather than accepting the next arbitrary
output event. Medians below are calculated from **per-trial differences**:

| Window | Acceptance → echo collection, host ms (baseline / candidate) | Same interval, guest virtual ms (baseline / candidate) |
| --- | ---: | ---: |
| First | 25.674 / 30.562 | 26.829 / 26.927 |
| Later | 20.264 / 30.285 | 26.980 / 26.927 |
| Pooled, descriptive only | 20.854 / 30.431 | 26.882 / 26.927 |

This confirms delayed **collection** of the matching echo in host time, while
the observed guest-time interval is similar. It does not separate guest
execution, host waiting and SCC queue residence. The original 20.744/30.443 ms
figures were differences between endpoint medians, not medians of paired
durations; their labels and correlation are superseded by this audit. None of
these diagnostic results revises the accepted unprofiled performance table.

### Next bounded task

Correct the trace before its next guest run; keep this measurement-only:

1. Log successful reads and actual returned bytes/stream offsets **inside
   `SerialPty::poll`**, so all callers are covered. Log the successful OS write
   and accepted byte count there too. Keep collection and enqueue as separate,
   accurately named events.
2. Add default-off core tracing at actual SCC output enqueue, with channel,
   byte/stream identity and host/guest time. A step-boundary observer is also
   acceptable if it explicitly bounds its timestamp error and does not label
   a batch drain as production. Use existing execution outcomes for PC/trap
   context; never inspect instructions through a live MMU load.
3. Correlate complete input and echoed byte streams, preserving trial identity
   across split reads, batched output and trailing prompts. Reject missing or
   out-of-order associations rather than borrowing the next trial's event.
   Self-check the correlator with leftover prompt space, split/batched echo and
   missing-observation examples before rebooting a guest. Record host write
   start/end and a monotonic time mapping; current wall-clock stage times and
   post-write monotonic host-receipt times have different origins.
4. Add per-trial actual wait start/end, requested cap, virtual start/end,
   committed time and cancellation outcome. Cover acceptance through the first
   correlated echo, including waits after input acceptance. Include the already
   requested isolated character-echo probe with verified command-line cleanup.
5. Keep all instrumentation behind `parking-profile` and check both build
   configurations. The retained patch adds unconditional `ParkingProfile`
   parameters/call arguments despite that type and worker variable being
   feature-gated; it is not a working default-build compile-out patch as-is.
   Remove the unused import reported by its diagnostic builds.

Use one corrected pair with both windows, retain the parser/source/build
recipe and raw bounded events, and return per-window paired stage statistics.
Do not alter the 2 ms cap, deadlines, proof, input protocol, default-off scope
or SMP exclusion. No extra broad performance sweep is requested. An uncertain
cause after this bounded trace should be reported as such, not replaced by a
timing workaround.

Original evidence: `stage-trace.json`, `stage-trace-baseline.patch`,
`stage-trace-candidate.patch`, `stage-trace-build.md`, `measure_trace.py`,
`trace-baseline/`, `trace-candidate/` and binary hashes. Independent reproducible
audit: `local/review-issue5-worker-stage-2026-11-18/audit.py` and `audit.json`.
The audit changes no source and runs no guest. This review updated documents
only, created no temporary builds or clones, and made no commits; #5 stays open.


## Actual core SCC trace: localization accepted, wait attribution pending

The second bounded pair fixes the production endpoint: `scc_data_read` is
recorded at the SCC RX pop, and `scc_output` immediately after the transmit
output enqueue in `Port::send`. `scc_data_write` records an attempted register
write before its checks; in this evidence its byte stream matches production
exactly. All new hooks are feature-gated. Admission and wait policy are unchanged.

The independent audit reproduced all **39/40** retained trials per arm. Each
window has exactly 240 reads, 460 data-write events and 460 production events;
the complete input/output streams match the commands, echoes, command results
and prompts. No window reaches the 8192-event limit. Trial 1 in each arm is
excluded because its first host byte is a trailing prompt space. Both guests
halt cleanly with exit 0, source-input hashes match across arms, and disposable
disk clones are absent.

### Corrected stage arithmetic and endpoints

The reported token endpoints are the first **H** of `HZnnnn` in the input and
echo streams, not the first `e` in `echo` and not complete-token receipt.
20.906/29.146 ms were differences of endpoint medians. The correct medians of
each matched trial's read-to-production interval are **21.158/29.048 ms**.
This arithmetic correction needs no guest rerun.

| Median across the 39 matched trials | Baseline | Candidate |
| --- | ---: | ---: |
| Host write → guest read of first token byte H | 1.221 ms | 1.318 ms |
| Guest read H → production of echoed H, paired host interval | 21.158 ms | 29.048 ms |
| Same H → H interval, guest virtual time | 25.823 ms | 25.801 ms |
| Guest read e → production of echoed e, paired host interval | 21.854 ms | 30.150 ms |
| Same e → e interval, guest virtual time | 26.728 ms | 26.706 ms |
| Host write → first host receipt | 22.132 ms | 30.611 ms |

Keep the two windows separate in subsequent reports:

| Window | Paired H → H host ms (baseline / candidate) | Paired H → H virtual ms (baseline / candidate) |
| --- | ---: | ---: |
| First, 19 matched trials | 28.519 / 29.005 | 25.899 / 25.801 |
| Later, 20 matched trials | 19.414 / 29.112 | 25.818 / 25.801 |

These events support the additional host delay occurring after the guest reads
input and before it produces the matching echo. They do **not** establish a
guest serial/tty defect: host waits, normal execution and host scheduling can
all occur between those guest events. The near-identical virtual intervals
make host/virtual-time pacing a useful hypothesis for the next trace, not a
proven explanation.

The approximate same-byte first-e production-to-host-receipt medians are
0.191/0.283 ms, much smaller than the read-to-production difference. This uses
a wall-clock pre-write timestamp and a monotonic post-write origin, so retain
clock-origin uncertainty; do not subtract H-production and e-receipt endpoint
medians and call the result an exact delivery duration. Core `guest_ns` is
the frontend's pre-step stamp, not an in-step clock read.

The current frontend trace patch exports core events and updates that virtual
stamp. Contrary to the final bullet in `stage-trace3-build.md`, it contains
**no** PTY observation/acceptance/drain instrumentation. That does not prevent
the narrower SCC-read-to-production conclusion above, but those stages must
not be presented as measured by this run.

### Finish the bounded assignment

Use the actual SCC hooks for one pair covering both windows. Complete both
missing measurements in that pass:

1. Record each actual park wait's start/end, requested duration, guest virtual
   endpoints, committed time and cancellation result. Correlate waits with
   each same-byte read-to-production interval, including waits after the SCC
   read. Sum overlapping host-wait durations per trial and report the residual
   elapsed interval; that residual is not CPU time and may include scheduling
   or diagnostic overhead. Keep clocks monotonic or retain explicit bounded
   clock mappings, and report trace overflow rather than silently accepting a
   truncated phase.
2. Add the isolated single-character echo probe with verified command-line
   cleanup. Match that same byte at SCC read, production and host receipt.
   Preserve the command tests as a separate workload and reject leftover
   prompt bytes. Record channel/stream identity for the extended trace.
3. Retain the parser and exact baseline predicate patch/build commands, plus
   raw events and hashes. Report per-window **paired** statistics and exclusion
   reasons. Use the independent audit below to check the existing command
   correlation before a new guest run; do not repeat the old endpoint-median
   subtraction. Fix the build-note claim about nonexistent PTY instrumentation.

No additional command-only trace or broad performance sweep is required. If
host wait accounts for the extra wall time while virtual response time stays
constant, report the pacing tradeoff explicitly rather than assuming a guest
bug. Otherwise return the residual interval and a bounded next hypothesis.
Keep the 2 ms cap, proof, deadlines, input handling, default-off setting and
SMP exclusion unchanged; no optimization is authorized by this diagnostic pass.

Evidence: `stage-trace3.json`, `core-stage-trace.patch`,
`frontend-stage-trace.patch`, `stage-trace3-build.md`, `trace3-baseline/`,
`trace3-candidate/`, `trace3-binaries.txt`. Independent replayable audit:
`local/review-issue5-scc-stage-2026-11-18/audit.py` and `audit.json`.
The 814 default / 825 profiling successful test executions are confirmed in
the retained logs; independent formatting and diff checks pass in both repos.
This review ran no guest/build and changed no production source. Diagnostic
code remains uncommitted in both repos; no commit was made, and #5 stays open.


## Wait-aware trace: sufficient evidence for a bounded prototype

The independent audit reproduced all 37/40 baseline and 40/40 candidate byte
matches, with complete command input/output streams. Both guests halted with
exit 0; source-input hashes match across arms, scratch clones are removed,
and no command trace reaches its event cap. Core 814/826 and frontend 115/119
successful executions are confirmed in the retained logs.

Two reporting corrections are necessary, without another guest run:

- The reported 19.079/29.722 ms again subtract endpoint medians. Paired H-read
  to echoed-H-production medians are **19.103/29.544 ms**.
- The submitted wait selector starts at the **host write**, not the SCC read.
  Candidate trials 0 and 20 consequently include pre-read waits. Restricting
  events to the actual read-to-production interval gives 12 waits in 38/40
  candidate trials, 11 in trial 0 and 9 in trial 20. The baseline has no waits
  in 28/37 trials and 12 waits in the other 9, all in the first window. Zero
  is its median, not a statement that it never parks after input.

| Corrected median across matched commands | Baseline | Candidate |
| --- | ---: | ---: |
| Host write → first token byte H read | 0.983 ms | 1.399 ms |
| H read → echoed H production, paired host interval | 19.103 ms | 29.544 ms |
| Same interval in guest virtual time | 25.841 ms | 25.801 ms |
| Host write → first host receipt | 20.087 ms | 31.309 ms |
| Wait events within read → production | 0 | 12 |
| Requested / reported elapsed wait total | 0 / 0 ms | 23.069 / 23.921 ms |
| Per-trial host interval minus reported wait total | 18.619 ms | 5.503 ms |

Baseline trials with no waits have a 18.727 ms median read-to-production
interval; its nine trials with waits have a 28.778 ms median. Together with
the similar virtual response intervals, this supports a pacing tradeoff.
The wait duration is not an additive estimate of the regression: parking
also replaces ordinary idle-loop execution. The residual above is elapsed
host time, not CPU time.

The trace records one end-observation timestamp and call elapsed time for
each wait, rather than explicit sleep start/end and commit/cancel outcome.
The retained events selected within the read interval have reconstructed
starts after that read. This is sufficient to establish the wait chain;
do not claim exact scheduler or committed-time accounting from these fields.

There is one isolated `X` probe per window and arm, not a distribution whose
median/p95 can be reported. It observed X and completed Ctrl-U/newline cleanup:
27.202/18.316 ms baseline and 27.266/27.661 ms candidate. The harness resets
profiling after each character probe, so its core/wait events are not retained.
Its duplicate X in `serial.log` is a double transcript append in `char_probe`,
not duplicate guest delivery. Fix that append when reusing the harness.

Evidence: `stage-trace4.json`, `core-stage-trace2.patch`,
`frontend-stage-trace2.patch`, `stage-trace4-build.md`, `measure_trace2.py`,
`trace4-baseline/`, `trace4-candidate/`. Replayable independent audit:
`local/review-issue5-wait-chain-2026-11-18/audit.py` and `audit.json`.
These reporting limits do not require another measurement-only round before
the prototype below. They remain limits on the causal precision of this run.

## Next slice: bounded frontend serial-input grace

Implement and measure this isolated prototype, then return it **uncommitted**.
This is a host responsiveness policy within opt-in parking, not a correction
to Solaris, SCC hardware, timers or the proved idle loop.

### Policy

- Use one initial **40 ms monotonic host-time deadline** after each byte
  successfully delivered by `serial.receive(Channel::A, byte)`. This is an
  experimental bound informed by the observed response scale, not a hardware
  constant or a claim of a guaranteed response time. Keep it as one named
  implementation constant; do not add another user setting for this slice.
- Renew the deadline only for another successfully accepted serial byte.
  Both PTY and internal-console input use the existing `SerialBridge::service`
  delivery path. Queued-but-undelivered bytes and rejected receives already
  have pending-work handling and must not falsely count as accepted input.
- While the deadline is active, decline parking and continue ordinary machine
  execution and host event service. Do not sleep and discard the time, jump
  guest time forward, consume input twice, or bypass device service.
- Expiry is unconditional: echo/output may be absent, disabled, binary,
  delayed, or unrelated to the input. Do not infer pending guest work from
  echo matching, clear the deadline on arbitrary output, or extend it due to
  output. No guest/terminal protocol acknowledgement is required.
- Keep this state ephemeral in the frontend; clear it on receiver reset,
  machine reset/replacement and stop. Handle pause/resume without carrying a
  stale deadline. Use the same guard in ordinary and profiled parking paths.
  Avoid new clock reads in the per-instruction path when no grace is active.
- Preserve the accepted default-off setting, SS-20/UP scope, exact CWP proof,
  full token validation, 2 ms cap, wait-before-commit/clipping, device deadlines
  and all input/BREAK/network/shutdown checks. No core/MMU/SCC behavioral
  changes, broader idle recognition or SMP parking belong in this slice.

The deadline is essential: "until processing/echo" has no reliable generic
completion signal. A password prompt or disabled terminal echo could otherwise
disable parking indefinitely. Continuous real input may renew the grace; after
the final accepted byte the extra ordinary execution is bounded to 40 ms.

### Validation and comparison

Extend meaningful existing parking/protocol fixtures using an injected clock,
without tests that sleep on the host. Cover successful versus rejected
delivery, renewals, expiry with no guest output, unrelated output, reset and
machine replacement, input delivery exactly once, and ordinary/profile parity.
Once grace expires, a valid idle machine must again prepare and commit parking.
Keep the existing wakeup/token fixtures and required repository checks passing.

Compare **current CWP-0–7 parking with and without grace**, with parking enabled
in both arms and profiling absent. Do not reuse the historical CWP-0-only
frontend as the reference for this change. Retain exact core/frontend revisions,
patches, feature flags, lockfiles, toolchain and binary hashes.

Use two alternating pairs on identical disposable media/NVRAM with fixed RTC.
Measure command and isolated-character latency, pre/post idle worker CPU after
grace expires, command-phase worker CPU, and the existing fixed useful-work
phase. Include a controlled echo-disabled/no-output input case: demonstrate
that parking resumes after the last accepted byte and restore terminal state.
Use paired per-trial statistics and clearly list exclusions. Report the CPU
cost of the grace period; latency improvement is not free CPU work.

Acceptance requires a repeatable responsiveness improvement, preserved
settled-idle savings, no indefinite inhibition when output is absent, and no
meaningful active-work regression. If 40 ms is ineffective or too expensive,
return the measurements before tuning multiple bounds. Preserve source media
and the live catalog, retain compact evidence/runner sources, and remove owned
scratch builds/clones. No default switch, commit, push or merge in this slice.

This review implemented no policy or production code, ran no guest/build,
and created only compact audit evidence and documentation. Issue #5 stays open.


## Bounded frontend input-grace prototype

The frontend now has one named 40 ms monotonic grace, armed only after a
successful `serial.receive(Channel::A, byte)`. It is renewed only by another
successful delivery, expires unconditionally, and does not inspect echo or
output. Receiver reset, machine replacement, pause and stop clear it. Both
ordinary and profiled prepare paths decline parking while it is active; the
core proof, 2 ms cap, deadline, wakeup and SMP behavior are untouched.

Injected-clock tests cover renewal, expiry without output, unrelated output,
rejected versus successful delivery (exactly once), receiver reset, lifecycle
clearing and ordinary/profile parity. Frontend suites pass **120 default /
125 profiling**; core **814 default / 826 profiling**; Clippy passes both
configurations.

Two alternating control/grace pairs used the same disposable media and NVRAM
seeds, fixed RTC, parking enabled in both arms and no profiling. Control is
the same source with the grace compiled to zero.

| Window | Control command median | Grace command median | Control CPU | Grace CPU |
| --- | ---: | ---: | ---: | ---: |
| Pair 1, first | 33.50 ms | 21.18 ms | 30.1% | 99.2% |
| Pair 1, later | 34.09 ms | 21.36 ms | 33.1% | 99.1% |
| Pair 2, first | 33.66 ms | 21.11 ms | 31.5% | 99.0% |
| Pair 2, later | 32.81 ms | 21.22 ms | 30.3% | 100.9% |

Paired command medians improve by **-12.63/-12.00 ms** in the two pairs;
paired p95 improves by **-10.13/-9.11 ms**. Five trials per grace arm are
excluded from first-byte statistics because their first host byte is a
leftover prompt space; complete echo/output timing remains valid.

Settled idle worker CPU stays low: pre/post medians are 7.4/6.8-8.4% control
and 6.8/7.2-8.8% grace. Fixed useful work is unchanged at 5.235/5.177 s
control versus 5.172/5.181 s grace. Character probes complete with cleanup;
individual first-byte times are 31.85/31.01 and 35.25/37.52 ms control versus
23.06/24.05 and 23.81/21.25 ms grace.

The controlled echo-disabled probe produced no output, showed grace execution
(81.3% worker CPU in the ~37 ms window), then returned to **5.0/7.5%** worker
CPU after expiry; `stty echo` restoration and prompt cleanup succeeded in both
pairs. Parking therefore resumes without an echo acknowledgement.

The 40 ms grace removes the measured command-latency regression and preserves
settled idle/fixed work. Back-to-back commands renew grace continuously,
keeping command-phase worker CPU at about 99-101% versus 30-33%. The independent
review below accepts this explicit cost for the experimental default-off
checkpoint: parking saves CPU in quiet periods, and is not a CPU usage cap
during continuous input. Do not tune another bound in this slice.

Evidence: `grace-comparison.json`, `frontend-input-grace.patch`,
`grace-build.md`, `grace-inputs.sha256`, `measure_grace.py`,
`grace-ba{1,2}-{control,grace}/`, their logs and `echo-disabled.json`.
The prototype is uncommitted in Heliacal; Helios core remains unchanged.

## Independent grace review: accepted default-off checkpoint

The reviewed source arms only on successful delivery, renews on another
successful byte, ignores output and clears the deadline on expiry. Ordinary
and profiled preparation share the guard; proof, wait/advance, clock and
device behavior are unchanged. The accepted change is frontend-only; earlier
uncommitted tracing remains a separate concern.

An isolated current-source copy passes **14 default / 19 profiling** parking
tests, including three independent fixtures for renewal while still active,
preserving an existing deadline on rejected/empty delivery, and completing a
full ordinary park after expiry. The same tests also pass after removing all
prior tracing from the copy and using the committed core/profiler. Thus the
grace change does not require those uncommitted diagnostic APIs. Formatting
and diff checks pass in both working repositories.

The arithmetic audit confirms all four clean guest halts, fixed-work checksum,
matching media/NVRAM inputs, source hashes, raw latency timestamps and idle
CPU tick deltas. The measured grace release hash is `503a0a7e...`; it has no
`HELIACAL_PARKING_PROFILE` runtime marker. Run order is control/grace/control/
grace, not reversed-pair counterbalancing. The repeated improvement is large
enough to accept this bounded checkpoint without another ordering experiment.

Use precise reporting labels:

- Each grace arm has five invalid first-byte observations caused by trailing
  prompt spaces. These do not invalidate the complete command-output timings.
  Using all 40 command-output trials gives paired median changes of
  **-12.433/-11.955 ms**. The 35-trial common first-byte subset gives the
  reported **-12.633/-12.002 ms**; identify which subset is used.
- **-10.128/-9.114 ms** are the 95th percentiles of paired differences, not
  differences between the arms' 95th percentiles. No rerun is required.
- Actual settled-idle samples span **6.4-8.4% control / 6.8-8.8% grace**.
  Character probes are single observations per window. The ~37 ms no-echo CPU
  sample is coarse because CPU accounting uses scheduler ticks; its later
  5.0-7.5% recovery and absence of output establish the intended behavior.

### Finish this checkpoint, without tuning

1. Update Heliacal README/configuration documentation. The current text still
   says parking necessarily adds 6-11 ms and omits the grace policy. State the
   default-off SS-20/UP scope, 40 ms after the last successful serial delivery,
   unconditional no-echo expiry, measured low settled-idle CPU, and the possible
   full-core cost of continuous serial input. Do not advertise a latency or
   CPU guarantee for every host/guest.
2. Include one small lifecycle cleanup: a failed NVRAM save in
   `Command::Shutdown` changes Running to Paused without clearing grace.
   Call the same `serial.clear_input_grace()` there when entering Paused.
   Normal explicit pause, successful replacement, receiver reset, power-off
   and execution-error paths already clear it. This is a bounded stale-policy
   edge case, not a failure of the latency/idle results; no new guest run is
   required for this one-line cleanup.
3. Keep the grace commit separate from SCC/wait tracing. An isolated,
   independently tested `production-grace-only.patch` is retained in the
   review directory; it contains the submitted grace code/tests only, without
   the three frontend tracing additions or uncommitted core APIs. It does not
   include the documentation or pause-error cleanup above. Stage a complete
   self-contained production change, and keep diagnostics separate.

Retain the 40 ms bound and default-off setting. Continuous input renewing
grace is expected; shrinking the bound merely because a continuous-command
benchmark uses a core would trade the accepted responsiveness away without
evidence of a better policy. Broader CPU-efficiency policies can be considered
later under issue #5. This review does not authorize default-on use or close #5.

Independent evidence is under `local/review-issue5-input-grace-2026-11-18/`:
raw-data `audit.py`/`audit.json`, focused test logs, fixture source, build recipe,
the separated production patch and `cleanup.json`. The owned scratch checkout
and build `/tmp/heliacal-grace-review-wnbnv0_r` were removed. No guest media or
live catalog was changed. This review changed documentation only and made no
production edits, commits, pushes or merges.


## Checkpoint completion

The production frontend grace was committed separately from tracing as
Heliacal `682d85b Add serial input grace to idle parking`. That commit contains
the 40 ms grace, lifecycle cleanup, injected-clock tests and the README/
configuration tradeoff documentation; it contains no SCC/wait tracing and was
built and tested from an isolated checkout against Helios `3a1cf6f`.

The failed-NVRAM-save path now clears grace when it forces the worker into
Paused. The staged production tree passes 120 default / 125 profiling tests,
formatting and Clippy for both configurations. The remaining tracing changes
in both repositories stay uncommitted and separate. The 40 ms bound and
default-off setting remain; continuous input may keep the worker fully active,
and #5 remains open.


## Integration handoff

The SCC/wait tracing was removed from both production worktrees after capture.
Reproducible patches and apply/build instructions are retained under
`local/issue5-jit-2026-11-18/tracing/`; raw evidence remains under
`local/issue5-cwp-admission-2026-11-18/`. The production trees are clean at
Helios `2358935` and Heliacal `682d85b`, with parking default-off, SS-20/UP only,
the CWP proof, 2 ms cap and 40 ms serial-input grace.
