# Issue #5: locate the idle-parking command-response delay

Status: CWP checkpoint housekeeping is complete and ready to commit. The
host-only first-byte probe is reviewed; worker-stage attribution remains
unfinished. No latency-policy optimization is authorized by this handoff.
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
also reconciled. This accepted checkpoint is ready to commit separately from
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

| Window | Baseline first byte / echo | Candidate first byte / echo |
| --- | ---: | ---: |
| Pre-command | 25.15 / 29.41 ms | 30.46 / 30.91 ms |
| Later | 19.33 / 19.57 ms | 30.62 / 30.79 ms |

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

The `p95` fields in `latency-first-byte.json` contain each 20-trial maximum.
For consistency with the existing nearest-rank convention, use sorted index
`ceil(0.95 * n) - 1` (18 for n=20). Correct first-byte p95 values are
37.050/21.455 ms baseline and 32.622/33.522 ms candidate. Medians are unchanged;
this reporting correction needs no guest rerun and does not revise the
accepted four-pair performance table.

Commit the accepted checkpoint before adding worker instrumentation. The next
work is the already specified bounded correlation of host write, worker PTY
observation, SCC acceptance, guest output production, output drain and host
receipt, including waits and both time domains. Do not run another host-only
benchmark as a substitute, and do not change timing policy until the missing
stages are measured. Preserve the bounded patch/build recipe and report the
remaining uncertainty once if causality is still unresolved.

Independent arithmetic, test-log counts and source hashes are retained at
`local/review-issue5-parking-latency-initial-2026-11-18/audit.json`. This review
changed documentation only, reran formatting checks, created no temporary
builds or guest clones, and made no commits.
