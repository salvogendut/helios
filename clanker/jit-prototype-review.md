# Issue #5: first execution prototype review

2026-11-19. Reviewed Helios `3ac11c0` on
`feat/issue-5-jit-prototype`, based on integrated main `5fcffdc`.

The parking integration remains accepted. This execution prototype is **not
ready to merge**. It is a decoded-block interpreter, and six independent
correctness fixtures fail. Keep #5 open; these findings do not reopen parking.

## Execution backend does not meet the JIT milestone

`src/jit.rs:101` converts instruction words into a `Vec<Op>`.
`src/machine.rs:2015` iterates those operations and calls the existing Rust
ALU implementation through `Cpu::execute_jit_op`. No host code is generated
or executed. The design document states this accurately, but the feature/API,
commit title and result summary call it a JIT. Treat this as a decoded-block
experiment; the agreed native-code JIT milestone remains unimplemented.

Core `unsafe_code = "forbid"` is a requirement, not a reason to substitute
another interpreter for the requested backend. The work order allowed an
optional separate audited crate to own native compilation/executable memory.

## Reproduced correctness findings

The submitted seven integration tests pass. In an isolated copy of the same
commit, the independent suite passes two controls and fails six regressions.
Each expected result follows ordinary `Machine::step` plus explicit register,
PTE or fault assertions. No firmware or guest disks are needed.

### P1: compilation performs speculative architectural fetches

`src/machine.rs:1963` fetches up to 16 future words through the **live** MMU,
regardless of the requested execution budget. These reads populate TLB state,
write referenced bits and capture faults before the corresponding instruction
is executed. Falling back after a failed speculative fetch does not undo them.

- At the last instruction of a valid page, `run_jit(1)` executes the current
  ADD but captures SFSR `0x364` for the unexecuted next, unmapped page. The
  interpreter's SFSR remains zero.
- With that next page mapped, the same one-instruction run changes its PTE
  from `0x098e` to `0x09ae`, setting R before the page is executed. The reference
  leaves that PTE unchanged.
- When the current instruction lacks execute permission, compilation and
  fallback both attempt the faulting fetch. SFSR becomes `0x369` instead of
  the interpreter's `0x368`: one guest fault has become two fault captures.

Do not repair this by clearing fault registers after compilation: that cannot
restore earlier fault state, PTE writes, TLB replacement or device side effects.
For a bounded initial native prototype, explicitly falling back for MMU-enabled
or boot-mapped execution is preferable to adding an unproved speculative MMU.
Any discovery mechanism must be observational; real instruction accesses and
their effects still belong at the appropriate execution boundary.

### P1: a budget split loses branch-delay execution state

`src/machine.rs:1914` initializes `force_fallback` to false on every call.
Block admission does not require sequential PC/nPC. After `run_jit(1)` executes
`ba 0x10`, the next call starts at the delay slot with PC=4/nPC=16. Compilation
collects operations from 4, 8, 12, but execution changes PC to 16 after the
first operation and then executes the operation decoded from address 8.

The fixture's skipped `add %g2, 99, %g2` executes: `%g2` becomes 99 instead of
zero, and the target ADD is missed. The same program in one run passes, which
explains why the existing branch test does not detect this.

Admission must validate actual PC/nPC at every entry, including entry after
`step`, a short budget, trap return or host restoration. Keep nonsequential
delay-slot execution on the interpreter until entry is safe.

### P1: pending resets and latched errors return silent success

`src/machine.rs:1916` breaks before ordinary reset/error dispatch.

- A previously latched UNIMP/ET=0 fault makes the next call return
  `Ok(RunStats { executed: 0, stopped: None, ... })`; `step` returns the fault.
- A pending software reset produces the same apparently successful empty
  result, leaves the request pending and never reports `Step::SoftwareReset`.

Use the established interpreter/machine boundary handling and preserve its
outcome, error attribution and reset side effects.

## Performance and coverage limits

The retained benchmark arithmetic checks out: ALU medians are 41.10 versus
30.08 ns/outcome (1.366x); memory medians are 53.425 versus 136.98 ns/outcome
(about 2.56x slower). All retained benchmark runs report matching checksums,
outcome/instruction counts and zero traps/interrupts. These were audited from
the raw logs, not independently rerun; no Solaris speedup is established.

Do not expand memory instruction support merely to hide fallback overhead.
Use a bounded admission/cache policy so a memory-heavy unsupported region
can stay on the reference path without recompiling millions of blocks. Check
this again after the correctness fixes and native backend are in place.

The submitted invalidation claims exceed their fixtures:

- `guest_store_invalidates_cached_code_before_target_execution` writes a
  target before its first execution; it never warms an old cached target.
- `mapped_fetch_uses_the_current_translation` creates two separate machines;
  neither changes mapping after cached execution.
- `removed_fetch_permission_falls_back_to_the_interpreter_fault` starts with
  denied permission and checks an error string; no permission is removed
  from a warm entry, and it misses the doubled fault capture.

Promote the independent failures and add actual warm-cache self-modification,
remap/flush and permission-removal cases with full relevant state comparison.
The design's claim that fallback clears the cache is also unsupported by the
implementation: `Cache::clear` is not called by `run_jit`. Reconcile the design
with the final invalidation policy and verify changes within one run, since
the present cache is call-local.

## Bounded next assignment

1. Keep `3ac11c0` unmerged and preserve its measurements as a decoded-block
   experiment. Correct the terminology; do not close #5 with this backend.
2. Carry the review fixtures forward into the native candidate. Fix ordinary
   reset/error dispatch and sequential-entry eligibility first. Limit initial
   acceleration to a proved device-free, single-CPU, MMU-disabled ordinary-RAM
   case if needed; dispatch every other configuration through its appropriate
   interpreter/scheduler path or reject unsupported execution explicitly.
3. Implement native compilation for the already agreed SETHI/basic-ALU subset
   in an optional isolated backend. Preserve core safety policy, MSRV, precise
   state and timing; do not add branches, memory operations, SMP acceleration
   or wider MMU shortcuts in this slice.
4. Once correctness passes, remeasure fixed ALU and memory workloads with
   identical builds, compilation separated, and no material default-path or
   fallback regression. Return the candidate for review. No additional parking
   experiments or Solaris boot campaign is needed to resolve these fixtures.

## Evidence and cleanup

`local/review-issue5-jit-2026-11-19/` contains `jit_review.rs`, the failing
`fixtures.log`, `run_review.py`, source hashes/toolchain/command in `build.json`,
benchmark/integration arithmetic in `audit.json`, and `cleanup.json`.

Reproduce from the reviewed commit with:

```sh
python3 local/review-issue5-jit-2026-11-19/run_review.py
```

The runner copies tracked source into an owned temporary directory, adds only
the review fixture there, runs the seven submitted and eight independent
tests, and removes its scratch tree. Promote the fixture into tracked tests
when addressing this review. The original scratch build was also removed.

Local main refs match Helios `5fcffdc` and Heliacal `682d85b`. The retained final
production smoke includes login, echo-disabled recovery/cleanup, sync, halt
and exit 0. This review did not rerun the full reported validation or guest
smoke, touch guest media, change production source, commit, push or merge.
