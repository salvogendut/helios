# Workstation JIT dispatch after word LD/ST

On 19 November 2026 the remote `feat/workstation-jit` heads were Helios `5281937` and Heliacal `b33eb55`. Applying the retained word-RAM patch to `9b0b3ea` reproduced all seven source and test files byte for byte. All eight word-access tests passed again with `jit-profile,perf-counters`. The three prior Solaris pairs therefore cover this executable source. Their median command times were 156.9 ms interpreted and 358.2 ms with JIT. The 10% target was 141.2 ms. Keep both feature branches unmerged. [Solaris measurements](workstation-jit-ram-gate.md)

## Short versus long spans

The existing SS-20 probe used one release binary, interpreted single-word fallback, a warm translated page and a full board. Each table entry is the median of three trials, with reversed mode order in pair two. Each trial ran 24,000 warm-up outcomes and 2.4 million measured outcomes. The probe checked the expected registers, PC/nPC and virtual time. Native share counts guest instructions retired natively.

| Guest loop | Interpreter ns/outcome | JIT ns/outcome | Native share |
|---|---:|---:|---:|
| ADD, branch, delay NOP | 57.0 | 79.7 | 88.3% |
| Alternating ADD and word LD, branch, delay NOP | 68.9 | 113.1 | 60.0% |
| Fourteen ADDs, branch, delay NOP | 57.7 | 42.0 | 95.0% |
| Same long loop, SCSI DMA reset held | 53.5 | 94.6 | 0.0% |

Long eligible spans already beat the interpreter on this loop. Short spans spend more time per outcome even at 88% native coverage. With DMA reset held, dispatch costs about 41 ns extra per outcome despite making zero native calls. These fixtures localize a cost to the span path. Its share of the Solaris command time remains unmeasured.

## Bounded assignment for the implementation agent

1. Try one change at Helios `5281937`. In `Machine::step_native_span`, make speculative clock projection lazy. Currently each eligible call computes up to 16 guest clock increments before the live fetch, opcode check and bus readiness decision. Compute prospective lengths only after a candidate and a post-fetch time limit exist. Preserve fractional clock phase, strict deadlines, interpreter fallback and the final readiness check.
2. Compare the original and candidate release builds on the same four loops above. Run three alternating pairs with the same outcome count. Compare actual outcomes, native calls and architectural state, including hardware-clock phase, interrupts, traps, MMU counters and device deadlines. Keep the ordinary interpreter as the timing control. Existing translated-span, branch-slot and word-access fixtures cover relevant boundaries.
3. Stop after this experiment. Report whether rejected and short spans got cheaper and whether the long loop retained its advantage. Defer further Solaris boots until a candidate shows enough improvement on guest-relevant work to justify another acceptance run. The Solaris merge criterion remains 10%.

The runner, raw timings, binary and probe hashes, and test log are under `local/review-workstation-jit-ram-dispatch-2026-11-19/`. The runner uses a disposable build and leaves both repositories' production sources untouched.
