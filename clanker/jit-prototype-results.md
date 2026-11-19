# Bounded JIT prototype result

Branch: `feat/issue-5-jit-prototype`, based on merged main `5fcffdc`.
Feature: `jit`, default off; no unsafe or executable host memory.

## What runs through the JIT

The safe block decoder supports straight-line SETHI and integer ALU/condition
code operations. It compiles at most 16 instructions, validates each word
through the current MMU/ASI, and uses the interpreter for branches, delay
slots, memory, windows, traps, ASIs, FPU, devices and unknown encodings.
Device-equipped machines deliberately use the interpreter path in this
milestone so PROM wait accounting remains exact. The cache is call-local and
conservatively invalidated by bus mutation and MMU generations; unsupported
words are negatively cached.

## Fixed-work measurement

Identical optimized release builds, `examples/perf fixed alu 5000000`,
25,000,000 executed instructions. Three alternating runs:

| Run | Interpreter ns/step | JIT ns/step |
| --- | ---: | ---: |
| 1 | 40.90 | 30.08 |
| 2 | 41.10 | 30.49 |
| 3 | 41.30 | 30.05 |

Median: **41.10 -> 30.08 ns/step**, about **1.37x** faster on this fixed ALU
workload. JIT compilation was measured separately at 1.1-2.4 microseconds for
two positive blocks and one negative branch block; execution checksums and
25M instruction counts match. The feature-off build remains the reference.

The fixed `memory` workload regresses (about 53.3 -> 136 ns/step): loads/stores
are correctly unsupported, and the per-store mutation epoch forces repeated
block compilation. That is the measured bottleneck, not evidence for a memory
JIT. Do not describe these synthetic results as faster Solaris boot.

## Validation

Seven JIT tests cover independently expected SETHI/ALU results, full diagnostic
snapshot equality, self-modifying code, mapped fetch/remap, removed fetch
permission, branch/fallback boundaries and device-machine interpreter fallback.
Default tests: 814; with `jit`: 823. Formatting, Clippy and all four built-in
demos pass. Evidence: `local/jit-issue5-2026-11-18/`.


## Native candidate

`jit-native` adds the isolated x86-64 emitter. Native admission is narrower
than the decoded experiment: device-free, single-CPU, MMU-disabled writable
RAM only. The review fixtures remain passing on the decoded/fallback path;
native fixtures cover straight-line results, split branch entry, reset/error
delivery and device fallback.

Fixed ALU, 25M outcomes, identical optimized builds:

| Run | Interpreter | Native |
| --- | ---: | ---: |
| 1 | 41.15 | 34.37 |
| 2 | 41.52 | 34.26 |
| 3 | 41.76 | 34.12 |

Median **41.52 -> 34.26 ns/outcome**, about **1.21x**, with native compilation
and register/ICC marshalling dominating the small three-instruction loop.
Decoded-block remains faster than native for this short loop; the native path
is the milestone candidate, not a claim of a large general speedup.

The native cache stops admission after 64 compile attempts. The fixed memory
workload therefore returns to the interpreter rather than compiling every
store: median 54.00 -> 55.57 ns/outcome, a small fallback cost rather than the
previous 2.56x regression. Memory operations are not JIT-compiled in this
slice.
