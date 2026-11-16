# Issue #5: SS-5 MMU page-walk investigation

Status: unsafe page-walk shortcut removed; per-slot host fast-cache generations
retained. The SS-5 WP=0 page-walk bottleneck remains open. TRCR/WP replacement
semantics are unchanged. No MMU model correction or JIT is included.

## What changed

The SS-5 profile retains its existing microSPARC-II replacement model. TRCR
WP=0 still limits architectural replacement to slot zero exactly as before.

The host fast caches now use **per-slot generations** rather than advancing a
single global generation for every architectural fill. A 4 KiB fill invalidates
only the direct-mapped fast entry for its own virtual page; a larger mapping
invalidates both host fast caches by advancing their shared epoch. This avoids
array scans and clears while preserving the narrow-over-broad lookup rule.
MMU flushes and SS-5 ASI 6 diagnostic edits invalidate globally; ordinary MMU
register writes preserve unrelated fast hits.

The attempted leaf-only page-walk shortcut was removed. It could not validate
context/L1/L2 ancestors after eviction, swallowed leaf faults, and contaminated
snapshot equality. `set_walk_cache_enabled` remains as a no-op fixture
compatibility hook; there is no host page-walk shortcut in the execution path.

## Coherence and correctness

- Architectural fills still go through `Srmmu::insert`; replacement counters,
  freeze/IL/PL behavior and WP=0 semantics are unchanged.
- A 4 KiB mapping affects only its one direct-map host slot. Larger mappings
  invalidate both host caches in O(1) through their shared epoch.
- MMU flushes and SS-5 ASI 6 diagnostic edits still invalidate the fast caches;
  ordinary MMU register writes preserve unrelated fast hits, as required by the
  overlap fixtures.
- The seven independent review fixtures are promoted as
  `tests/mmu_walk_review.rs`. They cover ancestor retargeting after eviction,
  diagnostic TLB invalidation, leaf/ancestor bus faults and host stops, snapshot
  cache-toggle equality, leaf retargeting and explicit flush controls. **7/7
  pass.**
- `tests/overlap.rs` covers both narrow 4 KiB direct-slot invalidation and the
  larger-mapping epoch fallback, including global mappings and ASI 6 permission
  edits. **2/2 pass.**
- `tests/mmu.rs::slot_generation_keeps_unrelated_fast_entries_after_wp_zero_fill`
  remains as the positive host-cache regression.

## Matched performance

The retained sources are `local/issue5-mmu-walk-2026-11-16/benchmark.rs` and
`large_bench.rs` in the same directory; results are in `benchmark.log`. The
workloads have distinct physical pages and checked nonzero checksums.

Five interleaved runs, pinned to host CPU 2, 10,000,000 accesses; all checksums
were asserted:

| Workload | Baseline `9271454` | Candidate | Change |
| --- | ---: | ---: | ---: |
| 32 distinct 4 KiB pages | 78.6578 | 78.7432 | +0.11% |
| Two alternating 4 KiB pages | 75.9548 | 75.9925 | +0.05% |
| Two alternating 256 KiB mappings | 73.1230 | 72.9693 | -0.21% |
| Two alternating 16 MiB mappings | 69.9843 | 69.1134 | -1.25% |

Larger mappings now advance the existing global epoch in O(1) instead of
clearing both 256-entry arrays. The candidate is close to the predecessor on
this matrix; the earlier 18.5% synthetic claim is withdrawn. An independent
review rerun measured the 16 MiB row at +2.57%; a focused seven-pair repeat
measured +0.63% (71.9000 versus 72.3514 ns/access). Both results are retained
under `local/review-issue5-mmu-walk-2026-11-16/final/`. The former fourfold
larger-page regression is removed, but these local microbenchmarks do not prove
zero overhead in every guest phase. Per-slot generations do not by themselves
reduce the WP=0 slot-zero fill storm, so no SS-5 guest speedup is claimed.

Validation: **720 normal tests / 723 perf-counter tests**, formatting, both
Clippy configurations and all four demos pass. The corrected overlap fixtures
are tracked in `tests/overlap.rs`.

## Next

The SS-5 bottleneck still needs a model-level explanation of WP=0 or a host
mechanism that can prove complete table-path coherence, not a leaf-only
shortcut. Do not change WP/TRCR semantics without independent evidence. Any
future cache must validate ancestor mappings, preserve bus faults and remain
outside architectural snapshot equality.
