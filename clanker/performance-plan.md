# Performance investigation and implementation handoff

2026-11-13. Issue: [helios#1](https://github.com/salvogendut/helios/issues/1).
Investigation branch: `perf/host-cpu-investigation`.

This is an implementation plan backed by profiling, not a completed speedup.
The investigation changes no production CPU, device or frontend behavior.
Treat reducing idle host CPU use and accelerating useful guest work as separate
acceptance criteria. A faster interpreter still occupies a core when executing
an idle loop without a host pacing/parking policy.

## Baseline and evidence

Reviewed revisions:

- Helios `f560056941ca2332d2d45f424f773a129c46c55d`.
- Heliacal `d5b6737792f52e4c661313d5a80aa7c6141fee3b`.
- IRIS, local `xiri` branch, `84125cacb985f974c8f9d81150560150267fb194`.

Host: Ryzen 9 PRO 8945HS, x86-64 Linux 7.2.15-300.fc45. Rust
1.101.0-nightly, `41089d07a` (2026-10-27). Core profiles use release/thin LTO,
one codegen unit, line tables and `-Cforce-frame-pointers=yes`. Sampling uses
Linux user-mode CPU-clock events at 1 kHz per thread, excluding kernel call
chains, with zero lost records in the completed runs. CPU percentages below
use one host logical CPU as 100%, not the whole 16-thread host.

Private reflink clones booted original OBP and Solaris 2.6, reached a root shell,
and completed `sync`/`halt`. SS-20 also started Xsun, olwm and an idle xterm.
No user catalog disks were used for writes. Baseline source images were
`local/completion-2026-11-13/{ss5-final,desktop}.raw`.

### The worker continues executing idle instructions

Heliacal's running worker executes batches of 2,048 `Machine::step` calls with
no parking or rate limit. Paused workers wait on a channel. In the SS-20 serial
idle sample, **98,494 of 100,000 sampled guest PCs** were inside Solaris `idle`
or `generic_idle_cpu`. The names and ranges were independently checked against
the guest's `/platform/sun4m/kernel/unix` ELF symbols/disassembly:
`idle` starts at `f006fa08`, `generic_idle_cpu` at `f0071708`.
These addresses describe this image; they are not a portable idle detector.

The SS-20 idle window executed 204,796,545 instructions, took 2,057 interrupts
and 1,398 synchronous traps in 204,800,000 steps. Do not label every step an
instruction or count time advanced while parked as retired instructions.

### Both framebuffer invalidation schemes cause unchanged full scanouts

The harness repeated Heliacal's scanout-key check every 33 host milliseconds,
called the same core pixel decoder, and compared successive complete images.
Each window executed 204,800,000 steps. Pixel comparisons are measurement
instrumentation, not a proposed production dirty detector.

| Workload | Wall time | Full scanouts | Images differing from previous buffer | Time in pixel decoding |
| --- | ---: | ---: | ---: | ---: |
| SS-5 serial shell, CGsix installed | 22.930 s | 692 | 1 | 1.726 s |
| SS-20 Xsun/olwm/xterm idle, CG14 | 15.337 s | 463 | 1 | 1.313 s |

The one different image was the initial image, compared with an all-zero
buffer. Subsequent decoded images were byte-identical. Decoding averaged
2.49 ms for CGsix and 2.84 ms for CG14. This excludes Heliacal's subsequent
RGBA conversion, possible `Arc::make_mut` copy, texture upload and presentation.

Concrete causes:

- `Cgsix::scanout_key` incorporates the **entire retrace/blink frame counter**.
  It changes every retrace even if blinking is disabled or its visible phase
  has not changed. Register writes also invalidate broadly.
- `Sx::write` increments `generation` for the CG14 MSR acknowledgement at
  offset 4 and for SX execution/register bookkeeping, even without a visible
  change. Xsun acknowledges retrace during idle desktop operation.
- Heliacal already caches scanout and texture uploads, but those keys defeat
  the cache. `main.rs` also draws/presents every approximately 16 ms regardless
  of whether the image, overlay or status changed.

### Frontend redraw has a separate cost

An isolated, **diskless** production Heliacal ran on Xvfb, with X11 selected and
software rendering requested. Mesa llvmpipe worker threads were observed.
The actual selected SDL renderer name is not currently logged. These results
describe this software-rendered environment, not the user's normal GPU path.
Each steady measurement interval was approximately 12 seconds:

| State | Main thread | Machine worker | llvmpipe workers, combined |
| --- | ---: | ---: | ---: |
| Paused, no machine constructed | 17.1% | 0.1% | 64.2% |
| Running diskless OBP, after 25-second warmup | 21.2% | 99.4% | 56.9% |
| Paused, machine retained | 25.3% | 0.2% | 78.2% |

Thus pausing correctly stops guest execution, but continuous presentation can
still be expensive with a software renderer. Record the real desktop backend
before attributing these exact numbers to normal Heliacal use.

### Throughput profiles identify work before a JIT

Approximate **inclusive** user-CPU sample shares during boot-to-shell, including
the login/identification commands. Parent/child percentages overlap and must
not be added to claim a speedup:

| Path | SS-5 | SS-20 |
| --- | ---: | ---: |
| `Srmmu::translate` | 48.4% | 28.8% |
| `Srmmu::walk` | 20.2% | below the leading SS-20 paths |
| Full TLB search on a host-cache miss | 15.1% | below the leading SS-20 paths |
| `Srmmu::record_diag_view` | inactive for this profile | 12.1% |
| `SystemBus::read32` | 20.7% | 11.3% |
| `SystemBus::interrupt_levels` | see raw profile | 7.8% |
| `Machine::service_scsi_batch` | see raw profile | 8.3% |

`record_diag_view` performs six `HashMap` lookups/conditional inserts on eligible
ordinary data translations, including host-TLB hits. It models POST-visible
state and must be optimized without removing that state. SS-5 instead spends
substantial time searching/walking the MMU; investigate its miss causes before
assuming another larger host cache will solve them.

**SS-5 follow-up: replacement-state investigation takes priority over host-cache
tuning.** An isolated counter-only source copy measured this 40,960,000-step
idle interval:

| Translated access class | Requests | Fast host-cache hits | Full TLB-search hits | Page-table walks |
| --- | ---: | ---: | ---: | ---: |
| Instruction | 40,959,587 | 26,683,691 | 48 | 14,275,848 |
| Data | 8,207,008 | 0 | 846 | 8,206,162 |

There were 22,482,010 leaf insertions but only eight diagnostic/flush-generation
refreshes. Nearly all misses had the expected host-cache address/privilege tag
but an obsolete generation. Importantly, the full architectural TLB search also
missed: preserving host entries across genuine hardware eviction would merely
hide the problem.

A separate write trace observed OBP program TRCR from `0x00001f80` to zero and
then `0x00100000` (VP set, WP=0, TC=0), still present at Solaris idle. The second
run reproduced zero data-side fast hits and more than 99.98% data-side walks.
With the current `insert` implementation,
WP=0 means `limit=0`, so CPU leaf fills repeatedly replace slot zero. This is a
concrete explanation for the observed thrashing, **not proof of the correct
silicon interpretation of WP=0**. Check the firmware sequence, replacement and
ordinary cache behavior against the microSPARC-II sources before changing it.
Do not silently force WP=63, disable eviction, or assume the related IIep chip
defines this SS-5 behavior.

`InterruptController::levels` remaps 15 hardware sources to PILs on every CPU
boundary. `service_scsi_once` polls ESP and synchronizes interrupt sources even
when no transfer is returned. RAM reads traverse multiple device decoders
before DRAM bank decoding and `MemoryBus` region lookup.

These are exploratory profiles, not controlled before/after benchmark claims.
Sampled builds, counter instrumentation, host scheduling and guest boot paths
affect absolute timing. Some setup/symbolization work overlapped runs. Repeat
unprofiled, serialized runs before accepting a performance patch.

## What is already implemented

Do not reimplement the previous [performance work](performance.md):
separate 256-entry host instruction/data translation caches, 2 microsecond
device-time accumulation, 500 ns timer ticks, up to 16 SCSI DMA bytes per
instruction boundary, idle-mouse avoidance, batched Heliacal input/network
polling, 33 ms snapshot publication, and generation-based scanout/texture reuse.

`STEP_NS` is a **100 ns functional time quantum**, not a conversion from the
advertised 85 MHz SS-5 clock to cycle-accurate execution. Changing it is not a
performance optimization. Keep the interpreter, device ordering, serial pacing
and interrupt rules as the correctness reference.

## Implement in this order

Make small, separately measured commits. Preserve an option/reference path for
larger scheduling or CPU experiments; report rejected experiments as well as
wins. The first four work packages can improve visible CPU cost and boot speed
without introducing an idle heuristic or JIT.

### 1. Establish repeatable workloads and counters

Promote a small dependency-free headless benchmark from the local harness.
Inputs must be supplied ROMs, NVRAM and disposable disk clones. Record revisions,
compiler/profile, machine/display, media identities, network setting, renderer,
host CPU time and wall time. Use fixed-work ALU/branch/load-store/MMU workloads
as well as boot-to-login and a deterministic scrolling/window-move workload.
Use incremental serial matching, bounded polling and explicit step/trap/IRQ/
error counts. Never scan the complete transcript after every instruction.

Add opt-in aggregated counters for host-TLB hit/miss reasons, hardware TLB
search/walk/fill/flush, IRQ recomputation, empty/active DMA service, scanout,
RGBA copy, uploaded frames and presented frames. Avoid per-instruction logging
and atomics in the normal interpreter path. Log the SDL video driver and actual
renderer once; optional diagnostics belong behind Tinker/developer controls.

Acceptance: three serialized unprofiled runs per workload, medians and spread;
profiling only for attribution. Keep cold boot and steady idle distinct. Idle
steps/s alone is not an improvement metric. Final coverage must include SS-5,
SS-20 and Axil, both framebuffer types, and network enabled/disabled.

### 2. Repair display invalidation, then make presentation event-driven

Core files: `src/devices/cgsix.rs`, `src/devices/sx.rs`.
Frontend files: `../heliacal/src/{emulator,main,view,activity}.rs`.

Use a visual-change generation/key. For CGsix, include only the effective
blink phase when configured pixel/overlay/cursor blinking can affect scanout.
For CG14, status acknowledgement and nonvisual SX register writes must not
dirty the display. VRAM writes/drawing, palette/XLUT/CLUT changes, cursor shape/
position/color, video enable and pixel mode still invalidate as appropriate.
Keep retrace time, pending bits, acknowledgement and video IRQs fully active.
Do not hide display changes by suppressing device advancement.

Next, redraw/present only for changed guest frames, visible UI/LED/notice state,
expose, resize, restore, fullscreen or renderer reset. Track the **visible LED
mask**, not merely rising instruction counters, or the lit CPU LED will force
continuous redraw. Cache SDL grab/relative-mode/scale setters. Use a main-thread
event wait with bounded timeout or a coalesced worker notification; worker
commands/input/network wakeups must remain prompt. Presentation must not
control guest time. Keep mailbox/frame ownership bounded.

Tests: equal keys after nonvisual writes; new keys on every visible cause;
CGsix blink phase boundaries with blink masks on/off; CG14 pending/ack/enable
IRQ behavior; reset and machine replacement cannot reuse stale cached pixels.
Repeat the accepted text/scroll/move/resize/uncover/iconify captures. Exercise
F9, typing, mouse capture/release, notices/LED expiry, minimize/restore and
window exposure while paused. Target near-zero steady redraws and, on the same
Xvfb setup, less than 5% of one CPU while paused after animations settle.

Only after this, measure whether dirty rectangles, cursor-only composition,
buffer reuse or direct RGBA scanout are worthwhile. The current first-order
problem is decoding/presenting unchanged images.

### 3. Remove repeated interrupt decoding and inactive DMA work

Files: `src/devices/interrupt.rs`, `src/system_bus.rs`, `src/dma.rs`, and ESP/
Ethernet device state.

Cache the delivered PIL bitset, updating it only when a source, mask, software
request, CPU timer or module-error state changes. A repeated `set_source` with
the same level should be cheap. Preserve pending-register visibility separately
from delivery masks. Multiple sources sharing a PIL must OR correctly; clearing
one must not hide another. Cover mask-all, unmask of an already pending source,
CPU timer, module error and reset.

Introduce an exact cheap `needs_service`/active-state guard for DMA engines.
ESP can progress commands and non-DMA operations, so **DMA enable or a current
DRQ alone is not a sufficient guard**. MMIO writes, command queues, timeout
expiry, device completions and incoming Ethernet frames must reactivate service.
Keep SCSI's 16-byte bound and byte-level count/address/fault ordering initially.
Do not enlarge polling intervals as a substitute for tracking readiness.

Acceptance: unchanged step-boundary IRQ/trap sequences, ESP selection/stacked
IRQ regressions, disk/CD boot, read/write/checksum plus clean reboot, and
Ethernet/serial responsiveness. Show fewer empty polls and recomputations in
the profiles; target a repeatable throughput gain, with no boot regression.

### 4. Optimize MMU and ordinary RAM paths from the measured miss causes

Files: `src/mmu.rs`, `src/system_bus.rs`, `src/dram.rs`, `src/bus.rs`.

Start with the SS-5 TRCR trace and counter results above. Establish independent
expected behavior for zero/nonzero WP, TC freeze, IL/PL restrictions and reset,
and explain the OBP-programmed value before choosing a correction. The current
documentation calls modulo replacement a functional choice; it should not be
treated as independent hardware evidence. Separate an actual model correction
from a host optimization, with its own regression and boot measurements.

For SS-20/Axil, replace hot diagnostic `HashMap` operations with bounded storage
for documented diagnostic slots/aliases, retaining a fallback if the existing
supported diagnostic address space needs it. Preserve fill-if-zero behavior,
software-written entries, context/page flushes, reset, PTP2 views and all POST
regressions. Simply skipping diagnostic maintenance or making it a build-time
debug feature would change hardware-visible behavior.

For SS-5, measure architectural TLB misses versus host-cache tag/generation
misses, replacement controls and guest-issued flushes. Every current insertion
bumps a global host-cache generation. If that needlessly evicts unrelated hot
translations, consider slot generations plus correct overlapping-map
invalidation. A new narrow mapping must supersede a cached broad mapping;
tagging the old slot alone does not ensure this. Do not enlarge the emulated
hardware TLB, ignore flushes, or retain translations after real eviction.

After re-profiling, try an early low-RAM dispatch before unrelated MMIO tests,
while retaining DRAM mirrors, absent-bank behavior, request observation and
error capture. A host RAM-page access cache must store validated mapping/region
information, respect changes, widths and permissions, and keep the full slow
path for devices/ROM/diagnostics/faults. Keep word-transparent data and all
doubleword/atomic fault-atomicity guarantees. No blanket direct pointer shortcut
through the 36-bit SS-20/Axil map or MXCC/SX aliases.

Acceptance: differential cached/uncached execution, both MMU configurations,
context/supervisor changes, R/M bits, no-fault mode, overlapping page sizes,
diagnostic writes/flushes, DMA/coherency and bank boundaries. Boot all three
machines and rerun the desktop acceptance before retaining a faster path.

### 5. Add deadline-aware idle parking as an explicit, guarded mode

This is what can stop a running, idle guest from occupying a host core. It is
more involved than adding `sleep` to the existing loop.

First expose deterministic next-event/deadline information for timers, RTC,
CGsix/CG14 retrace, keyboard/mouse byte delivery, ESP and Ethernet. Account for
pending accumulated device time and PROM wait-state time. Advancing a sleeping
CPU must deliver events in order and stop at an event that needs CPU service.
Keep an unpaced headless mode; the frontend owns host waiting and wakeup policy.

Then add conservative, opt-in idle recognition with recorded evidence. A known
Solaris code signature plus validated supervisor/PSR.ET/PIL, PC/**nPC**, run-queue
and side-effect conditions is safer to start with than assuming any repeated
PC means idle. The observed SS-20 idle helper and SS-5 path differ. Unknown
firmware/OS versions must fall back to execution. Hardware wait/powerdown paths
may be used only if independently documented and correctly implemented.

Never park ordinary user loops, masked-interrupt waits, PROM polling, active
DMA or unclassified kernel loops. Sleeping the only machine owner without
advancing device time prevents the timer needed to wake it. Wake on input,
pause/reset/shutdown, serial/network arrivals and the next device deadline.
Use a race-safe announce/check/wait sequence; input currently uses a mutex
queue and will need to signal the sleeping worker. Network socket progress
must be included even when no external event thread currently exists.

Acceptance: on/off guest-visible equivalence at event boundaries, negative
idle-detection fixtures, timer/RTC progress, serial and network traffic while
idle, prompt input and shutdown, and sustained idle/login/X tests on all three
profiles. Proposed target: idle worker below 10–15% of one CPU, input latency
below 50 ms, no measurable boot slowdown beyond baseline spread. Report time
parked and wake reasons. These targets are goals, not results already achieved.

### 6. Evaluate decoded execution, then a JIT only if still justified

After the above work, profile again. Try a compact predecoded instruction cache
without bypassing instruction fetch/permission checks; validating the freshly
fetched instruction word is a simple first coherence strategy. Preserve dynamic
register-window aliases, condition codes, delay/annul behavior, PC/nPC, traps
and retryable host stops. Avoid a large decoded structure or indirect dispatch
that loses more host cache/branch prediction performance than it saves.

A later block engine/JIT needs code-page generations shared by all CPU and DMA
writes and aliases, context/mapping/privilege validation, bounded event budgets,
and exact architectural state at every fault/interrupt exit. Keep the interpreter
as an independently tested fallback and differential reference. IRIS's Cranelift
JIT is useful design evidence, not reusable SPARC code. Host JIT memory/execution
also needs a deliberate dependency and `unsafe_code = "forbid"` boundary design.
Do not start with multiprocessing, device threads or a whole-page JIT just to
solve an idle-power problem.

## IRIS and primary references

Read these at the IRIS revision above, not as claims about an unspecified build:

- `../iris/src/cpu/idle_park.rs`: PC sampling, privilege/IRQ guards, warmup,
  backoff, counters and race-safe wakeups. **`idle-pause` is opt-in and disabled
  by default** in that checkout. Its comments document false positives slowing
  boot; do not transplant IRIX cluster thresholds or wall-clock rules to Solaris.
- `../iris/src/cpu/mips_exec.rs`: compact 24-byte `DecodedInstr`, handler and
  operand caching, batched dispatch, interpreter fallback. Its architecture-
  specific delay-slot fusion is not a SPARC correctness argument.
- `../iris/HACKING.md`, `docs/{ppmem-design,tcache-design,nutlb-design}.md`:
  direct physical mapping and tagged translation shortcuts. The tcache note
  records a near-parity interpreter experiment, not a universal speedup.
- `../iris/src/cpu/jitv2/` and `HACKING.md` section 8: background compilation,
  physical-page generations, alias/DMA coherence and differential checks.
- [QEMU instruction counting](https://www.qemu.org/docs/master/devel/tcg-icount.html):
  event-limited instruction budgets and explicit handling of MMIO timing.
  Useful scheduling design reference; it is not permission to change Helios's
  time model.
- [QEMU translator internals](https://www.qemu.org/docs/master/devel/tcg.html):
  block state assumptions, invalidation and fast RAM versus MMIO paths; CPU
  state changes that can unmask interrupts require rechecking delivery.
- SDL3 [event waits](https://wiki.libsdl.org/SDL3/SDL_WaitEventTimeout),
  [renderer name](https://wiki.libsdl.org/SDL3/SDL_GetRendererName) and
  [video driver](https://wiki.libsdl.org/SDL3/SDL_GetCurrentVideoDriver): event
  waits/video-driver queries belong on the main/video thread; waits can overrun
  their requested timeout under host scheduling.
- [microSPARC-II User's Manual](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf),
  sections 5.2.1 and 5.6.6, printed pages 65 and 88–90: replacement counter,
  wrap and lock controls. The inspected text does not explicitly resolve the
  WP=0 corner. Sections 2.2/5.12.1 describe DRAM page-hit registers; those are
  not evidence for retaining a stale CPU translation after TLB eviction.

Continue using the SPARC/manual and MAME/QEMU hardware revisions pinned in
[docs/references.md](../docs/references.md) for architectural behavior. The
current upstream QEMU design documents above are additional performance-design
references, not a replacement hardware specification.

## Reproduction and completion checks

Local evidence and exploratory tools are in `local/performance-2026-11-13/`:

- `src/main.rs`, `Cargo.toml`, `desktop.commands`: boot/idle/scanout harness.
- `sample.py`, `report.py`: Linux CPU-clock sampler and inline-aware reporting.
  These are local investigation tools, not a supported cross-platform profiler.
- `ss5-baseline.*`, `ss20-baseline.*`, `ss20-desktop.*`: logs, serial transcripts,
  raw samples, symbols and reports. Retain the matching executables for symbols.
- `gui_measure.py`, `gui-software.*`: diskless Xvfb test, per-thread CPU ticks and
  phase timestamps. It exercises the existing production Heliacal executable.
- `core-counts/`, `counts/`, `ss5-counts.*`, `ss5-trcr.*`: separate instrumented
  source copy and MMU-counter experiment; never use its timings as production
  throughput. Counter order is I requests/fast hits/search hits/walks, then the
  same four D counts, leaf insertions, diagnostic-generation refreshes, I/D
  generation-only misses, I/D tag-or-privilege misses.

Use [build-environment.md](build-environment.md) for the local toolchain. The
local harness is run from the Helios root, with a writable **scratch copy**:

```sh
RUSTFLAGS='-Cforce-frame-pointers=yes' cargo build --release --offline \
  --manifest-path local/performance-2026-11-13/Cargo.toml
python3 local/performance-2026-11-13/sample.py OUTPUT_PREFIX \
  local/performance-2026-11-13/target/release/helios-perf-investigation \
  ss20 SCRATCH_DISK SERIAL_NVRAM OUTPUT_PREFIX \
  local/performance-2026-11-13/desktop.commands
```

Omit the commands argument for serial-idle measurements. SS-5 mode uses saved
keyboard/screen NVRAM and switches OBP via actual `ttya io` keystrokes; SS-20
uses `local/ss20-ttya.nvram`. Make fresh reflink copies from the retained clean
sources for controlled repetitions. Preserve media/config and compiler hashes,
and symbolize against the exact executable before rebuilding it. Normal Linux
`perf record -e cpu-clock:u -g` is an alternative where available/permitted.
The local `report.py` accepts an optional second argument overriding the main
executable used for symbols: use `serial-baseline.elf` for the original SS-20
serial profile and `ss20-desktop.elf` for the desktop profile. `inputs.sha256`
records firmware, initial NVRAM and retained clean source-disk identities.

For each implementation commit, run the changed component's meaningful
regressions and required repository checks. Core: `cargo fmt --all --check`,
`cargo clippy --locked --offline --all-targets -- -D warnings`,
`cargo test --locked --offline`, and `demo`, `trap-demo`, `serial-demo`,
`timer-demo`. Keep Rust 1.74 compatibility. Heliacal: package format, Clippy,
tests, SDL smoke, real firmware smoke and release rebuild against the new core.
The previous milestone had 499 core tests and 29 Heliacal tests; do not call
those a fresh validation of future optimization changes.

Merge only after fresh SS-5/Axil/SS-20 boot-to-login plus clean writable-disk
reboot and the CGsix/native-CG14 desktop/input checks. Report performance
medians/spread, host CPU use, renderer, correctness evidence and remaining
unsupported behavior. A quiet host obtained by slowing boot, dropping input,
freezing guest timers or hiding repaint errors fails acceptance.

For this documentation-only investigation, fresh core formatting, Clippy with
`-D warnings`, all 499 tests and all four demos passed. The network fixtures
needed execution outside the restricted socket sandbox. All five guest probe
runs reached a clean halt. Heliacal production code/binary was unchanged;
the isolated GUI measurement exited normally. Local documentation links and
all three catalog machines' firmware/media paths were checked after cleanup.

## Session artifact cleanup

Removed 13 ignored `local/review-*/target` build trees and five superseded
scratch disks from the SX execution, hit-test and mouse investigations, after
checking that no visible same-user process had them open. The exact inventory
is `local/performance-2026-11-13/cleanup-manifest.json`.

Retained the three-machine Heliacal catalog, firmware, source/accepted disks,
NVRAM, regression harness sources, accepted captures, current release binaries
and this investigation's evidence. Removed entries accounted for approximately
2.18 GiB of allocated file extents; reflink sharing means this is **not** a claim
that 2.18 GiB of unique physical storage was reclaimed.
