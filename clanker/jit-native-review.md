# Issue #5: native backend review and bounded corrections

2026-11-19. Candidate `412ba26` / implementation `a0472b3` on
`feat/issue-5-jit-prototype`. Parking integration remains accepted.

The candidate now emits and executes native x86-64 instructions in an isolated
optional crate. The previous review failures pass on both the decoded path and
an independently adapted native path. Three execution-boundary errors still
block merging. A small tested guard patch is retained; no larger backend or
parking redesign is needed for these corrections.

## Findings

1. **P1: cached blocks can exceed the remaining budget**
   (`src/machine.rs:2195`). Compilation respects `max_words`, but a cache hit
   executes the complete previously compiled block without a remaining-budget
   check. A loop containing two ADDs, a branch and its delay slot, run for five
   instructions, executes six. The second ADD runs early and virtual time
   advances too far. Fall back for the remaining suffix when a cached block
   does not fit, or use a separately validated shorter block.

2. **P1: an interpreted MMU-control write does not revoke native admission**
   (`src/machine.rs:2103`). `eligible` is computed only at call entry. A guest
   `sta` enabling translation executes through the interpreter; the next
   iteration still uses the old admission decision and reads native code
   directly from untranslated RAM. The fixture's translated ADD should set
   `%g3` to 7; the native path executes a physical decoy and sets it to 111.
   Recheck the live MMU/boot-mode restriction at every block boundary. Apply
   this restriction to the retained decoded experiment as well.

3. **P1: untranslated execution still needs the CPU's physical-address mask**
   (`src/machine.rs:2135`, `src/mmu.rs:1767`). Disabling the MMU does not mean
   every virtual address equals its physical address. The reference fetch
   masks with `MmuConfig::physical_mask`; the native path reads `u64::from(pc)`.
   At PC `0x80000000`, the microSPARC-II fixture should execute low physical
   RAM's ADD 1 but executes a high-address decoy ADD 9. A second fixture starts
   below the mask boundary and crosses it inside a block, also selecting the
   wrong second instruction. Prove identity over the complete candidate span,
   or fall back. Checking only the first address is insufficient.

These are three causes covered by four failing edge fixtures. The fifth new
fixture passes a 900-case independently calculated ADD/SUB/AND/OR/XOR and ICC
matrix, including carry/borrow/overflow, `%g0`, source/destination aliases and
global/out/local/in register destinations. This matrix also compares complete
diagnostic snapshots against the interpreter.

## Tested correction

`local/review-issue5-native-2026-11-19/conservative-native-fixes.patch` adds:

- live untranslated-execution checks to both block loops;
- whole-span physical-mask identity guards before either block decoder;
- a remaining-budget check before executing a cached native block.

The patch was applied **only to an isolated source copy**. All 34 focused tests
pass there: seven original decoded tests, six native tests, eight promoted
review fixtures, eight native adaptations of those fixtures, and five new
edge/matrix fixtures. The unpatched branch fails four of the five new tests.

Native memory handling was inspected: generated code uses the SysV x86-64
argument convention, bounded register/ICC offsets and caller-saved scratch
registers; the flags push/pop is balanced. The executable allocation is first
RW, changes to RX with `mprotect`, and is released on protection failure or
drop. No new unsafe operations were added to the core. This is a source audit
and Linux/x86-64 execution check, not a portability certification.

## Measurements

Submitted log arithmetic checks out: ALU medians 41.52 versus 34.26 ns/outcome;
memory medians 54.00 versus 55.565. The native path is limited to the declared
bare-metal subset; full workstation/device execution remains interpreted.

A fresh release build of the **guard-patched copy**, using the same
`jit-native` binary for both dispatch modes, gives:

| Workload | Interpreter | Native | Runs |
| --- | ---: | ---: | --- |
| ALU, 25M outcomes | 41.57 ns/outcome | 35.21 ns/outcome | Three pairs, order alternated |
| Memory, 7M outcomes | 54.59 ns/outcome | 55.195 ns/outcome | Two pairs, order alternated |

That is about 1.18x on ALU and 1.1% additional memory fallback time in this
bounded sample. Every checksum/count passes; native ALU compilation totals
26.9–72.5 microseconds. These observations establish that the tested guards
retain a synthetic benefit here; they do not establish Solaris acceleration
or replace a default-feature-off comparison against main.

## Finish this candidate

1. Apply the retained guard patch or an equivalent change. Promote
   `native_edges.rs` and the native versions of the earlier review fixtures;
   retain the independent expected-state assertions.
2. Rewrite the current design/results introduction around the final native
   implementation. The opening text still describes the discarded live-MMU
   discovery and fallback-invalidation claims. Clearly separate historical
   decoded measurements from the native candidate, document the x86-64 Unix
   scope and fallback behavior, and preserve the no-workstation-speedup limit.
3. Run final normal/native validation, formatting, Clippy, demos, and the
   small matched performance checks after staging the exact correction.
   Keep the backend default-off. No instruction-set, memory-JIT, SMP or parking
   expansion is part of this correction.
4. Return the corrected candidate for the merge check. Broader register
   marshalling, persistent caching and workstation acceleration are follow-ups,
   rather than additions to this acceptance gate.

The requested upstream research is in
[JIT reference comparison](jit-reference-comparison.md). QEMU and UltraViolet
both provide concrete examples of the missing state/budget guards.

## Reproduction and cleanup

Evidence: `local/review-issue5-native-2026-11-19/`. `native_edges.rs` contains
the four failing cases and matrix; `native_previous_review.rs` runs the earlier
eight fixtures through `run_jit_native`. Original and patched logs, the guard
patch, source/toolchain hashes, benchmark output and reference provenance are
retained there. To reproduce against `412ba26`:

```sh
python3 local/review-issue5-native-2026-11-19/run_review.py
python3 local/review-issue5-native-2026-11-19/run_review.py --patch
```

The runner creates and removes an owned temporary copy. The review's original
scratch source and release/debug builds were removed after validation. No
production source, guest media or Heliacal configuration was changed; no
commit, push or merge was made. The full submitted validation was not rerun
by this review; the independent checks above are explicitly scoped.
