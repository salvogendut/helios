# Issue #5: opt-in Heliacal parking trial

Status: committed default-off frontend checkpoint `a88d769` on accepted core
`e3d87e3`; issue #5 remains open and no merge or push was made. Frontend
sleeping, host pacing and SMP parking are opt-in/out-of-scope exactly as stated
below.

Review: **accepted as a default-off experimental checkpoint**. The independent
[frontend review](performance-issue5-idle-parking-frontend-review.md) confirms
14/14 protocol cases, 114 frontend tests, formatting, Clippy, release build,
SDL smoke and original-OBP display smoke. Fresh matched measurements are now
recorded below.

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

## Real run and measurements

A disposable SS-20 Solaris 2.6 disk was booted to a root shell and allowed to
settle. The core-only run confirmed 20/20 boundary admissions and 20/20
zero-time advances, with CPU/MMU/RAM preserved and no host sleep
(`local/issue5-parking-real-2026-11-17/`).

Fresh matched headless runs used separate disposable disk/NVRAM clones and
separate output directories:

| State | Machine worker | PTY median | PTY p95 |
| --- | ---: | ---: | ---: |
| Parking off | 99.746% of one CPU | 14.263 ms | 17.612 ms |
| Parking on | 23.586% of one CPU | 14.608 ms | 29.603 ms |

The corrected CPU reduction is meaningful but usage remains above the 10–15%
target; proof failures for transient active/steady-state boundaries fall back
to normal execution. Input p95 remains below 50 ms. This is not an X/desktop or
SMP claim and not a physical keyboard latency result. The wait ordering, host
admission checks and console drain are covered by independent protocol
fixtures. Evidence is under `local/issue5-parking-measure-2026-11-17/{off,on}/`;
the measured binary hash is
`a06775bd1278bad31b53ec71616f8e15f6db4f74afd298d5348102b530230be9`.

## Validation

Heliacal passes **114 tests**, formatting, Clippy with `-D warnings`, release
build, SDL smoke and original-OBP display smoke. Independent protocol fixtures
pass 14/14. Core source remains the accepted `e3d87e3` checkpoint; its earlier
781/784 unique core tests and standard checks were not repeated because the
core source is unchanged.
