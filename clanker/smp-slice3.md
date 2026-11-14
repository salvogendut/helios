# SMP slice 3: deterministic execution and synthetic SMP

Next assignment on `feat/sun4m-smp`, starting from the accepted slice-2C
checkpoint that includes this file. The board foundations are accepted;
two-CPU execution and guest SMP support are not. Read `AGENTS.md`, the root
development documentation, [the SMP plan](smp-plan.md), the accepted slice
reviews, and the timing/reset/interrupt documentation before implementation.

Implement this bounded execution slice and return uncommitted for review.
Keep original-OBP secondary startup, two-CPU Solaris boot and CPU-count UI
for the subsequent milestones.

## Ownership and compatibility

- Introduce authoritative storage for the two independently constructed
  single-CPU modules on SS-20 and Axil: hardware processors 0 and 2, MIDs 8
  and A. Every CPU/MMU/FPU/MXCC state has one owner. Do not clone CPU 0 to
  create CPU 2 or swap CPUs through a fictitious CPU-0 slot.
- Retain one CPU by default and reject two CPUs on SS-5. Provide an explicit
  construction path for synthetic two-CPU tests. Presence, MXCC installation,
  interrupt/timer banks and scheduler storage must agree. No live hotplug.
- Preserve existing one-CPU callers where possible. If the public `cpu` field
  becomes accessors, migrate repository and Heliacal callers; never keep a
  shadow CPU merely to preserve that field. Heliacal still runs one CPU in
  this slice. Explain the selected ownership/API design in the handoff.
- Standalone IOMMU and board devices remain machine-owned. CPU-local ASI/MMU
  state and invalidations remain local. Keep slice 2C's scoped initiators,
  actual CPU privilege, DMA attributes and physical MXCC target selection.

## Scheduling and one shared clock

Write down the execution contract before coding. Use one host thread and
deterministic instruction interleaving with stable hardware-ID ordering for
ties. Per-CPU ready times plus one shared device clock are a suitable design.

- Preserve the one-CPU 100 ns functional quantum, PROM wait accounting,
  instruction-boundary interrupt sampling and precise stop/error behavior.
  Do not call the old whole-machine `step()` once for each CPU: this would
  multiply elapsed device time. Define exactly when shared device time and
  SCSI/Ethernet DMA service advance, including batches and tied CPU deadlines.
- Feed each CPU its own interrupt bank. Respect processor presence, AER grants
  and CPU error state. CPU 2 starts arbitration-disabled; an interrupt cannot
  grant it. State when a guest AER enable/disable becomes observable, how a
  local reset affects readiness, and what happens when no CPU can execute.
- Tag every instruction, synchronous trap, interrupt entry and error with its
  hardware processor number. Define aggregate/per-CPU budget and counter
  units. Device-only advancement or arbitration denial is not an instruction.
  Surface unsupported operations and errors; do not silently skip a failed
  secondary CPU or spin forever on it.
- Keep each guest atomic operation, its translation and fault handling inside
  one scheduling boundary. Shared coherent RAM suffices for this functional
  model; do not add cache timing, host CPU threads, JIT or idle parking.
- Whole-machine reset must reset all owned CPUs once and shared devices once.
  Module-local reset must preserve the other CPU and shared board state.
  Preserve configured topology and module identities across reset.

## Required synthetic evidence

Use hand-encoded or independently assembled guest programs with bounded
budgets on both SS-20 and Axil. Expected state must follow the architecture,
not the scheduler implementation. Prove:

1. Both hardware IDs make repeatable progress, with separate register/window,
   FPU, PC/nPC, MMU context and fault state. One CPU can be denied arbitration
   while the other runs; a guest AER write admits the secondary at the defined
   boundary. Trap/interrupt/host-stop attribution stays precise.
2. Shared-memory publication and contended `SWAP`/`LDSTUB` locks work without
   partial operations or starvation from an accidental fixed-order policy.
3. Directed software IPIs reach the intended hardware bank and return through
   normal guest trap code. Broadcast level-15 captures remain independent.
4. Identical virtual addresses can use distinct CPU contexts/mappings. A local
   TLB flush does not flush the other CPU. A guest IPI/handler handshake can
   perform a cooperative shootdown and observe the new mapping afterwards.
5. Equal elapsed virtual time yields equal system/processor timer, SCC and
   retrace progress with one or two CPUs. Include a PROM wait and an
   arbitration-disabled CPU; device time must not scale with CPU count.
6. CPU activity cannot steal standalone IOMMU flushes or change DMA identity.
   Local and whole-machine reset retain/clear exactly their documented state.

Return a short trace showing both processor IDs, PC/nPC, event category,
per-CPU counts and shared virtual time. Repeat the same run to demonstrate
deterministic ordering. Test counts alone do not establish synthetic SMP.

## Validation and stopping point

Run normal and `perf-counters` tests, formatting, Clippy with `-D warnings`
in both configurations and all four demos. Test/rebuild Heliacal. Repeat
default single-CPU SS-5, SS-20 and Axil boots through root, `uname -a`, the
acceptance marker, sync and clean halt on disposable clones; verify Axil
persistence. Preserve source media and the accepted evidence.

Compare fixed one-CPU work and a matched guest phase against the accepted
slice-2C baseline, using identical builds/configuration and bounded incremental
serial polling. Report distributions and any regression; SMP does not promise
higher host throughput. Update root/docs for changed APIs and timing, and
record local evidence in `clanker/` with an index entry.

Stop after synthetic two-CPU execution and unchanged default UP boots are
reviewable. No firmware/device-tree patches, direct Solaris entry jumps,
two-CPU OS experiments or frontend CPU-count setting. Issue #2 stays open.
