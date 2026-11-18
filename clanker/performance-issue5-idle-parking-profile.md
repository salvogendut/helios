# Issue #5: idle-parking cost profile (corrected)

Status: feature-gated profiling diagnostics checkpoint `9b21916`. The accepted
default-off frontend behavior remains `a88d769`/`2682872` on core `e3d87e3`.
Issue #5 remains open; no merge or push was made. Deeper token profiling has not
started.

## Method

Opt-in worker diagnostics require the default-off `parking-profile` Cargo
feature and are enabled by environment variables (`HELIACAL_PARKING_PROFILE`,
reset/dump/toggle markers). The ordinary build compiles the module and all
associated branches out; see the
[isolation slice](performance-issue5-idle-parking-profile-isolation.md). All elapsed values below are
worker `Instant` intervals since the last reset, not CPU time.

Three phases were measured separately in each run:

- **Idle:** a 5-second window before any command is issued.
- **Command:** 30 PTY `echo` trials, with their own reset and profile dump.
- **Fixed work:** an `awk` loop of 100,000 iterations, with instrumentation
  toggled active/inactive inside the same process. Three runs (8 repeats each
  in the last) used `perf stat` user-space instructions/cycles for corroboration.

Each run used a disposable disk reflink, seed NVRAM, separate config/output
directory, and ended with guest `sync` and `halt`. The phase runs used binary
`c243b5c0…` (before the active/inactive toggle); the fixed-work runs used
`4b37d5fe…`. The toggle is profiling-only; the accepted default path is
unchanged. Temporary disks, FIFOs and PTY locks were removed. Evidence:
`local/issue5-parking-profile-corrected-2026-11-17/` (`*/windows.json`,
`*/profile-idle-0.json`, `*/profile-command-0.json`, `*/fixed.json`).

## Accepted repeated CPU/latency result

The previously accepted matched clean pair remains the CPU reduction evidence:
worker CPU **99.560% -> 25.687% (74.2% lower)**, PTY median **22.305 -> 22.145
ms**, p95 **29.768 -> 38.854 ms**, max **36.421 -> 50.739 ms**. The corrected
idle-only samples were 28.4% and 17.2% with parking on versus 99.4% and 99.6%
off; they are consistent but are not used to replace the accepted matched pair.

The latency cause is **not resolved**. The harness records host-visible input
write, first echoed token, second token and prompt timestamps only. It has no
timestamp for worker input observation or output publication, so it cannot
separate parking wait, worker scheduling, terminal buffering and host
scheduling. Do not use the previous echo-tail explanation.

## Instrumentation overhead, fixed work

Fixed 100,000-iteration guest work, toggling diagnostics in the same process:

| State | Worker CPU median | PMU instructions median | PMU cycles median |
| --- | ---: | ---: | ---: |
| Diagnostics inactive | 5.235 s | 97.865 B | 25.744 B |
| Diagnostics active | 5.275 s | 97.931 B | 25.867 B |
| Difference | +0.76% | +0.067% | +0.48% |

These medians are over 8 inactive and 8 active repeats across three runs. The
first active repeat in every run was roughly 15–20% slower; background guest
work is a possible explanation, but was not independently measured. Medians
include those repeats. The earlier **+1.30 percentage-point CPU overhead claim from
utilization is withdrawn**: fixed work does not establish overhead that large.
This check covers normal-execution instrumentation only; candidate/token timer
overhead remains unmeasured at this resolution.

It also does not compare the profiling candidate with diagnostics absent
against the accepted frontend: inactive still retains the profiling object
and dispatch. The [independent correction review](performance-issue5-idle-parking-profile-review.md)
found a repeatable approximately 4.2% slowdown in a fixed synthetic workload
through the actual worker. The subsequently accepted
[compile-time isolation](performance-issue5-idle-parking-profile-isolation.md)
resolves that ordinary-build regression. The historical timings on this page
remain scoped to their original binaries and runtime toggle comparison.

## Idle phase, parking on

Two independent idle windows, each reset before its 5-second sample:

| Worker section | Run 1 | Run 2 |
| --- | ---: | ---: |
| Elapsed since reset | 5.054 s | 5.063 s |
| Host wait | 3.613 s | 4.174 s |
| Normal execution | 0.728 s | 0.090 s |
| Token construction | 0.325 s | 0.361 s |
| Commit validation | 0.359 s | 0.412 s |
| Host checks | 0.011 s | 0.011 s |
| Host I/O | 0.010 s | 0.006 s |
| Candidates / committed | 1,812 / 1,767 | 2,104 / 2,047 |
| Token+validation share of active | 47.4% | 87.0% |
| Normal-execution share of active | 50.5% | 10.1% |

Across these first, pre-command idle samples, token construction plus parked
commit/advance is **62.54% of elapsed time outside waits**; normal execution is
35.1%; host checks/I/O are about 2%. The legacy `commit_validation` metric
includes the entire `advance_parked` call, not only token validation. These
elapsed intervals do not establish CPU-time attribution.

The retained later, post-command idle windows are different: worker CPU is
53.56%/51.59%, with normal execution accounting for 80.58% and the token/commit
pair for 17.84% of combined elapsed time outside waits. Their cause is not yet
identified; retain and report both regimes when choosing the next optimization.

## Command phase, parking on

Thirty command trials, reset separately after the idle dump:

| Worker section | Run 1 | Run 2 |
| --- | ---: | ---: |
| Elapsed since reset | 1.006 s | 1.007 s |
| Host wait | 0.426 s | 0.453 s |
| Normal execution | 0.466 s | 0.441 s |
| Token construction | 0.041 s | 0.044 s |
| Commit validation | 0.048 s | 0.047 s |
| Host checks | 0.013 s | 0.012 s |
| Host I/O | 0.009 s | 0.006 s |
| Token+validation share of active | 15.3% | 16.4% |
| Normal-execution share of active | 80.3% | 79.6% |

Command processing is therefore a different workload from idle. The idle
breakdown must not be combined with command elapsed time, and the command
numbers do not locate the latency tail.

## Scope and next slice

The current conclusion is limited to SS-20, one CPU, 128 MiB, Solaris 2.6,
headless PTY and networking disabled. No X, SMP, physical-keyboard or
sustained-load claim is made. Fixed work did not isolate candidate/token timer
costs; worker timestamps are still required before any latency optimization.
The next slice is deeper profiling only, preserving the 2 ms wait, proof checks,
default-off behavior and SMP exclusion.
