# Issue #5: typed device-digest review

Accepted 2026-11-18 and committed as Helios `939a7bc`; Heliacal remains at
`8e8d82a` on baseline Helios `316ea8a`. **No blocking correctness findings.**
The review made no commit, push, merge or production-code edit; #5 remains
open.

## Coverage and behavior

The new encoders cover all 142 declared fields across 23 state structs in 16
device modules. An independent sensitivity suite was generated from the state
declarations, not the encoder implementations. Each field mutation changes
both the old Debug fingerprint and the typed fingerprint. This includes nested
clock, timer, serial-port, interrupt-bank and floppy-motion state, as well as
MXCC identity, data and registers.

Additional tests verify option tags for zero/false, nested sequence boundaries,
equal-length payload changes, high integer bits, array tails, map value changes,
map insertion-order independence and deque backing-layout independence. The
enum encodings were also inspected: Ethernet phases, interrupt target policy
and I/O flush variants retain distinct tags and payloads. Existing ESP/SCSI
diagnostic methods are unchanged.

Two independent default-build token fixtures configure serial reception before
token construction, then add or replace only received payload bytes. Both
invalidate the token; the equal-length replacement also changes the diagnostic
snapshot while preserving CPU state. The submitted staged-serial fixture also
changes WR3 after token construction, so it alone cannot isolate payload
coverage. For ongoing regression coverage, promote the stronger retained
fixtures from
[`typed_digest_review.rs`](../local/review-issue5-typed-digest-2026-11-18/typed_digest_review.rs).
All additional fixtures passed in default, profiling and combined builds.

Source inspection confirms no change to admission order, proof/readiness,
virtual-time advancement, frontend waiting, input/BREAK/network checks, the
2 ms cap, default-off behavior or SMP exclusion. No field is replaced by a
cache or generation shortcut. New hash values intentionally differ from the
Debug representation; their numeric compatibility is not an API requirement.

## Performance verification

The submitted 16 logs reproduce the reported 4.511017 -> 0.464657 s medians,
or **89.70% less wall time** for the fixed token workload. I rebuilt both
checkpoint and candidate in separate release targets, without profiling, and
repeated eight alternating pairs after the regression suite had finished:

| 20,000 token + 1 ns advance cycles | Checkpoint | Typed digest |
| --- | ---: | ---: |
| Median wall time | 4.559940 s | 0.468929 s |
| Median worker CPU | 4.540 s | 0.460 s |

The independent wall reduction is **89.72%**. Every run completed exactly
20,000 advances totaling 20,000 ns; binaries have distinct retained hashes and
both complete builds succeeded. The original submitted benchmark build logs
contain an unrelated missing-ASCII-logo CLI build error despite emitting fresh
test executables. The clean independent builds and repeated measurements
resolve that provenance limitation for the accepted fixed-work result.

Frontend phase arithmetic was independently recomputed from `/proc` samples
and transaction timestamps. Both arms used matching source disk/NVRAM/ROM
hashes, and all four runs retained guest sync/halt and exit 0. Pre-command idle
CPU improved, later idle CPU rose, and command CPU stayed similar. These data
do not establish the cause of the later-window increase or a broad frontend
speedup. No new guest boot was necessary for this review.

Two reporting clarifications were applied directly to the implementation
report:

- The converted encoders are allocation-free. The **whole parking digest is
  not**: ESP still formats its controller state, and attached SCSI targets
  format their kind/media information inside the unchanged subdigests.
- Latencies use host write to the second output token. The table summarizes
  each run's median/p95 across two runs; these are not pooled percentiles.
  Guest-work or host variation remain possible explanations for later idle
  CPU, rather than an established cause.

## Validation and next checkpoint

Independent full core validation, including 22 additional review tests:

- 805 default / 816 `parking-profile` / 819 combined successful executions;
  each includes the existing isolated MXCC child rerun, with zero failures.
- Formatting, Clippy `-D warnings` in all three configurations and all four
  built-in demos pass.
- The submitted core 783/794/797 and frontend 114/117 logs, frontend build,
  SDL smoke and OBP smoke evidence were inspected. Heliacal source was not
  modified, and its full suite was not independently repeated this time.

Checkpoint this digest change before further optimization. Keep the existing
parking safeguards and #5 open. If performance work continues, the unresolved
frontend question is how much normal guest work occurs in the later idle
window; preserve both idle regimes and actual command latency when measuring
the next step. Do not infer that removing the remaining small formatting calls
will fix that behavior without measuring their cost.

Evidence is in `local/review-issue5-typed-digest-2026-11-18/`: field inventory,
fixture generator and patch, token fixtures, source hashes, validation logs,
independent eight-pair results, and recalculated submitted measurements.
Temporary source copies and build directories are removed after review. Source
media, the live catalog and the submitted evidence are preserved.
