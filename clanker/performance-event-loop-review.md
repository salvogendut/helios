# Corrected performance slice and frontend event-loop review

2026-11-14. Follow-up to [the first review](performance-slice-review.md), on
the uncommitted changes on `perf/host-cpu-investigation` and sibling Heliacal.

## Resolution and commit recommendation

The three findings below are resolved in the current working trees. The SDL
wait result is dispatched before newer polled events; all picker completion
paths request redraw; both counter initializers use the low ten bits after
SETHI. There is no remaining blocker from this review to committing this
bounded performance slice in Helios and Heliacal.

Independent verification after the fixes:

- Both benchmark modes pass for 1, 4095, 4096, 5000, 8191 and 8192 iterations.
  Zero and malformed counts exit with status 2. Evidence:
  `local/review-performance-2026-11-14/verified-boundaries.log`.
- The 63 focused CGsix/SX/interrupt tests pass. Evidence:
  `local/review-performance-2026-11-14/verified-focused-tests.log`.
- The rebuilt production Heliacal ran on its own Xvfb display with native
  SS-20 CG14, original 2.25R OBP, a private NVRAM copy and no attached disks.
  Real XTEST/SDL input evaluated `decimal 1234 4321 + . cr` to `5555` on the
  guest display. After Ctrl+Enter, `1111 2222 + . cr` was ignored. Recapture
  allowed another calculation yielding `5678`. Unmap/map focus loss released
  capture, subsequent input was ignored, and recapture yielded `6789`.
  F12/Left/Return then exited with status 0. The complete run took 28.7 seconds.
  Expected guest results and absent released input were asserted against OCR
  of full-resolution guest screenshots, retained with the test script under
  `local/review-performance-2026-11-14/guest-input/`.

This adds guest-visible firmware input acceptance; it is not a new Solaris
desktop drag/resize test. The matched Solaris desktop performance runs remain
the evidence for scanout reduction. Worker idle parking and the remaining
MMU/DMA work are separate milestones. The sections below retain the original
findings and their reproduction details for reference.

The corrected SS-20 desktop measurements now compare matching phases. The
three baseline/new runs in `local/review-performance-2026-11-14/matching/`
have identical executed/interrupt/trap counts for each phase. Median wall
times match the report: 15.383703 to 14.030991 seconds without scanout;
17.435009 to 14.146030 with scanout. Desktop scanout drops from a median of
526 decodes to one, with one initial nonblank image in each run. This supports
the measured reduction; it does not establish interactive frontend acceptance.

The strengthened core graphics and interrupt suites passed independently:
34 CGsix tests, 22 SX tests and 7 interrupt tests, 63 total. This review did
not rerun the full core/frontend suites or the graphical guest acceptance.

## Blocking: dispatch the event returned by the SDL wait

`../heliacal/src/main.rs:965` ignores the return from
`events.wait_event_timeout(...)`. SDL removes that event from the queue.
The next `poll_iter()` cannot retrieve it. The first event waking a waiting
frontend is therefore lost: this can discard F9, key/button releases,
focus loss, window exposure or Quit. In particular, lost release/focus events
can leave guest input held or capture active.

The exact pinned Rust binding, SDL3 0.20.0, returns `Option<Event>` and passes
an event pointer to `SDL_WaitEventTimeout`. A minimal reproduction using the
same `/lib64/libSDL3.so.0` as the release frontend confirmed:

```text
wait returned event type=0x100 (QUIT)
following poll found no event: ignoring the wait result loses QUIT
```

Reproduction and output:
`local/review-performance-2026-11-14/sdl-wait-review.{py,log}`.
It uses only SDL's event subsystem, without a window, guest, or disk.

Retain the returned event and dispatch it through the same handler as polled
events, before draining newer events. For example, keep a `pending_event`
outside the loop, dispatch `pending_event.take().into_iter().chain(events.poll_iter())`,
then assign the wait result to `pending_event` at the end. Do not push it back
onto the SDL queue: doing so could reorder it behind later events.

Add a queue-level regression for an event arriving during the wait and verify
exactly-once delivery and ordering with the next queued event. Exercise F9,
key/button up, FocusLost and Quit through the real SDL queue. Existing smoke
checks invoke `app.key(...)` directly, so their success does not test this path.

## Correct benchmark constant materialization

Both counter initializers use `sethi(4, iterations)` followed by
`or_imm(4, 4, iterations)` (`examples/perf.rs:37,59`). The latter retains 13
bits, and SPARC sign-extends that immediate. Counts with bit 12 set load an
incorrect upper part into `%g4`.

Both workloads reproduce the checksum failure with 4096 and 5000 iterations;
1, 4095 and 8192 pass. The failing final `%g4` is `0xffffe000` instead of zero.
See `local/review-performance-2026-11-14/corrected-benchmark-boundaries.log`.
These are debug correctness runs, not throughput measurements.

Use only the low ten bits after SETHI: `or_imm(4, 4, iterations & 0x3ff)` at
both counter initializers. Preserve the general signed-immediate encoder.
Verify counts across 4095/4096 and 8191/8192 as well as the reported benchmark
sizes. The reported 2M/1.5M-iteration runs do not have this sign bit set and
their successful checksums remain valid.

## Mark asynchronous file-picker updates for redraw

The successful picker result at `../heliacal/src/main.rs:668–672` updates the
overlay draft via `app.picked(...)` and clears the picker without setting
`redraw`. A completion can arrive through the channel after focus/window
events have already been processed. With a paused machine, no notice or LED
transition, the selected path then remains stale on screen until another event.

Explicitly mark visible picker-result changes for redraw and test completion
with no additional SDL input. UI state changed by asynchronous work must
participate in the new invalidation logic.

## Acceptance before committing the frontend slice

After these fixes, rebuild and rerun the required checks, including real
queued-event delivery. Reuse the previous private-clone Heliacal acceptance
for typing, mouse drag/button release, capture/release/recapture and focus loss.
Check paused expose/restore, asynchronous file selection and UI timer expiry.
Repeat the paused CPU measurement after proving that a single input or Quit
event wakes and is handled by the application. The core slice can remain
separate from frontend acceptance; idle parking is a later milestone.
