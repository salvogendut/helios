# Issue #5: opt-in Heliacal parking trial

Status: uncommitted frontend trial on core checkpoint `e3d87e3`; issue #5
remains open. No commit or push was made in Heliacal. Frontend sleeping, host
pacing and SMP parking are opt-in/out-of-scope exactly as stated below.

Review: **accepted as a default-off experimental checkpoint**. The independent
[frontend review](performance-issue5-idle-parking-frontend-review.md) confirms
14/14 protocol cases, 114 frontend tests, formatting, Clippy, release build,
SDL smoke and original-OBP display smoke. Fresh matching performance results
and final user-facing documentation remain separate follow-ups.

## Change

- Added `Settings.idle_parking`, defaulting to `false`, plus the
  `--idle-parking` CLI switch. It is not a hardware/cold-reset setting. The
  worker reads it when a machine is successfully constructed, so it is a
  boot-time experimental selection rather than a live toggle. It only has an
  effect for a single-CPU SS-20 running the validated Solaris boundary.
- The worker now uses prepare/wait/recheck/commit parking. It computes a
  wait budget from the core's earliest deadline (maximum 2 ms), waits on the
  host first, then derives an explicit virtual delta from the measured elapsed
  time or the deadline budget. It never commits a future interval before the
  host wait. Host oversleep is capped; excess elapsed time is discarded, not
  caught up across events.
- Before preparing the token it drains the core SCC output into the PTY and
  internal console, preserving transcript/frame state; the normal batch
  boundary uses the same helper. It then checks queued commands (stashing
  rather than consuming them), abort/shutdown, GUI serial/keyboard/paste/mouse
  queues, pending keyboard delivery and PTY input/output. Network service
  occurs in the outer loop before the CPU batch and again after waiting; it
  is bounded polling rather than a notification-based wakeup.
  The input mutex is not held during the wait. A pending/active serial BREAK
  rejects parking because its release is outcome-count based.
- The outer worker loop continues to service commands, serial/PTY input,
  keyboard/mouse, network and pause/reset after each bounded wait. A queued
  pause/reset/replacement cancels an opportunity without committing time.
- SMP and non-SS-20 machines cannot admit a token; the existing execution
  path is unchanged when the option is off.

## Real run

A disposable SS-20 Solaris 2.6 disk was booted to a root shell and allowed to
settle. The core-only run confirmed 20/20 boundary admissions and 20/20
zero-time advances, with CPU/MMU/RAM preserved and no host sleep
(`local/issue5-parking-real-2026-11-17/`).

Earlier headless frontend measurements on this host, before the final output
drain/context/test correction (not rerun on the accepted candidate):

| State | Machine worker | PTY median | PTY p95 |
| --- | ---: | ---: | ---: |
| Parking off | 99.17% of one CPU | 11.156 ms | 12.956 ms |
| Parking on | 24.79% of one CPU | 11.362 ms | 37.637 ms |

The corrected CPU reduction is meaningful but usage remains above the earlier 10–15%
target; proof failures for transient active/steady-state boundaries fall back
to normal execution. Input p95 remains below 50 ms. This is not an X/desktop or SMP claim and not a physical keyboard
latency result. The corrected wait ordering and host admission checks are
covered by independent protocol fixtures. The retained measured binary is
from the preceding wait-first candidate, not this final correction. Evidence is under
`local/issue5-parking-frontend-2026-11-17/`.

## Validation

Heliacal passes **114 tests**, formatting, Clippy with `-D warnings`, release
build, SDL smoke and original-OBP display smoke. Independent protocol fixtures
pass 14/14. Core source remains the accepted `e3d87e3` checkpoint; its earlier
781/784 unique core tests and standard checks were not repeated because the
core source is unchanged.
