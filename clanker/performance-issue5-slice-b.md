# Issue #5 Slice B: comprehensive differential validation

Status: implementation complete; issue #5 remains open. No cache, MMU behavior
or JIT changes are part of this slice.

## What was added

- `Machine::diagnostic_snapshot()` returns a `MachineSnapshot` containing:
  - a complete `Cpu` clone for CPU0 and, when present, CPU2. The clone includes
    all active and inactive register windows, CWP/WIM, PC/nPC, PSR, Y, TBR,
    error state and the complete MMU/FPU state. `Cpu` equality already covers
    those fields; translation perf counters are intentionally excluded.
  - exact scheduler state: virtual time, both next-ready deadlines, arbitration
    eligibility, last device-service deadline and per-processor counters.
  - both `CycleClock` snapshots: hardware/selected rate, mode, quantum, phase
    and remainder.
  - exact standalone MBus IOMMU state when the profile has one.
  - `SystemBusState`: layout, access context, configured processors, PROM
    cycle/phase count, device time, RAM/PROM and DRAM fingerprints, device
    fingerprint, Cgsix/SX/MXCC fingerprints and optional OpenBIOS state.
- `MachineSnapshot::differences()` returns category names instead of a single
  boolean: `cpu0`, `cpu2`, `scheduler`, `clock0`, `clock2`, `bus.memory`,
  `bus.dram`, `bus.devices`, `bus.sx`, `bus.cgsix`, `bus.mxcc`, `bus.prom`,
  `bus.context`, `bus.device_ns`, and the remaining bus/state fields.
- Bus/device/memory state is captured with deterministic FNV-1a fingerprints
  because raw 128 MiB RAM and full video/device clones would make checkpoints
  impractical. CPU state is exact. Fingerprints include all bytes/words or all
  derived device fields; they are intentionally not reversible. SCSI media
  bytes are represented by target metadata/state, not by a full image hash.
- Snapshot CPU clones detach the MMU's MXCC `Rc<RefCell<_>>` into snapshot-owned
  state. Live CPU/bus MXCC sharing is unchanged; a later live control write
  cannot mutate an already captured `MachineSnapshot`.
- `MachineSnapshot` has an explicit `Clone` implementation that runs
  `snapshot_cpu()` for CPU0 and CPU2. A cloned checkpoint owns independent MXCC
  state; mutating the clone cannot mutate the original or the live machine.
- The ESP fingerprint hashes the actual `Response` DATA IN/OUT bytes and
  direction/status in addition to the derived `Debug` state. The SCSI backing
  media file itself remains excluded and is documented separately.

The implementation is in `src/machine.rs`, `src/system_bus.rs`, `src/bus.rs`,
`src/dram.rs`, `src/devices/scsi.rs` and the small internal
`src/diagnostic.rs` helper. There are no step-path, MMU-behavior or device-
behavior changes.

## Validation behavior

`tests/differential.rs` covers:

1. Two independently constructed machines have no differences.
2. A changed inactive register window (save/add/restore) is detected after
   restoring PC/nPC; the active window is CWP 0 again.
3. FPU register changes and MMU initiator changes are detected.
4. RAM writes, NVRAM/device writes and `advance_ns` timing changes are detected
   under their distinct categories.
5. One executed step changes CPU and bus timing state.
6. Hardware-mode clock phase changes are detected.
7. Read-only bus probes do not mutate the diagnostic state.
8. Repeated identical stepped machines remain equal.
9. A real `TrapTaken` after ET is enabled enters the TT=0x870 handler with
   saved PC/nPC, CWP and TBR state.
10. An annulled branch skips a side-effecting local-register delay slot.
11. CPU2 state is compared separately from CPU0; a secondary FPU change reports
    `cpu2` without falsely reporting `cpu0`.
12. Page-table remap and TLB flush translate a virtual page to new data and
    replay identically on two machines.
13. IOMMU DMA reads/writes replay identically and change detected memory/IOMMU
    state.
14. Interrupt-controller source/mask state replays identically and detects a
    deliberate mask change.
15. Captured CPU0/CPU2 MXCC state is independent of later live control writes.
16. A CPU interrupt entry (level 10) replays identically and changes the
    snapshot's CPU/handler state.
17. Two ESP controllers holding different DATA IN payloads compare unequal
    before any PIO read, proving the staged buffer is part of the fingerprint.
18. `tests/snapshot_clone.rs`: cloning CPU0/CPU2 checkpoints and mutating the
    clone's MXCC leaves both the source checkpoint and live machine unchanged.

The deliberate mutations are applied to real machines, not fabricated snapshot
fields, so the tests demonstrate that the comparator observes state rather than
always returning an empty difference set.

## Deliberate scope boundaries

- This is not a semantic replacement for the interpreter, a MMU test suite or a
  device conformance test. It detects divergence in the represented state.
- SCSI media contents are not byte-hashed; target metadata and guest-visible
  state are included. Add an explicit media digest if a future DMA/cache slice
  can modify disk bytes during validation.
- Fingerprint collision is theoretically possible. CPU/scheduler/clock state is
  exact; fingerprints are used only for large RAM/device/video structures.
- `perf compare` remains the small ALU/memory smoke test. The new snapshot
  comparison is the comprehensive validation entry point.

## Validation

- `cargo fmt --all --check`
- `cargo clippy --locked --offline --all-targets -- -D warnings`
- `cargo clippy --locked --offline --all-targets --features perf-counters -- -D warnings`
- `cargo test --locked --offline`: **727 normal tests**
- `cargo test --locked --offline --features perf-counters`: **730 perf-counter tests**
- `tests/differential.rs`: 17/17 pass
- `tests/snapshot_clone.rs`: 2/2 pass
- all four built-in demos pass

Slice B has passed independent review; issue #5 remains open.

## Independent review status

The external review's MXCC alias, staged-ESP payload and snapshot-clone checks
now pass, as does its corrected inactive-window control. Its two `reported_*`
tests still encode the discarded faulty fixtures (`0x82002001` and `ta` with ET
disabled), so they fail by construction and are not validator regressions. The
corrected fixtures are the active-view assertion, real ET-enabled trap handler
test and side-effecting annulled-delay test in `tests/differential.rs`; the
clone isolation regressions are promoted to `tests/snapshot_clone.rs`.

## Next

Run this comparator around the first decoded-cache implementation and require
identical snapshots at instruction boundaries and checkpoints before accepting
that cache. Do not add JIT or alter MMU replacement behavior in the cache slice.
