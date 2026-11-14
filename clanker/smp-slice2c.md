# SMP slice 2C: MBus initiator identity and processor arbitration

Next assignment on `feat/sun4m-smp`, after the accepted standalone-IOMMU
slice 2B. This completes the remaining board-state work before the
deterministic scheduler in [the SMP plan](smp-plan.md). Implement this bounded
slice and return for review before committing or attempting a two-CPU boot.

## Objective

Make the initiating processor/device explicit at the bus boundary and expose
correct processor-arbitration state for the future scheduler. Today the bus
largely carries one CPU privilege flag, while `IoControl` stores arbiter bits
without controlling processor eligibility. Production execution must remain
single-CPU during this slice.

Read `AGENTS.md`, `README.md`, `HACKING.md`, `docs/references.md`, the SMP plan
and the accepted slice-1/2A/2B review notes before changing behavior.

## Required work

1. Establish the register/identity contract from primary sources. Sun-4M
   section 5.1.2 defines the Arbiter Enable Register at `0xfe0001008`:
   bit 0 reads one, the MID-9 enable resets set for split-requester modules,
   and the MID-A/MID-B enables reset clear. Our installed processors are
   hardware 0/MID 8 and hardware 2/MID A; an enabled absent slot does not
   create a CPU. Keep SS-5's integrated arbitration semantics separate.
   Record writable/reserved bits and implementation-specific choices.
2. Introduce an explicit initiator contract for CPU, DMA and direct host/debug
   accesses. CPU identity must come from the actual module, not the last
   register address or a compact vector index. Preserve the compatibility
   path for existing CPU-0/headless callers and migrate affected callers,
   including Heliacal if its API use changes.
3. Use the initiator where the documented guest-visible behavior depends on
   it, including supported request/fault attribution. Audit identity-bearing
   registers before changing them: a bridge identity may be fixed, a local
   CPU identity follows the initiator, and physical MXCC windows select the
   addressed module. Do not replace every literal MID 8 with the caller MID.
   Local ASI 2 and cross-module physical MXCC accesses must retain their
   accepted semantics and recursion/borrow safety.
4. Provide a clear query or transition interface for processor arbitration
   eligibility, accounting for configured presence and the AER enable bits.
   Distinguish arbitration denial, CPU error mode, module reset and a future
   firmware wait loop. Pending software/timer/level-15 interrupts must not
   silently enable arbitration for processor 2. Denial is not a retired
   instruction or a synthetic guest trap.
5. Specify how the future interpreter will observe enable/disable changes
   at instruction boundaries. Sun-4M describes bus parking and an
   indeterminate hardware delay; document the functional approximation rather
   than claiming cycle accuracy. Do not add scheduling or device-time
   advancement in this slice. Reset of one module must not reset shared
   board devices, the standalone IOMMU or another module.

Prefer explicit attributes or safely scoped context so CPU/DMA/debug accesses
cannot inherit a previous caller's identity or privilege after success,
failure or nested MXCC transfers. Avoid live `RefCell` borrows across bus
access and avoid per-access heap allocation. Do not swap CPU objects through
a fictitious CPU-0 slot or keep mirrored copies of authoritative state.

## Required regressions

- On both SS-20 and Axil, independently constructed CPU-0/CPU-2 modules
  produce the correct initiating identity. Direct accesses to the other
  module's physical MXCC window still select that target; local ASI 2 remains
  local. SS-5's existing identity behavior stays intact.
- Interleave CPU-0, CPU-2, DMA and host/debug accesses, including error paths,
  and prove identity/privilege do not leak between them. Preserve standalone
  IOMMU ownership, CPU-local translations and independent flush consumers.
- Verify profile-specific AER reset/readback, reserved writes, enable and
  disable transitions for processor 2, and correct handling of absent IDs.
  CPU 0 remains eligible; the reset MID-9 bit does not populate processor 1.
- Show that IRQs remain pending while arbitration is disabled and do not
  grant it. Preserve the separate processor-0/processor-2 interrupt and timer
  banks and the one shared device clock.
- Check module-local versus whole-machine reset, with memory, other-module
  state and IOMMU state retained or reset according to the documented scope.

Derive expected values from the hardware definitions, not the decoder under
test. Test the admission/state API directly with the two module objects; do
not add a second production CPU execution loop just to exercise it.

## Validation and stopping point

Run normal and `perf-counters` tests, formatting, Clippy with `-D warnings`
in both configurations and all four demos. Test/rebuild Heliacal. Repeat
single-CPU SS-5, SS-20 and Axil boots on disposable disk clones through root,
`uname -a`, `HELIOS-BOOT-ACCEPTED`, sync and clean halt; check Axil persistence.
Retain source media and logs, and remove only completed-run scratch copies.

Return the initiator/eligibility API, source references, tracked regressions,
validation evidence and remaining limitations. No scheduler, CPU-count UI,
guest firmware/device-tree patches, synthetic Solaris entry jumps or two-CPU
boot work. The following slice will integrate authoritative CPU storage and
deterministic execution, using the board state established here.
