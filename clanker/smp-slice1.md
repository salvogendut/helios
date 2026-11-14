# SMP slice 1: topology and independent module state

Implemented on `feat/sun4m-smp` from `3799eef`; review checkpoint before any
scheduler or two-CPU boot work. This revision also corrects the two blocking
host-failure paths and the port-policy gaps reported in
[smp-slice1-review.md](smp-slice1-review.md).

## Production changes

- Added `topology.rs` with the explicit MBus mapping: storage slot 0 /
  connector 0 / processor 0 / MID 8, and storage slot 1 / connector 1 /
  processor 2 / MID 10. Processors 1 and 3 (MIDs 9 and B) are absent.
- `MachineProfile::processor_topology` accepts one module for SS-20/Axil,
  two modules for SS-20/Axil, and no MBus module for SS-5. Machine defaults
  stay single-CPU.
- `ProcessorModuleState` constructs a fresh CPU, MMU diagnostics and MXCC for
  one module. It never clones or swaps CPU 0 state.
- `MxccState` carries its module MID and read-only identity. Reset zeroes
  register/data storage without changing that identity.
- `SystemBus` registers MXCC states by MID. It records the module identity at
  install time and decodes physical control/port addresses from that immutable
  metadata instead of borrowing live MXCC state on every bus access.
- ASI 2 word stores now use the same snapshot/transfer/publish discipline as
  doubleword stores. A legal RAM source or destination transfer no longer
  re-enters a live mutable `RefCell` borrow through `SystemBus` (the single-CPU
  regression reported by review).
- Before any module register or bus memory changes, stream source and
  destination targets are checked over their complete 32-byte span against all
  architected MBus module register windows and port words for MIDs 8, 9, A and
  B. A control-space stream returns an explicit unsupported error with no
  partial effects, covering both populated and absent modules, ASI and physical
  entry points, source and destination commands, and cross-module cycles.
  Ordinary RAM transfer failures keep the pre-existing address/RDY and
  partial-transfer behavior rather than becoming unconditional rollback.
- Width policy is uniform across reads and writes. Populated MXCC control
  windows allow byte and aligned halfword/word accesses and 8-byte-aligned
  register doublewords. Populated port words allow byte and aligned
  halfword/word identity reads with writes ignored. Absent control windows and
  the architected port words for MIDs 9, A and B zero/discard byte and aligned
  halfword/word accesses. Unaligned halfword/word accesses are `Misaligned`.
  Doubleword accesses are checked over their entire 8-byte span: any overlap
  with a zero window is `Unmapped` (or `Misaligned` when the start is not
  8-byte aligned), whether contained or boundary-crossing, and writes have no
  effects. The zeroing is limited to the documented empty-slot windows and
  architected module port words; unrelated physical space was not changed. The
  exact matrix is in `docs/machines.md`; the zero-window rows are a functional
  empty-slot convention, not an evidence-backed claim about absent-module bus
  response or narrow port cycles.
- Extended `tests/smp_topology.rs` and `tests/mxcc.rs` coverage for numbering,
  identities, independent CPU/MMU/diagnostic/MXCC state, widths and aliases,
  local ASI versus remote physical routing, distinct MMU contexts over one
  shared bus, reset isolation, absent MIDs, ASI word streams on both modules,
  cross-module streams, the isolated cyclic case, and the width-matrix cases
  for absent/populated ports, control windows and zero-window boundaries.

## Evidence

- Core: 533 unique tests normal, 535 with `perf-counters`. (The captured logs
  contain 534/536 successful test lines because the isolated cycle fixture
  re-executes one test in a child process.)
- `cargo fmt --all --check`, Clippy with `-D warnings` in both configurations,
  all four demos.
- Heliacal: 29 tests and build against the changed core.
- Independent review fixture: all six cases pass, including the word-stream RAM
  transfers, cross-module no-partial-effects check, isolated cycle child, absent
  ports and populated-port narrow lanes.
- Fresh disposable clones on the final width-policy tree: SS-5, SS-20 and Axil
  each reached serial login, accepted root, printed `uname -a` and
  `HELIOS-BOOT-ACCEPTED`, then halted cleanly to OBP. The Axil persistence
  check returned `3034423527 17 /helios-disk-test`. Logs and serial transcripts
  are under ignored `local/review-smp-slice1-2026-11-14/boot-width/`.

## Limits

No CPU scheduler, arbitration/startup handshake, directed interrupts, per-slot
timers, IOMMU ownership split, two-CPU OBP discovery or two-CPU Solaris boot is
implemented. MXCC error/control semantics remain incomplete, and control-space
stream rejection is a functional safety rule rather than a full MBus error
model. The second module is constructed only by focused tests for now.
