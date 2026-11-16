# Issue #5 Slice C: bounded decoded-instruction cache

Status: implementation and interpreter validation complete; the cache is
**opt-in and disabled by default** until its performance trade-off is settled.
Issue #5 remains open. No MMU semantics or JIT changes are part of this slice.

## Implementation

- `Cpu` owns a fixed **64-entry direct-mapped** cache. Entries are indexed by
  `(pc >> 2) & 63`, tagged by the full virtual PC and the raw fetched word.
- The cache stores only decoded integer plans: `SETHI`, the basic integer ALU
  group and integer `Bicc` branches. Every other instruction falls back to the
  existing interpreter. `Cpu::set_decode_cache_enabled(false)` clears and
  bypasses the cache; this is the default.
- Instruction fetch still goes through `Srmmu::load` with the current ASI and
  privilege each step. The cache never bypasses translation, permissions,
  faults or the physical bus.
- A hit validates the current raw word against the cached word, so self-modifying
  code and DMA/CPU writes are observed without an architectural invalidation
  hook or stale decoded state. No page/MMU semantics are inferred from this
  cache.
- Decode plans are non-architectural state: `Cpu` equality deliberately ignores
  `DecodeCache`, and reset/reconstruction clears it.
- The cache is bounded and owned; there are no host pointers, allocation or
  unbounded growth.

`examples/perf.rs fixed <alu|memory> [iterations] [cache]` can temporarily
enable it for measurement. The normal fixed benchmark remains cache-off.

## Validation

`tests/decode_cache.rs`:

1. Runs the built-in demo with cache on and cache off, comparing `Step` results
   and `MachineSnapshot::differences()` after every instruction. All steps and
   final registers/RAM match.
2. Enables the cache before execution, warms an ALU entry, replaces the
   instruction at the same PC with a different raw word, executes again, and
   checks the new result. This proves raw-word validation prevents stale
   self-modified instructions.

`tests/decode_cache_review.rs` promotes the independent cache-enabled fixtures:
ALU/SETHI operand/window/collision matrices, all integer branch conditions and
annul/displacement boundaries, guest code replacement and UNIMP fallback, page
remap plus execute-permission removal, IOMMU DMA code writes, and annulled
faulting slots with interrupt priority. Six of six pass.

The broader Slice B snapshots remain the differential oracle; the cache is only
enabled explicitly in these tests.

## Performance note

The independent measurement used five interleaved runs per variant/workload,
pinned to host CPU 2, 10,000,000 loop iterations (ALU 50M instructions; memory
70M instructions), release thin-LTO/one-codegen-unit builds with matching
checksums and zero traps:

| Median ns/step | Before Slice C | Cache off | Cache on |
| --- | ---: | ---: | ---: |
| ALU | 44.06 | 43.10 | 46.99 |
| Memory | 57.30 | 56.54 | 54.47 |

Cache-off showed no regression versus the pre-Slice-C baseline. Cache-on was
9.03% slower on the ALU loop and 3.66% faster on the memory loop. These are
microbenchmark results, not a guest-boot speedup claim. The cache therefore
remains disabled by default; broader decoded plans and a cheaper hit path are
required before enabling it for production. JIT remains separate.

## Validation

- `cargo fmt --all --check`
- both Clippy configurations with `-D warnings`
- `cargo test --locked --offline`: **720 normal tests**
- `cargo test --locked --offline --features perf-counters`: **723 perf-counter tests**
- `tests/decode_cache.rs`: 2/2 pass
- `tests/decode_cache_review.rs`: 6/6 pass
- all four built-in demos pass

## Next

Keep this cache as an opt-in candidate. Before enabling it by default, run the
boot/phase baseline and use the snapshot comparator at instruction boundaries;
consider decoded load/store plans and a cheaper hit path, but do not change MMU
behavior or add JIT in that slice.
