# First performance slice review

2026-11-14. Review of the uncommitted changes on `perf/host-cpu-investigation`
at `9835122`. See [the implementation report](performance-progress.md) and
[the original plan](performance-plan.md).

The subsequent correction and remaining frontend findings are recorded in
[the event-loop follow-up](performance-event-loop-review.md). The measurements
and fixtures below describe the earlier version reviewed here.

The visual-generation and cached-interrupt changes are sensible. Inspection
found no missing interrupt-cache update or obvious omitted visible write path.
Independent reruns of `tests/cgsix.rs`, `tests/sx.rs` and `tests/interrupts.rs`
passed: 59 tests. This review did not rerun the full suite or guest acceptance.
The reported throughput and SS-20 scanout improvement need correction before
they can serve as the next performance baseline.

## Fix the fixed-work benchmark

`examples/perf.rs:17–24` initializes a counter to ten, executes ten additions
and subtractions, then reaches `ba,a .` at PC `0x18`. From step 43 onward it
executes only that self-branch. In a 10M-step run, 9,999,958 steps therefore
measure the self-branch, not the intended ALU loop.

An instrumented copy ran 10,000 steps and asserted the PC/instruction histogram:
ten additions, ten subtractions, 9,958 self-branches, `%g2 = 55`, no traps or
interrupts. The review copy lives in
`local/review-performance-2026-11-14/src/main.rs`; `alu.log` records the result.
Its debug-build timings include instrumentation and are not performance data.

The memory fixture also needs correction. Words `0xc4086000` and `0xc4286000`
encode LDUB/STB, despite their LD/ST comments. The branch at PC `0x18` targets
PC zero, resetting the counter and address on every iteration. It repeatedly
reads and rewrites byte `0x88` of its first instruction; the later addition is
discarded on the next load. The instrumented 10,000-step run confirmed 1,250
iterations of this eight-instruction loop, `%g4 = 9`, and an unchanged first
instruction. See `memory.log` alongside the ALU evidence.

Use continuous loops with a deliberate instruction mix and a separate data
region. Verify a known final register/memory checksum outside the timed region,
require zero unexpected traps/interrupts/resets, and reject malformed or zero
step budgets. Retake the three release runs after repairing the programs.

## Compare matching workloads and measurement phases

The report combines three different SS-20 phases. The baseline's 463 full
decodes and 1.313 seconds of decoding came from Xsun/olwm/xterm desktop idle.
The new one-decode result came from serial idle with an all-zero framebuffer
(`changed_frames=0`). The reported new 13.27 seconds came from the earlier
phase with scanout disabled altogether.

The raw phase values are:

| Machine and phase, 204.8M steps | Baseline wall seconds | New wall seconds | Baseline/new decodes |
| --- | ---: | ---: | --- |
| SS-20 serial idle, no scanout | 13.666446 | 13.272671 | 0 / 0 |
| SS-20 serial idle, scanout enabled | 13.750194 | 14.742839 | 1 / 1 |
| SS-20 Xsun/olwm/xterm idle, scanout enabled | 15.336704 | Not measured in the reported run | 463 / unknown |
| SS-5 serial idle, no scanout | 21.847825 | 22.538507 | 0 / 0 |
| SS-5 serial idle, scanout enabled | 22.929910 | 21.777996 | 692 / 12 |

These single runs establish neither a controlled speedup nor a regression.
The SS-5 scanout windows also differ in guest activity: synchronous traps were
1,375 before and 67,069 afterward, and differing images were 1 versus 11.
The scanout reduction is promising, but the claim that the remaining changes
are all blink needs pixel/phase evidence, not just `changed_frames` counts.

Baseline logs are in `local/performance-2026-11-13/`:
`ss20-baseline.log`, `ss20-desktop.log`, and `ss5-baseline.log`.
The new raw logs were found at `/tmp/opencode/perf5-{ss20,ss5}.err` and copied
into `local/review-performance-2026-11-14/` for this review. The review does not
modify those runs or their disks.

Rerun the same desktop command sequence, compiler/profile, source disk/NVRAM,
ROM and fixed step budgets for both revisions, with three serialized runs.
Record input hashes, command lines and phase names. Assert Xsun/olwm/xterm
startup and a known nonblank image before the desktop window. Keep serial
execution, serial scanout and desktop scanout results separate. Retain step,
trap, interrupt and image-change counts so workload drift remains visible.

## Strengthen the new scanout regressions

In `tests/cgsix.rs:832`, the post-VRAM key is compared with the original key,
before video was enabled. Video enable already changed that key, so the test
would pass even if VRAM invalidation were removed. Compare against a key saved
immediately before the VRAM write.

The new blink test changes a mask while the display remains disabled. Exercise
enabled video with distinct palette entries and a visible pixel, check both
the key and decoded pixel at a blink boundary, and check a stable key within
the same phase. Cover cursor blinking too. Extend the visible-change checks
to draw/blit/font, CG14 CLUT/XLUT stream writes, mode changes and reset. Reset
and replacement must not leave the frontend displaying a cached old frame.

## Next implementation slice

After correcting the evidence and completing the existing acceptance gates,
commit this bounded core slice. Leave the rejected ESP readiness shortcut out.
Make Heliacal presentation event driven next, as described in plan step 2:

- Redraw for changed guest frames, visible UI/LED/notice changes, exposure,
  resize/restore/fullscreen and renderer reset.
- Use bounded event waits or coalesced frame notifications, preserve prompt
  input/F9/capture/release and UI expiry, and keep device time independent of
  rendering. A continuously increasing instruction counter is not a visual
  change while the CPU LED remains lit.
- Log the actual video driver/renderer and measure UI, worker and renderer
  thread CPU separately. Rebuild Heliacal against the new core before testing.

A faster interpreter still consumes a full worker core when allowed to run an
idle loop without limits. Presentation changes address avoidable frontend
work; guarded idle parking remains a separate, later milestone.
