# Workstation JIT span integration review

2026-11-19. Helios `787b5ca`, Heliacal `60ecaad`.

The bounded span implementation passes the focused architectural checks below.
The original-OBP Solaris logs support the reported functional boot/probe/halt.
This is not an acceleration acceptance: the warmed path violates the requested
allocation-free-hit contract, and the required phase counters and frontend
budget/lifecycle regressions are still missing.

## Findings

### 1. Four allocations per warmed span or common fallback dispatch

`src/machine.rs:2364` allocates `span_times` on each call. The CPU allocates
`words` and `proofs` at `src/cpu.rs:751`, then an outcome vector at line 782
or 799. These allocations happen even when compiled code is already cached.
Heliacal discards the outcome records immediately and retains only their
count at `src/emulator.rs:573`.

A separate diagnostic allocator executable, after 32 warm-up calls, measured
2,000 calls per case. It delegates unchanged to `std::alloc::System`; its
unsafe allocator wrapper exists only in this local diagnostic crate. Core
`unsafe_code = "forbid"` remains unchanged.

| Path | Outcomes | Allocations | Allocations/call | Allocated bytes |
| --- | ---: | ---: | ---: | ---: |
| Ordinary interpreter | 2,000 | 0 | 0 | 0 |
| Ordinary one-word bridge | 2,000 | 0 | 0 | 0 |
| Warm 16-word spans | 32,000 | 8,000 | 4 | 2,928,000 |
| Native span API, budget one | 2,000 | 2,000 | 1 | 128,000 |
| Branch fallback | 2,000 | 8,000 | 4 | 1,008,000 |
| Held SCSI-DMA-reset fallback | 2,000 | 8,000 | 4 | 1,008,000 |

The probe rewinds PC and flushes deferred time between calls to keep the
case stable. It is an allocation measurement, not a Solaris timing model or
an attribution of the entire 4x guest slowdown. Results and exact source are
under `local/review-workstation-jit-spans-2026-11-19/`.

There is also avoidable rejected-span work: the code gathers up to sixteen
proofs before checking device/time readiness in its descending-length loop.
For example, held DMA reset can reject every candidate length after the
lookahead has already happened. Check cheap exclusion and whether **two**
instructions can fit before proving a longer suffix; retain the first live
fetch and precise fallback effects.

### 2. Shutdown counters show short spans, not useful-work coverage

Both retained `inputs.json` files have the same binary hash (`e156adab…`) and
ROM/NVRAM/source-disk hashes. Both fixed-work records print the expected 4950;
both frontends exit 0 after sync/halt. The recorded command times are
0.154207080 s interpreter and 0.617055547 s JIT. This is one sample per arm,
not the requested three alternating pairs.

The JIT totals are printed at Quit, not at useful-work phase boundaries:

- Native instructions: 579,432,680.
- Native calls: 545,483,922.
- Calls with length >=2: 22,115,529.
- Consequently, single-instruction calls: 523,368,393, **95.95% of calls**.
- Mean instructions/call: **1.062**; mean length among multi-instruction spans:
  **2.535**. Only **9.68% of native instructions** belong to those spans.

This arithmetic follows from the current counter increments, but it applies
to the cumulative run. It does not prove the useful-work distribution, a
specific maximum span length used in that phase, or the cause of its slowdown.
Do not label these as post-boot/useful-work deltas.

The retained `measure_jit.py` still hardcodes `--execution jit`. The interpreter
log reports zero native work, so this review does not reject its mode label;
make the next harness select its mode explicitly and retain actual argv and
the exact script used for both arms.

### 3. Required regression/evidence coverage is incomplete

The three new tracked span tests are insufficient by themselves:

- Their `mapped` helper enables native execution for the reference machine,
  so the oracle uses the one-word bridge rather than the interpreter.
- `unsupported_suffix_shortens_the_span` puts the unsupported word at index
  16 while requesting only 16 outcomes. It never examines that word and
  therefore does not exercise shortening at an unsupported suffix.
- They do not compare explicit MMU counters (excluded from snapshots), exact
  returned PC/nPC/word sequences, warmed shorter budgets, budget 17 or
  shortened event-time boundaries.
- Heliacal's commit adds no tests for its new worker budget/accounting path;
  the unchanged 121-test count is not new integration coverage.

The six independent fixtures supplied in `native_span_review.rs` all pass.
They use a native-disabled oracle and cover exact outcomes/counters across
budgets 1/2/3/15/16/17 on SS-5/SS-20 and both clocks, warmed shorter budgets,
an unsupported word **inside** the candidate span, deferred-time shortening,
a page-edge fault, and late-word code modification. Together with the
fourteen pair fixtures and three submitted span tests, **23 tests pass**.
No architectural mismatch was found in these checks. The full suites and
frontend were not rerun for this review.

## Next bounded implementation pass

1. **Remove dispatch allocations and wasted lookahead.** Use fixed-capacity
   stack/owned reusable storage for at most sixteen times/words/proofs and an
   allocation-free outcome representation. Preserve exact outcomes for tests
   and traces without forcing a heap vector that Heliacal immediately drops.
   Warm successful spans, budget-one and ordinary fallback calls should all
   allocate zero times. Check cheap eligibility after the first live fetch,
   cap the possible length by time/budget first, and avoid analyzing a suffix
   when the already-fetched first opcode cannot join a native span.
2. **Measure interpreted single-instruction fallback.** Compare a variant
   that executes isolated ALUs through the already-fetched interpreter helper
   instead of entering native code once per word. Keep the bridge available
   as a reference. Do not implement this by toggling global native enablement
   or clearing its caches per step, and do not fetch twice. Check architectural
   equivalence and report actual changed counters. Select a policy from
   bounded throughput/useful-work measurements, not a guessed minimum span
   length or a higher native-instruction count.
3. **Supply the missing evidence in the same pass.** Promote the six fixtures,
   correct the old suffix/oracle tests, and add real worker budget tests:
   exactly 2,048 outcomes, final short budget, progress before a fault/reset,
   power-off, keyboard/BREAK, parking exclusion and interpreted SMP. Expose
   reset-aware phase deltas, block-length histogram, compilation/cache-miss
   counts and fallback counts. The span cache currently clears all entries
   at 256; use the counters to detect recompilation rather than assuming this
   policy is harmless or increasing capacity blindly.

Run warmed synthetic/control measurements before another expensive guest boot.
Keep the accepted fetch/MMU/device contract, default-off selection, SMP scope
and parking policy. Then run the matched three-pair original-OBP Solaris
useful-work measurement, with parking off and reproducible mode selection.
Retain the ordinary-mode performance control and required validation.

Branch/delay-slot and safe memory coverage remain checkpoint-3 work. First
fix this measurable dispatch overhead and obtain useful-work coverage data;
then choose those execution paths from the observed stopping points. No merge
or acceleration claim until the original correctness and 10% gates pass.

Production source was not changed by this review. Retain the small fixture,
allocator, runner and logs; remove the review's temporary source/build trees.
