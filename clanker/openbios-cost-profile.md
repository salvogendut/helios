# OpenBIOS startup cost profile — 2026-11-16

Follow-up: [startup optimization](openbios-startup-optimization.md) implements
and validates the native dictionary search motivated by this baseline.

Question: why does the replacement firmware take tens of seconds on serial
and minutes on CGsix? This follow-up changes no production code or installed
firmware. Both runs use the current installed port (SHA-256
`084fa665e089edb0a00532f15bcc5fe383401f23f78ef1fd88dc8221bf161616`),
SS-5, 256 MiB, fast clocks, one CPU and the Solaris 2.6 source disk opened
read-only at SCSI 3. The headless core profiler has no SDL, PTY or networking.
Runs were sequential, release optimized, with `perf-counters` enabled.

## Measurements

| Stop condition | Guest outcomes | Host wall | Page-table walks |
| --- | ---: | ---: | ---: |
| Serial `0 >` prompt | 546,150,000 | 43.605 s | 1,078 |
| CGsix first banner text | 2,014,000,000 | 157.649 s | 1,592 |

The graphical stop condition requires three successive nonuniform decoded
frames after the firmware's UUID line. `graphics.png` shows the banner partway
through its first line, not the finished prompt. The earlier UI acceptance
shows the complete prompt by the 180 s capture. These are instrumented local
runs, not a multi-run performance claim.

The serial run completes 546,149,740 instructions and 260 synchronous traps;
the graphical run completes 2,013,999,719 instructions and 281 synchronous
traps. Both have zero interrupt entries. Throughput is about 12.5–12.8 million
outcomes per host second. The revised RAM clear is 62,914,560 instructions,
and is only 3.1% of the graphical instruction count.

Stages in the serial run: first serial configuration line 91.83M / 10.15 s;
first SBus probe 313.22M / 26.48 s; CPU line 511.05M / 41.02 s; UUID line
527.85M / 42.25 s; prompt 546.15M / 43.61 s. In graphics, the card's slot probe
adds about 172M instructions, but most extra work comes after the UUID line:
699.97M / 55.80 s to first banner at 2.014B / 157.65 s.

## Instruction sampling

Pseudorandom sampling intervals between 8,191 and 12,286 outcomes avoid
locking onto a single periodic loop instruction. Counts are guest-instruction
samples, not host wall-time percentages. Symbols came from a fresh build of
the exact same pinned source/patch. Its loaded segment differs from the tested
image in only three bytes, all in the printable build timestamp; function
addresses and executable code match.

| Guest function | Serial samples | Graphics samples |
| --- | ---: | ---: |
| `next` (Forth dispatch) | 25.40% | 30.55% |
| `enterforth` (interpreter loop) | 16.91% | 20.20% |
| `PUSH` + `POP` | 16.24% | 19.20% |
| `PUSHR` + `POPR` | 6.51% | 7.48% |
| RAM-clear loop | 11.51% | 3.12% |
| `udelay` | 5.07% | 1.38% |

Together, the dispatcher and stack helpers consume about 65% of serial and
77% of graphical sampled instructions. Additional Forth primitives account
for much of the rest. This is a portable C Forth interpreter executing inside
Helios's SPARC instruction interpreter. Each dispatch, stack operation and
primitive expands into guest instruction fetch/decode, CPU state, bus/MMU,
interrupt and device service work.

OpenBIOS uses `-Os`; helpers declared `static inline` still appear as separate
functions in this build. Interpreter/internal/stack debug tracing is disabled.
Heliacal and the core use release optimization and thin LTO. The worker does
not sleep while running. The MMU fast cache handles virtually all requests
(616,443,466 serial / 2,503,626,626 graphical hits); the old SS-5 WP=0 Solaris
page-walk hotspot does not explain this boot path. MMU hit processing still
has an ordinary per-instruction cost.

The evidence points to reducing OpenBIOS interpreter/stack dispatch overhead
and its console-initialization work first. Benchmark speed-oriented compiler
options/inlining against identical firmware and OS acceptance checks; do not
assume a particular speedup. Then profile Helios's native execution cost per
instruction. There is no evidence here that shorter device delays, removal of
RAM clearing, or further frontend refresh changes would solve the main cost.
Exact Forth-level words inside the post-UUID graphics phase remain unprofiled;
the C-function sampling identifies the dispatch overhead, not those callers.

Evidence and reproducible probe: `local/openbios-cost-profile/` contains
`probe.rs`, `report.py`, phase logs, sampled PCs, symbol/build provenance and
the framebuffer capture. Scratch binaries and private source/build trees were
removed; no source media, live settings, user NVRAM or installed image changed.
