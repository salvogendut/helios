# MMU performance slice review

2026-11-14. Review of the uncommitted counter and diagnostic-view cache changes
on top of Helios `6364a3a`.

The cache preserves the existing fill-only-zero diagnostic behavior in the
cases reviewed. No production correctness regression was found. The matched
wall-time improvement is supported by the raw logs.

## Resolution

The reporting and coverage follow-ups are complete. The reviewed fixtures are
tracked unchanged in `tests/diagnostic_cache.rs`. The implementation report
and SS-5 investigation now use fieldwise deltas for the second idle window;
the public API documents counter units, reset scope and equality behavior.
Normal validation passes with 514 tests, and `perf-counters` validation passes
with 516 tests. Formatting, Clippy in both configurations and all four demos
pass. The MMU slice is ready to commit with WP=0 replacement behavior unchanged.
The findings below retain the earlier review and reproduction details.

## Independent cache checks

The tracked fixtures were promoted from
`local/review-performance-2026-11-14/mmu-cache-check/tests/diagnostic_cache.rs`.
Seven behavior tests pass against both the committed pre-optimization core and
the proposed core with counters disabled. All eight tests pass with the new
`perf-counters` feature enabled; the eighth checks that a warm view avoids fill
work while still counting every translation.

The behavior checks cover:

- Warm accesses preserve both high-slot and low-offset diagnostic views.
- Zero context/tag values remain fillable; later nonzero values do not replace
  occupied diagnostic words, including virtual-region aliasing.
- Word writes refill explicitly cleared fields and retain nonzero edits.
- Doubleword writes invalidate both diagnostic address forms.
- Page, segment, region, context and full CPU flushes allow a cleared
  diagnostic word to refill after the view was cached.
- Instruction accesses do not fill data views; reset clears views and permits
  fresh fills.

These assertions preserve the existing functional diagnostic model. They do
not claim independent validation of the complete physical SuperSPARC MMU.
The previous core was exported from Git into the isolated
`local/review-performance-2026-11-14/mmu-cache-reference/` fixture.

The existing MMU and TLB diagnostic suites also pass with counters enabled:
27 tests. Logs for the proposed core and phase calculations are under
`mmu-cache-check/`; the reference run log is under `mmu-cache-reference/`.

## Counter reports mix a timed phase with cumulative totals

The local harness prints `translation_counters()` only at the end of
`measure()`. Those values accumulate since MMU construction/reset; they include
boot and preceding measurement phases. For example, SS-20's reported
`instruction_requests=1,394,234,033` cannot describe just the timed 204.8M-step
idle window. The 307,169,788/686,667 diagnostic-update counts and 2,181,592
walks are cumulative snapshots at that point.

The same issue affects the SS-5 investigation: 768,363,746 walks is cumulative
through the end of the first idle phase, not the walk count for that phase.
Take snapshots immediately before and after each timed window and report the
fieldwise delta. Keep cumulative boot totals explicitly labeled separately.

The existing logs already allow exact deltas for the *second* idle window,
`serial_idle_scanout`, by subtracting the two successive snapshots:

| Counter, 204.8M steps | SS-20 cache disabled | SS-20 cache enabled | SS-5 |
| --- | ---: | ---: | ---: |
| Instruction requests | 204,797,882 | 204,797,882 | 204,797,643 |
| Data requests | 43,977,446 | 43,977,446 | 45,337,026 |
| Fast hits | 248,665,530 | 248,665,530 | 137,931,212 |
| Architectural hits | 101,452 | 101,452 | 227,304 |
| Walks | 8,346 | 8,346 | 111,976,153 |
| Diagnostic fill-path executions | 43,977,446 | 1,101 | 0 |

All three SS-20 before/after pairs have these same per-window counts. The
derived data is saved as `mmu-cache-check/phase-deltas.json`, sourced from
`mmu-counters/diag-{base,on}-{1,2,3}.err` and `ss5-counters.err`.

This reporting error does not invalidate the measured wall times. The first
idle phase's medians are 12.464543 seconds before and 10.681054 after. The
boot-to-settle phase medians are 68.311289 and 58.033971 seconds. The second
idle phase above has medians of 12.483777 and 10.707293 seconds, respectively.

## Document counter units before using them for the SS-5 investigation

The public feature/API currently has only a general description. Document that:

- Instruction/data requests count translated CPU accesses, excluding boot
  PROM mapping, physical bypass and disabled-MMU accesses. These are attempted
  translations, not retired instructions.
- Fast hits, full TLB hits and walks are combined across instruction and data
  accesses. This API cannot by itself establish a data-only walk percentage.
  Preserve the earlier separately instrumented evidence as a separate result;
  add per-access-class outcome counters for the next SS-5 experiment if needed.
- `replacements` counts calls to the shared insertion path, including empty
  slot fills and I/O fills. It is not a count only of valid CPU-entry evictions.
- `diagnostic_updates` counts executions of the diagnostic fill path, not the
  number of HashMap writes or changed diagnostic words.
- Snapshots are cumulative and reset with the MMU; subtraction across reset
  is not a valid interval measurement. Counter state is excluded from MMU
  equality, while modeled state and the new diagnostic cache are compared.

Keep the SS-5 TRCR/WP=0 semantics unchanged in this slice. After promoting the
regressions and fixing the report scope/documentation, this bounded change is
ready for its final checks and review. Further hardware-behavior changes need
their own evidence and measurement.
