# Performance work progress

Started from the measured handoff in [performance-plan.md](performance-plan.md).
This note records implementation evidence; it is not a claim that the full
performance plan is complete.

## Implemented

- CGsix now has a separate visual scanout generation. Retrace acknowledgement,
  nonvisual FBC/TEC/timing writes and unrelated reads do not invalidate the
  image. The key includes the effective blink on/off phase only when enabled
  video and configured pixel/cursor masks can affect scanout. VRAM, palette,
  cursor, video enable and draw/blit/font changes still invalidate.
- CG14/SX now bumps its published generation only for direct VRAM writes, SX
  memory stores, CLUT/XLUT data, cursor control and video-mode changes.
  Retrace acknowledgement, SX register bookkeeping, status/GO writes and SX
  instruction-queue register operations no longer force a full decode.
- `InterruptController` caches the delivered PIL bitset and recomputes it only
  when sources, masks, software requests, the CPU timer or module error change.
  Repeated assertions of the same source are cheap.
- Heliacal now waits with a bounded SDL event timeout when no visible state has
  changed. It redraws only for changed guest frames, visible LED/notice changes,
  UI events, exposure/resize/fullscreen events, capture changes and worker state
  transitions. SDL grab/relative-mode/scale setters are cached. The actual
  video driver and renderer are logged once at startup. The event returned by
  the bounded wait is retained and dispatched before polling newer events;
  asynchronous file-picker completion explicitly requests a redraw. Machine
  replacement clears the scanout cache and published pixels so a new machine
  cannot reuse an old frame.
- Added `examples/perf.rs` with continuous fixed ALU and load/store loops. It
  verifies final registers, memory and zero traps/interrupts, rejects zero or
  malformed iteration counts, and times only the loop body.
- Added scanout regressions for enabled pixel/cursor blink, VRAM/draw/blit/font,
  CG14 CLUT/XLUT/mode changes, retrace acknowledgement and reset.

The attempted ESP `needs_service` guard was rejected: skipping inactive service
stalled Solaris boot. A correct readiness model remains part of the plan.

## Measurements

Fixed-work benchmark, three serialized release runs on the profiling host:

```text
ALU, 10,000,000 loop steps:  34.63, 34.00, 35.81 ns/step
load/store, 10,500,000 steps: 46.99, 48.31, 45.86 ns/step
```

The two workloads use 2M/1.5M iterations (10M/10.5M loop steps) and final
checksum assertions, including counts above the 13-bit immediate range; these are interpreter-only numbers. Raw rerun logs are under
`local/review-performance-2026-11-14/`.

Guest measurements used the same SS-20 source disk, NVRAM, ROM and
`desktop.commands` for both revisions. Desktop values are medians of three
serialized runs; serial values are one controlled run per revision.

| SS-20 phase, 204.8M steps | Baseline | New | Decodes baseline/new |
| --- | ---: | ---: | ---: |
| Serial idle, no scanout | 14.287 s | 13.859 s | 0 / 0 |
| Serial idle, scanout | 14.557 s | 13.966 s | 1 / 1 |
| Xsun/olwm/xterm idle, no scanout | 15.383 s | 14.031 s | 0 / 0 |
| Xsun/olwm/xterm idle, scanout | 17.435 s | 14.146 s | 526 / 1 |

The desktop runs had identical executed/trap/interrupt counts, one initial
changed frame, and Xsun/olwm/xterm present in both serial transcripts. The
baseline median scanout decode time was 1.742 s; the new median was 0.00285 s.
Serial scanout also remained one decode on both revisions.

For SS-5 serial idle (one controlled run per revision):

| Phase | Baseline | New |
| --- | ---: | ---: |
| No scanout | 25.369 s | 24.105 s |
| Scanout | 27.118 s, 818 decodes | 23.366 s, 12 decodes |
| Scanout decode time | 2.309 s | 0.038 s |

Both SS-5 runs had identical executed/interrupt counts and 67,069 synchronous
traps; the baseline reported 10 changed frames and the new run 11. The remaining
scans are visible blink transitions, covered by pixel/key tests. These single
runs establish matching-phase scanout reduction, not a controlled speedup claim.

Paused Heliacal on Xvfb with software rendering measured `0.0%` main-thread CPU
over 10 seconds after startup; the logged renderer was `x11/software`. The
earlier baseline main-thread figure was 17.1% with llvmpipe workers. This shows
event-driven waiting removed the continuous paused repaint loop; it is not a
claim about a live GPU desktop.

## Validation and remaining work

- Core formatting, Clippy with `-D warnings`, and all four demos pass.
- Core test suite passes with **514 tests** normally and **516 tests** with `perf-counters`, including the promoted diagnostic-cache regressions.
- SS-5, SS-20 and Axil serial boot-to-shell, idle measurement and clean halt pass. Axil used its cloned disk and NVRAM.
- Heliacal `cargo test` (29 tests), SDL smoke test and real-firmware
  `--boot-smoke-test` pass; the real-firmware screenshot was generated.
  The same source disk/NVRAM/ROM and `desktop.commands` were used for both
  revisions; baseline executable `local/performance-2026-11-13/ss20-desktop.elf`
  and the current `local/performance-2026-11-13/target/release/helios-perf-investigation`.
  The event loop was measured paused on Xvfb/software for 10 seconds after
  startup; the main thread reported 0.0% CPU and the log showed `x11/software`.
  An actual SDL event run booted with Ctrl+P, captured/typed/clicked, released
  and recaptured with Ctrl+Enter, exercised a window unmap/map focus loss, and
  exited through F12/Left/Return with status 0.
- A correct ESP/DMA readiness guard, further MMU/TLB work, idle parking and
  decoded execution/JIT evaluation remain.
- Independent review also verified guest-visible typing on diskless SS-20 OBP:
  captured calculations returned `5555`, `5678` and `6789`; released input was
  ignored, focus loss released capture, recapture worked and Quit exited 0.
  This is firmware input acceptance, not a new Solaris desktop interaction
  run. See [the resolved event-loop review](performance-event-loop-review.md)
  and `local/review-performance-2026-11-14/guest-input/` for the asserted
  screenshots, script and output.

## MMU counters and diagnostic-view hotspot

Added the opt-in `perf-counters` feature. With it enabled, `Srmmu` counts
instruction/data translation requests, fast-cache hits, architectural TLB hits,
walks, replacements, flushes and diagnostic-view updates. The fields and every
increment are absent unless the feature is enabled. Counters are plain
single-owner `u64`s and are reported outside the execution loop by the local
harness through `Machine::translation_counters()`.

The SS-20/Axil diagnostic view now keeps a bounded 64-entry per-region cache
for the six documented ASI 5/6 fill slots. A region is skipped only after each
nonzero tag/context/PTE value has been installed; zero values remain fillable.
Diagnostic writes, demux flushes and reset invalidate the cache, so guest-visible
diagnostic state is unchanged.

Counters are cumulative from MMU construction/reset. The following table uses
fieldwise deltas for the second 204.8M-step `serial_idle_scanout` window, with
equal instruction counts, traps and interrupts in both variants:

| Counter, 204.8M steps | Cache disabled | Cache enabled | SS-5 |
| --- | ---: | ---: | ---: |
| Instruction requests | 204,797,882 | 204,797,882 | 204,797,643 |
| Data requests | 43,977,446 | 43,977,446 | 45,337,026 |
| Fast hits | 248,665,530 | 248,665,530 | 137,931,212 |
| Architectural hits | 101,452 | 101,452 | 227,304 |
| Walks | 8,346 | 8,346 | 111,976,153 |
| Replacements | 8,500 | 8,500 | 112,008,165 |
| Flushes | 154 | 154 | 3,362 |
| Diagnostic fill-path executions | 43,977,446 | 1,101 | 0 |

The second-window wall medians were 12.484 s before and 10.707 s after. The
first idle-window medians were 12.464 s and 10.681 s; boot-to-settle medians
were 68.311 s before and 58.034 s after (15.0% reduction). These figures are
instrumented-run measurements, not cycle-accurate claims. The SS-5 row is a
second-window delta; the earlier 768M figure was cumulative through boot and
must not be used as an idle-walk rate.

A feature-off build was also run three times for the first serial-idle phase:
11.040, 11.034 and 11.020 s (median 11.034 s). The feature-on median was
10.681 s under the same host conditions; no counter-overhead regression was
observed, but this is a host-variable comparison rather than a cycle claim.

Counter units documented on the public API: requests exclude boot-PROM,
physical-bypass and disabled-MMU accesses; hits/walks combine instruction and
data; replacements include empty-slot and I/O fills; diagnostic updates count
fill-path executions, not map writes. Counter snapshots reset with the MMU and
are excluded from MMU equality.

Build the counters explicitly:

```sh
cargo build --release --features perf-counters
cargo test --locked --offline --features perf-counters
```
