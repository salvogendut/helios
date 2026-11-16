# Issue #5 Slice A: performance baseline and execution-engine handoff

Measured 2026-11-15, corrected 2026-11-16 on current `main`
`cc91f43973047e2a28c4b5039e226acd5e9e6b65`. This is a baseline/design slice
only. No interpreter, MMU, device or JIT behavior was changed. The tracked
source changes are host-CPU accounting and a single-CPU entry-path comparison
mode in `examples/perf.rs`.

## Environment and build settings

- Host: AMD Ryzen 9 PRO 8945HS, 16 logical CPUs, Linux 7.2.15-300.fc45.
- Rust: 1.101.0-nightly `41089d07a` (2026-10-27); Helios `rust-version = "1.74"`.
- Root and local benchmark manifests both use `[profile.release] lto = "thin"`
  and `codegen-units = 1`; `opt-level = 3` is Cargo's release default. No
  `RUSTFLAGS`, target-CPU override or PGO was used.
- Builds used `--release --locked --offline`. The local harness enables the
  `perf-counters` core feature; the tracked `examples/perf.rs` does not.
- The boot harness now uses production dispatch: `Machine::step()` for one CPU
  and `Machine::step_scheduled()` only for two CPUs. It previously used
  `step_scheduled()` for every configuration, which made the single-CPU baseline
  non-production.
- The fixed examples and entry-path comparison use `Machine::step()`; boot runs
  were serialized and not CPU-pinned. Fixed samples were pinned to otherwise
  idle CPU 6 after warmup.
- Guest workloads used disposable reflink clones of:
  - `local/completion-2026-11-13/ss5-final.raw`
  - `local/completion-2026-11-13/ss20-trace.raw`
  - `local/firmware/{sun-ss5.bin,ss10-20-v2.25r-rom,turbogx.bin}`
- SS-5 uses its screen-console NVRAM, switched to TTYA by the harness. SS-20
  uses the retained TTYA image `local/performance-2026-11-13/paused.cfg.nvram.ss20`.

## Workloads and method

The boot harness records phase wall time, process user+system CPU ticks from
`/proc/self/stat`, scheduler outcomes, and per-CPU MMU counters. Phases:

1. firmware/OBP prompt (SS-5 includes the keyboard console switch)
2. `boot disk` to Solaris `login:`
3. root login
4. useful work: 16 MiB write, checksum and read-back through the emulated SCSI disk
5. 100 million-outcome idle window at the shell
6. clean `sync` / `halt`

Boot measurements are one controlled run per configuration, not a statistical
study. Fixed-work samples are three serialized runs. The sampling run is used
for attribution only; its wall time is not a baseline.

```sh
cargo build --release --locked --offline --example perf
cargo build --release --locked --offline \
  --manifest-path local/issue5-perf-2026-11-15/Cargo.toml
taskset -c 6 ./target/release/examples/perf fixed alu 2000000
taskset -c 6 ./target/release/examples/perf fixed memory 1500000
./target/release/examples/perf compare alu 2000000
./target/release/examples/perf compare memory 1500000
taskset -c 6 local/issue5-perf-2026-11-15/target/release/device_compare
taskset -c 6 local/issue5-perf-2026-11-15/target/release/mmu_compare
local/issue5-perf-2026-11-15/target/release/issue5_perf ss5 1 ...
local/issue5-perf-2026-11-15/target/release/issue5_perf ss20 1 ...
local/issue5-perf-2026-11-15/target/release/issue5_perf ss20 2 ...
```

```sh
cp --reflink=auto --sparse=always local/completion-2026-11-13/ss5-final.raw /tmp/ss5.raw
cp --reflink=auto local/completion-2026-11-13/ss5-final.nvram /tmp/ss5.nvram
cp --reflink=auto --sparse=always local/completion-2026-11-13/ss20-trace.raw /tmp/ss20.raw
cp --reflink=auto local/performance-2026-11-13/paused.cfg.nvram.ss20 /tmp/ss20.nvram
```

## Fixed-work results

Three runs after matching release/profile settings, medians:

| Workload | Steps | Median ns/step | Host CPU |
| --- | ---: | ---: | ---: |
| Bare ALU loop | 10,000,000 | 42.69 | ~0.42 s, ~0.98 of one CPU |
| Bare load/store loop | 10,500,000 | 54.85 | ~0.57 s, ~0.99 of one CPU |
| Same ALU loop, full `sun4m` devices | 10,000,000 | 88.42 | ~0.88 s |
| One MMU fast data load | 10,000,000 loads | 11.69 total, 11.07 above direct RAM | — |

Interpretation:

- The bare dispatch/execute floor is about **42.7 ns/step**; adding RAM accesses
  raises it to about **54.9 ns/step**.
- Installing the full sun4m device set on the same ALU loop adds about
  **46.1 ns/step**. This is a composite of device time advancement, interrupt
  polling/synchronization, DMA/SCSI readiness and reset/watchdog checks, not one
  device's cost. The earlier 105.9/63 ns numbers came from the pre-profile local
  binary and are retired.
- A fast-cache MMU data access costs about **11.1 ns** above a direct RAM load
  in this synthetic fixture. Page-table walks dominate the SS-5 guest result.

The single-CPU entry-path comparison harness reports `state=match` for both ALU
and memory workloads: final PC/nPC/PSR, the active register view, RAM checksum,
steps, traps, interrupts and resets agree between `step()` and
`step_scheduled()`. This is a **smoke comparison**, not full-state validation:
it omits inactive register windows, MMU/FPU state, device state and timing. It
does not add a decoded cache or JIT; comprehensive differential validation is
the next slice.

## Guest phase results

Wall and host CPU are in seconds. `host_cpu` is process user+system time, not the
whole host. `steps` are scheduler outcomes, not per-CPU retired instructions.

| Configuration | Firmware/prompt | Solaris to login | Root login | Useful 16 MiB I/O | 100M idle | Halt | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| SS-5, 1 CPU | 11.47 | 122.06 | 3.38 | 64.23 | 10.98 | 17.61 | 229.73 |
| SS-20/SX, 1 CPU | 11.95 | 90.52 | 0.89 | 31.85 | 7.22 | 11.85 | 154.28 |
| SS-20/SX, 2 CPUs | 12.53 | 144.93 | 1.72 | 62.28 | 7.52 | 25.18 | 254.16 |

Host CPU tracked wall closely: **225.79 / 151.39 / 250.49 s**, or **98.1–98.6%
of one CPU**. There is no idle host-CPU saving in the headless worker: it
continues executing the guest idle loop.

Per-outcome costs from the **phase deltas**, not cumulative step counts:

| Phase | SS-5 | SS-20 1 CPU | SS-20 2 CPUs |
| --- | ---: | ---: | ---: |
| Solaris boot-to-login | 118.39 ns | 81.99 ns | 85.17 ns |
| Root login | 126.85 ns | 84.47 ns | 89.75 ns |
| 16 MiB disk work | 124.42 ns | 80.12 ns | 82.96 ns |
| 100M idle | 109.77 ns | 72.16 ns | 75.16 ns |
| Halt | 113.91 ns | 77.35 ns | 81.35 ns |

The retained pre-correction run (single-CPU `step_scheduled()` and the old local
build) recomputes to **151.07 / 94.98 / 101.67 ns/outcome** for Solaris
boot-to-login, confirming the correction. After matching the production
single-CPU path and release profile, the same arithmetic gives the lower values
above. The old logs remain in `runs-old/` for audit.

The 16 MiB I/O phase is dominated by emulated SCSI/device and guest filesystem
work rather than the shell; it is a useful-work regression workload, not an
instruction-throughput metric.

## Per-phase deltas and per-CPU counters

Counters are deltas between consecutive `PHASE` markers. They exclude boot-PROM
mapping and physical-bypass accesses; `fast_hits` is combined instruction+data.
SS-5 and SS-20/1 have only CPU0. SS-20/2 reports CPU0 and CPU2 separately. Exact
unrounded values, including TLB hits and flushes, are in
`runs/phase-counters.csv`; final aggregates are in `runs/final-counters.txt`.

### SS-5, 1 CPU, CPU0

| Phase | Δ outcomes (M) | Wall s | ns/out | I-req M | D-req M | Fast M | Walks M | Repl M | Diag M |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| firmware | 61.60 | 6.915 | 112.26 | 59.82 | 12.91 | 40.23 | 32.40 | 32.40 | 0.00 |
| prompt | 50.80 | 4.553 | 89.63 | 50.80 | 4.70 | 43.64 | 11.82 | 11.82 | 0.00 |
| solaris_boot | 1031.00 | 122.057 | 118.39 | 1030.99 | 235.87 | 719.81 | 546.29 | 547.87 | 0.00 |
| login | 26.65 | 3.381 | 126.85 | 26.65 | 7.16 | 18.50 | 15.26 | 15.28 | 0.00 |
| useful_work | 516.25 | 64.231 | 124.42 | 516.24 | 142.62 | 362.17 | 296.17 | 297.51 | 0.00 |
| idle_100m | 100.00 | 10.977 | 109.77 | 100.00 | 20.07 | 65.26 | 54.81 | 54.82 | 0.00 |
| halt | 154.60 | 17.610 | 113.91 | 154.60 | 32.26 | 105.53 | 81.23 | 81.27 | 0.00 |

### SS-20/SX, 1 CPU, CPU0

| Phase | Δ outcomes (M) | Wall s | ns/out | I-req M | D-req M | Fast M | Walks M | Repl M | Diag M |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| prompt | 141.30 | 11.949 | 84.57 | 120.45 | 23.73 | 143.99 | 0.00 | 0.00 | 0.01 |
| solaris_boot | 1104.10 | 90.523 | 81.99 | 1104.08 | 254.27 | 1329.13 | 2.92 | 2.92 | 0.75 |
| login | 10.55 | 0.891 | 84.47 | 10.55 | 2.34 | 12.41 | 0.05 | 0.05 | 0.01 |
| useful_work | 397.55 | 31.852 | 80.12 | 397.54 | 110.28 | 502.19 | 0.53 | 0.53 | 0.33 |
| idle_100m | 100.00 | 7.216 | 72.16 | 100.00 | 21.44 | 121.41 | 0.00 | 0.00 | 0.00 |
| halt | 153.20 | 11.851 | 77.35 | 153.20 | 34.09 | 184.83 | 0.17 | 0.17 | 0.14 |

### SS-20/SX, 2 CPUs, CPU0

| Phase | Δ scheduler outcomes (M) | Wall s | ns/out | I-req M | D-req M | Fast M | Walks M | Repl M | Diag M |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| prompt | 145.40 | 12.534 | 86.20 | 120.73 | 23.92 | 144.46 | 0.00 | 0.00 | 0.01 |
| solaris_boot | 1701.60 | 144.930 | 85.17 | 970.53 | 206.98 | 1161.52 | 1.62 | 1.62 | 0.55 |
| login | 19.15 | 1.719 | 89.75 | 9.57 | 1.91 | 11.28 | 0.02 | 0.02 | 0.01 |
| useful_work | 750.75 | 62.285 | 82.96 | 375.37 | 86.74 | 458.85 | 0.27 | 0.28 | 0.28 |
| idle_100m | 100.00 | 7.516 | 75.16 | 50.00 | 9.12 | 59.12 | 0.00 | 0.00 | 0.00 |
| halt | 309.50 | 25.177 | 81.35 | 154.75 | 30.05 | 183.01 | 0.11 | 0.11 | 0.14 |

### SS-20/SX, 2 CPUs, CPU2

| Phase | Δ scheduler outcomes (M) | Wall s | ns/out | I-req M | D-req M | Fast M | Walks M | Repl M | Diag M |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| prompt | 145.40 | 12.534 | 86.20 | 1.82 | 0.45 | 2.28 | 0.00 | 0.00 | 0.00 |
| solaris_boot | 1701.60 | 144.930 | 85.17 | 731.03 | 149.58 | 866.66 | 1.40 | 1.40 | 0.54 |
| login | 19.15 | 1.719 | 89.75 | 9.57 | 2.00 | 11.29 | 0.03 | 0.03 | 0.01 |
| useful_work | 750.75 | 62.285 | 82.96 | 375.36 | 87.37 | 460.25 | 0.21 | 0.21 | 0.19 |
| idle_100m | 100.00 | 7.516 | 75.16 | 50.00 | 9.12 | 59.11 | 0.00 | 0.00 | 0.00 |
| halt | 309.50 | 25.177 | 81.35 | 154.74 | 30.88 | 184.75 | 0.05 | 0.05 | 0.06 |

Final CPU totals: SS-5 CPU0 has 1.94B instruction and 456M data requests,
1.355B fast hits, 1.038B walks and 1.041B replacements. SS-20/1 CPU0 has
1.886B/446M, 2.294B fast hits, 3.672M walks and 3.680M replacements.
SS-20/2 splits 1.681B/359M CPU0 and 1.323B/279M CPU2, with 2.026M/1.690M walks
and 0.997M/0.806M diagnostic updates. The SS-20 cost is fast-path/diagnostic
traffic, not page-walk thrash.

## Sampled profile before ranking

The retained user-mode sampler (`local/performance-2026-11-13/sample.py`) was run
against the corrected SS-5 boot: `PERF_COUNT_SW_CPU_CLOCK` at 1 kHz,
**229,385 samples, 0 lost**, 232.76 s sampled wall time. Phase sample counts:
firmware 6,854; prompt 4,598; Solaris boot 121,813; login 3,316; useful work
63,967; idle 11,159; halt 17,656. This is leaf-self attribution, not a claim
that categories add to phase wall time.

| Leaf category | Share |
| --- | ---: |
| MMU (`Srmmu` translate/load/finish/cache) | 48.9% |
| DRAM (`Ss5Dram` reads/writes) | 16.0% |
| device/IRQ servicing | 12.6% |
| machine/bus dispatch and scheduler | 11.6% |
| CPU interpreter | 10.4% |
| harness | 0.5% |
| other | 0.1% |

Largest self symbols: `Srmmu::translate` 28.4%, `Ss5Dram::read32` 15.5%,
`Cpu::step_with_interrupts` 10.0%, the `Srmmu::load` closure 8.1%,
`SystemBus::read32` 6.6%, `Sun4mDevices::sync_interrupts` 6.0%,
`Srmmu::finish` 4.3% and `Machine::step` 4.4%. The phase split is similar;
idle remains MMU-heavy (51.4% MMU). Raw samples and the symbol report are in
`profile/`.

## Ranked bottlenecks

1. **SS-5 MMU page-walk/replacement thrash.** Profile share **48.9%**, with
   `translate` alone at 28.4%. SS-5 executes 1.038B walks and 1.041B
   replacements; idle costs 109.77 ns/outcome versus the 42.7 ns bare floor.
   This is the first target and must be explained as a hardware-model/TRCR
   issue, not hidden by a larger host cache.
2. **Memory/bus and interpreter dispatch.** DRAM is 16.0%, machine/bus 11.6% and
   CPU 10.4% of leaf samples. The fixed ALU floor is 42.7 ns/step and the memory
   loop is 54.9 ns/step; even perfect MMU/device work leaves this path.
3. **Device and interrupt servicing.** Device/IRQ self time is 12.6%; SCSI/ESP
   service is 2.1% and interrupt synchronization 6.0% of the SS-5 profile. The
   full-device differential adds 46.1 ns/step to the same ALU loop. Treat that
   as an end-to-end upper bound, not a single device's cost.
4. **SS-20 MMU fast-path/diagnostic bookkeeping.** SS-20 has 2.0–3.7M walks but
   2.0–2.3B fast hits and 0.8–1.25M diagnostic fill updates per CPU. Its idle is
   72–75 ns/outcome, so walk elimination is not the SS-20 lever.
5. **Frontend idle/presentation.** The corrected frontend trace puts the
   Heliacal worker at **99.4% of one CPU** at the OBP `ok` prompt, while the main
   UI thread is only 0.33%; paused, the worker falls to 0.25%. Presentation is
   cheap, but the machine worker still runs the guest idle loop at full speed.
   Idle parking and JIT remain separate acceptance goals.

## Frontend evidence

The old November 14 snapshot is retracted: it recorded only 0.13 s of worker
CPU, exited with status 1, and its screenshots showed Solaris services rather
than the claimed prompts. It is not used below and its stale files were removed.

The measurement was rerun inside `my-distrobox` with the current Heliacal
release. The generated SS-20 NVRAM was seeded from the retained
`local/ss20-ttya.nvram` so `auto-boot?` was false; Heliacal selected the screen
console. `frontend/measure.py` wrote phase markers and captured screenshots
while the guest was live. The process exited **status 0**. The sampler recorded
**28,367 samples, 0 lost**, 52.72 s elapsed and 1,002 timeline snapshots.

Retained phase timing (`frontend/frontend-cost.txt`), calculated from the
markers and timeline:

| Phase | Wall | Main UI | Machine worker | llvmpipe |
| --- | ---: | ---: | ---: | ---: |
| paused empty | 12.00 s | 0.17% | 0.25% | 0.00% |
| OBP boot | 15.03 s | 2.13% | 99.01% | 4.33% |
| OBP idle at `ok` | 12.04 s | 0.33% | 99.39% | 0.00% |
| pause transition | 1.03 s | 1.95% | 4.86% | 5.84% |
| paused machine | 12.05 s | 0.17% | 0.25% | 0.00% |

`frontend/obp-idle.png` was captured at the `prom_idle` marker and shows the
SPARCstation 20 OBP banner, `Type help for more information`, and the `ok`
prompt; `frontend/paused-machine.png` shows the same prompt after pause. The
prompt is reached without a disk or Solaris boot. `frontend/phases.log` contains
the exact marker timestamps; `frontend/trace.samples.json` contains the raw
samples/maps/timeline; `frontend/timing-snapshots.tsv` is the phase-labelled
timeline. The core boot serial logs separately show Solaris login, commands,
idle and clean halt.

The corrected idle result is **99.4% worker CPU**, not 11.53%: the current
headless worker continues executing the guest OBP loop at full speed. This makes
idle parking a confirmed, separate requirement and does not change the core
boot baseline.

## IRIS review

Reviewed local IRIS `xiri` at `db49ed4beb9b533a3669d6fcb5f12480e52dc16f`.

- **Decoded-instruction cache** (`src/cpu/decode_cache.rs`): 64 physical pages,
  direct-mapped by multiplicative PFN hash, 1,024 decoded slots/page, raw-word
  validation (including fused next word). It is host memoization, not
  architectural cache state. IRIS documents a 64-page collision boot variance
  and a 256-page improvement. The idea suits SPARC, but Helios must implement it
  with safe indices/owned storage rather than IRIS's raw host pointers.
- **Idle parking** (`src/cpu/idle_park.rs`): samples PCs in a ring, recognizes a
  dominant kernel PC cluster with enough distinct PCs, rejects user-dominated
  and short device-poll loops, waits through a boot warmup, backs off on masked
  interrupt wakes, and wakes parked CPUs on interrupts/peer run-queue work. This
  is the right model for a separate idle-CPU acceptance slice; guest idle
  detection must not be a hard-coded Solaris address.
- **JIT v2** (`src/cpu/jitv2/`): Cranelift physical-page PIC region compiler,
  one function per entry, compile queue and page gateway, page snapshots plus
  per-page write generations for invalidation, lockstep interpreter verification.
  It reports 9.7–11x integer gains but no useful FP-only gain in IRIS's first
  matrix; memory-bound kernels can regress. Its raw pointers, Cranelift dependency
  and delay-slot self-modifying-code window are not directly portable to Helios.
- The useful ideas are the cache/hot-page invalidation model and the idle
  detector, not copying the MIPS JIT.

## Compatibility and proposed slices

Helios declares `rust-version = "1.74"`, edition 2021 and
`unsafe_code = "forbid"`. A Cranelift JIT must be audited against that MSRV;
IRIS pins Cranelift 0.134 and necessarily uses unsafe FFI/executable-memory
management. Any JIT must therefore be an optional separate crate/feature with
its own audited unsafe policy, not core code. The safe interpreter/cache path
can proceed first with no unsafe.

Suggested order (each with independent acceptance):

1. **Comparison harness and counters — baseline completed.** `perf fixed` and
   `perf compare` share the ALU/memory workload; `compare` checks PC/nPC, PSR,
   the active register view, RAM checksum, steps, traps, interrupts and resets
   between `step()` and `step_scheduled()`. Host CPU accounting requires a
   successful positive `CLK_TCK` and prints `unavailable` on failure. This is a
   smoke check only; it omits inactive windows, MMU/FPU state, device state and
   timing.
2. **Comprehensive differential validation — implemented in Slice B**
   (`performance-issue5-slice-b.md`). Before any cache, compare full CPU
   windows/MMU/FPU state and scheduler/device/timing state through
   `Machine::diagnostic_snapshot()`. Include traps, annulled delay slots, page
   remaps and DMA; require deterministic replay and independent expected state.
   `perf compare` alone is not sufficient.
3. **Safe decoded instruction/block cache.** Key by physical page; validate the
   raw instruction word (and delay-slot word) on each hit; bounded, owned tables.
   Test code invalidation, page remap, traps, ASI/boot-mode, and exact guest
   counters. No JIT.
4. **MMU first.** Resolve the SS-5 TRCR/WP=0 replacement behavior with primary
   evidence and independent fixtures before host-cache tuning. Preserve SS-5
   integrated I/O semantics and SS-20 standalone MMU ownership.
5. **Device/interrupt readiness.** Remove empty SCSI/ESP and interrupt
   recomputation work with exact state changes; do not enlarge polling intervals
   as a substitute for readiness.
6. **Idle parking, separately.** Implement PC-cluster idle detection and wake
   rules without changing guest timing; acceptance is host CPU while paused/idle,
   not MIPS.
7. **Optional JIT prototype.** Only after 1–5, and only in a separate crate/feature.
   Start with integer/ALU blocks and fallback all memory/MMU/trap/window/ASI
   cases:
   - compile branch + delay slot as one unit; preserve annul and PC/nPC;
   - never compile `save`/`restore`, window overflow/underflow, traps, MMU
     control, ASIs or other architecturally observable system instructions;
   - validate every block against the interpreter with independent expected state;
   - invalidate on every RAM write/DMA/code page change, with a generation check
     at entry and backedge; no stale window hidden by a host cache;
   - one CPU-local execution state for SMP; JIT compile must not run in the hot
     CPU loop; device time/IRQ scheduling remains the scheduler's responsibility;
   - fallback to the interpreter on any unsupported or faulting operation.

No decoded cache or JIT was implemented in this slice.

## Evidence and cleanup

Raw logs are under `local/issue5-perf-2026-11-15/`:

- `runs/` corrected phase logs, serial logs, `phase-counters.csv` and
  `final-counters.txt`; `runs-old/` retains the pre-correction logs.
- `profile/` SS-5 sampler raw JSON, symbol report, category summary and script.
- `frontend/` corrected phase summary, raw sampler trace, phase markers, timeline
  TSV, and live OBP/paused screenshots; stale November 14 frontend files removed.
- `fixed/` fixed-work and comparison logs.

Source SHA-256 values checked before/after:

```text
7324f3a9741e963f75585d126315033bab79292159857104d32390b852577ffc  ss5-final.raw
5686d92de881321d825e100139f9222af469c34eec67f67f76e2ec1a0d79169b  ss20-trace.raw
e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e  sun-ss5.bin
b4f15bfafa69edfcea446778bd14b61b76273e4e77ddebe3a9002af757b48ca8  ss10-20-v2.25r-rom
3fb9a8dbe7c8621dd9a65246ac78d9353a492c41862c1dd61656fb03d1a16259  turbogx.bin
```

Disposable disk clones and the local benchmark target were removed. The
frontend NVRAM copy is the retained `local/ss20-ttya.nvram` initialization with
screen console selected by Heliacal. Source media and the user's live catalog
were not modified.

## Validation

At Slice A finish, current main had **683 unique normal tests (684 successful
lines), 685 unique with `perf-counters` (686 successful lines)**; `cargo fmt
--all --check`; both Clippy configurations; and `demo`, `trap-demo`,
`serial-demo`, `timer-demo` passed. The comparison harness reports `state=match`
for ALU and memory. After the MMU walk investigation the counts are 719/722. This slice is a
handoff, not a JIT acceptance or an idle-power improvement.
