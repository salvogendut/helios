# Terminal Bicc review

Helios `6e0fdf8` and Heliacal `b33eb55` are the starting revisions.
The submitted core passes seven machine-level branch tests against ordinary
interpreter stepping. They cover budget-two exit after CMP/BNE, fresh ICC,
annul, an unmapped page-edge slot, a relocated cached span, a slot that
writes ICC, a PIL 10 interrupt between the branch and slot, and an illegal
instruction trap in an executed slot. Interrupt and trap entry both save
the slot PC and the original target nPC in the trap window. Guest snapshots
and MMU translation counters agree in the differential cases.

## Two findings

1. **The native counter credits a host-evaluated branch.** The emitter
   writes x86 `0x90` for Bicc. After the generated call, Rust decodes the
   last word a second time, evaluates `self.icc.matches(condition)` and
   writes the guest PC/nPC. The histogram counts both instructions in
   CMP/BNE as native. The guest-state tests pass for this implementation,
   so a backend test must observe the exit selected by the emitted code.
2. **The public span compiler accepts a branch in the middle of a block.**
   Compiling and executing `NOP; BA,a target; INC %g2` changed %g2 from
   zero to one. SPARC annuls that slot. The workstation caller stops its
   scan at the first Bicc, which keeps this API defect out of its current
   execution path. The submitted compiler also accepts multiple branches
   and silently truncates overlength input.

## Tested correction for the other AI

Apply `local/review-workstation-jit-bicc-exit-2026-11-19/native-branch-exit.patch`
to Helios `6e0fdf8`. It includes the compiler correction, native lowering
and regression tests. It was tested in an isolated checkout.

- The generated function selects a pair of relative PC/nPC offsets using
  the post-prefix guest ICC and the branch's annul bit. Its return value
  carries the selected exit to the core. The core adds the executing
  branch PC with 32-bit wrapping. Cached code remains relocatable.
- The span compiler checks the full input and requires a supported prefix
  before a terminal Bicc. Single-word and straight-line compilation keep
  their existing opcode scope. The experimental backend's `execute`
  method now returns an optional `BranchExit` for its caller.
- Direct backend tests check 2,560 combinations of condition, ICC, annul
  and displacement, including the signed displacement limits. Another
  test starts with the opposite Z flag and checks the exit returned after
  CMP writes new ICC. The backend tests run without the machine's branch
  interpreter. Machine-level tests compare the resulting guest behavior.

The corrected checkout passed 814 default core test executions and 924
with `jit-profile,perf-counters`, including the isolated MXCC child run.
The native crate's three tests, formatting, Clippy with warnings denied
and all four demos passed. Heliacal at `b33eb55` passed all 126
profiling tests linked to the correction. Applying the two patches in
order reproduced the tested source files byte for byte.

## Next implementation

1. Adopt the correction and run the included tests. This checkpoint proves
   the terminal exit. It is useful independently of slot fusion.
2. Apply `next-fusion-test.patch` from the same evidence directory after
   the correction. It asks for CMP, BNE and a condition-code-changing
   slot in one call at a budget of three. It currently returns two
   outcomes. On success, the slot's reported incoming nPC and the final
   PC must still reflect the flags *before* the slot. Start with a
   non-annulled Bicc, then cover conditional annul. Count and commit
   mapped fetch effects only for executed instructions. Keep the split
   when the budget or device deadline ends at the branch.
3. Continue to ordinary RAM word LD, then ST, including precise faults,
   mapping changes and code-write aliasing. Run the three alternating
   same-binary Solaris pairs after those execution paths are present. Use
   the uninstrumented binary for timing and a separate diagnostic run for
   per-phase counts. The 10% useful-work target remains open.

The two patches, submitted/fixed test logs and fusion baseline are in
`local/review-workstation-jit-bicc-exit-2026-11-19/`. This review adds
no new Solaris timing sample. [Previous measurements and runner](workstation-jit-phase-review.md)
