# Solaris workstation JIT acceptance, 19 November 2026

Helios base `9b0b3ea`, Heliacal `b33eb55`. The patch was tested in an isolated copy with the existing Heliacal frontend.

## Release measurements

| Pair | Interpreter boot (s) | JIT boot (s) | Interpreter command (ms) | JIT command (ms) |
|---|---:|---:|---:|---:|
| 1 | 77.54 | 163.20 | 156.89 | 358.19 |
| 2 | 76.27 | 164.51 | 148.66 | 358.65 |
| 3 | 77.55 | 159.47 | 158.85 | 344.05 |

The useful-work threshold was JIT median wall time ≤ 141.2 ms (10% below interpreter). The interpreter median was 156.9 ms and the JIT median was 358.2 ms, or 2.28 times interpreter time. The branch misses the threshold. Keep both feature branches unmerged.

All six arms ran the same uninstrumented native-feature release binary with original SS-20 OBP, 128 MiB, one CPU, fixed RTC, and fresh clones of the same Solaris 2.6 disk and NVRAM seed. Order was interpreter/JIT, JIT/interpreter, interpreter/JIT. Boot time covers `boot disk` to login. Command time covers host PTY write through the next root prompt for the fixed AWK sum of 0 through 99. Every arm verified `4950`, synced, halted and exited successfully. The runner checked source hashes and removed each disk clone. Each arm had a 330-second hard limit.

## Word access

An integer ALU or SETHI followed by an aligned ordinary word LD or ST can execute in one x86-64 call. The data address uses the post-prefix register values. Both instruction addresses must pass the existing fetch proof. The data mapping must be a warm, permitted 4 KiB page backed by installed DRAM, with the referenced bit already set and the modified bit set for a store. The second fetch completes before one ordinary data translation and bus request. The backend receives a borrowed backing word. Stores update the existing mutation and code-watch state. Cold, faulting and device accesses follow the existing step path.

The fixtures compare retired outcomes, full diagnostic snapshots and MMU counter deltas. They cover address dependencies, self-modification through a physical alias, a misaligned load, the first dirty-bit update, a protected store and an MXCC port access. A direct backend fixture checks both byte orders and an ICC-setting prefix that produces the store value.

## Validation

The isolated build passed 814 default and 941 combined JIT-profile/perf-counter core test executions, all eight word-access fixtures, three standalone backend tests and 126 Heliacal tests. Formatting, Clippy with warnings denied and all four demos passed. The timing comparison measures the whole JIT mode. It does not isolate the cost of the new memory pair.

## Retained files

Evidence is under `local/review-workstation-jit-ram-gate-2026-11-19/`. The `matched/summary.json` and `matched/rows.json` files contain phase timings. Each arm has its invocation, input hashes, transcript and exit events. `native-word-ram.patch` contains the isolated core changes and fixtures. To check the patch against the stated Helios base, run

```sh
git apply --check local/review-workstation-jit-ram-gate-2026-11-19/native-word-ram.patch
```
