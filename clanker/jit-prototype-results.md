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
