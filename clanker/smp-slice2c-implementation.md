# SMP slice 2C: MBus initiator identity and processor arbitration

Implemented on `feat/sun4m-smp` from the merged slice-2B checkpoint `27c582e`.
This slice completes the board-state prerequisites for the deterministic
scheduler. Production execution remains single-CPU; there is no scheduler,
second execution loop, CPU-count UI, firmware/device-tree patch, synthetic
Solaris jump or two-CPU boot. Independently accepted after the corrections
recorded in [the review](smp-slice2c-review.md).

## Initiator contract

`src/bus.rs` now defines:

```rust
pub enum Initiator {
    Cpu(ProcessorModule), // actual module: processor 0/MID 8 or 2/MID A
    Dma { master: u8 },
    Host,
}

pub struct AccessContext {
    pub initiator: Initiator,
    pub supervisor: bool, // actual PSR.S pin, not ASI privilege
}
```

- `Bus::access_context` / `set_access_context` carry the attributes. The trait
  keeps `set_cpu_supervisor` as a compatibility spelling that changes only the
  supervisor bit.
- `Cpu::step_with_interrupts` saves the previous context, sets the executing
  module and PSR.S, runs fetch/execute/nested transfers, and restores the
  previous context on success or error.
- `Srmmu` stores its owning module (`initiator`) and scopes direct
  `load`/`store`/doubleword/exchange helpers with that initiator while
  preserving the caller's supervisor baseline. CPU/MMU reset retains it.
- `Machine::dma_read` / `dma_write` save the context and set `Dma { master }`.
  Standalone I/O uses `supervisor = false`; SS-5 integrated I/O explicitly
  supplies the CPU S pin and PCR.BM for memory-request capture. Both restore
  the previous context. The standalone IOMMU no longer changes supervisor state.
- `SystemBus::with_host_access` / `with_access_context` provide explicit scoped
  host/debug and test access. Direct `Bus` calls outside a scope use the
  host/debug baseline (no MBus owner) with the historical supervisor flag for
  direct CPU/MMU helper compatibility.

No per-access allocation, live `RefCell` borrow across a bus call or mirrored
CPU state was added. MXCC word/doubleword paths keep the accepted
snapshot/transfer/publish discipline.

## Registers and identity audit

Sources: Sun-4M System Architecture revision 50, section 5.1.2 (AER at
`0xFE0001008`, printed page 22), section 5.4.3 (MID register at
`0xFE0002000`, printed page 26) and section 5.5.3.3 (MID assignments); these
pages were inspected directly from the pinned manual. Existing microSPARC-II
references remain in `docs/references.md`.

- AER writable bits are 1:3, 16:20 and 31; bit 0 reads one; 15:4 and 30:21
  read zero. Standalone reset is `0x0000_0002` internally, reading as
  `0x0000_0003` (MID-9 split-requester set, MID-A/B clear). SS-5 retains its
  previous AER reset/readback with no processor effect.
- Standalone `+0x2000` is the read-only current-master MID register: 8 or A for
  the initiating module, 0 for DMA/direct host/debug (DVMA owner code). SS-5
  `+0x2000` keeps fixed MID 8 plus its six SBAE bits. Writes to the standalone
  MID register are ignored.
- MXCC port identity remains fixed per addressed module. Local ASI 2 uses the
  executing module; cross-module physical MXCC accesses select by physical
  address. Bridge/MACIO identity remains fixed.
- CPU/DMA memory-request capture (MFSR) and asynchronous SBus store capture
  (AFSR) retain the initiating context. SS-5 integrated DMA intentionally
  records the actual CPU S pin and PCR.BM (with CP=0) for I/O walks and target
  accesses. SS-20/Axil standalone DMA uses device attributes (S=0, BM=0, C=0,
  CP=0).
- SS-20/Axil AFSR captures the initiating module MID at bits 23:20 and the
  Sun-4M plain-store fields; AFAR is PA<31:0> and read-only. AFSR/AFAR reads
  preserve first-error capture; any AFSR write acknowledges and unlocks.
  SS-5 retains its fixed `1000` MID field and read/diagnostic semantics.
  Reading through another CPU does not rewrite the captured MID.
- SS-20/Axil plain-store errors assert held system source bit 29 (M-to-S write
  buffer) rather than ModuleError bit 30. Source masking is sampled at event
  time; per-CPU level-15 captures acknowledge independently; an AFSR write
  releases source 29 without clearing either CPU's captured level. ModuleError
  remains an independent level-15 input.

## Arbitration and run-state API

- `SystemBus::configure_processor` / `processor_present` record configured
  hardware presence without installing banks or CPU storage.
- `SystemBus::processor_arbitration` returns
  `ProcessorArbitration::{Absent, Disabled, Enabled}` using presence and the
  AER. Processor 0 is always enabled; processor 1/3 stay absent even when their
  AER bits are set.
- `SystemBus::set_processor_arbitration` and guest AER writes are the
  transition interface. Processor 0 cannot be disabled.
- `ProcessorModuleState::arbitration` / `run_state` expose
  `ProcessorRunState::{Ready, ArbitrationDenied, CpuError, Absent}`. Denial is
  not an instruction, trap or error mode; a future scheduler must leave the
  module unexecuted. A module's own `reset` clears CPU/MXCC/error state while
  retaining its MID. `Machine::reset_processor_module(0)` resets only the owned
  boot CPU/MMU and its MXCC.
- Pending software, timer and level-15 interrupts never change arbitration.
  Bank installation is independent of presence.

Functional timing: an AER write takes effect at the store completion; the next
instruction boundary for the affected module observes it and an in-flight
transaction finishes. Sun-4M parking/indeterminate delay is not modeled. No
scheduler or device-time advancement was added.

## Tracked regressions

`tests/smp_initiator.rs` (SS-20 and Axil where applicable):

- Independent CPU-0/CPU-2 modules read MID 8/A; local ASI 2 stays local; CPU-0
  and CPU-2 cross-module physical MXCC writes select the addressed module;
  host/debug reads MID 0.
- SS-5 fixed MID and absent MBus processor arbitration are retained.
- AER reset/readback, reserved writes, MID-A enable/disable, MID-9/MID-B absent
  slots and processor 0 eligibility.
- Interleaved CPU-0, CPU-2, DMA and host/debug success/error paths; MFSR
  privilege attribution; no identity/privilege leak into the next caller.
- Pending software/timer/level-15 interrupts with processor 2
  arbitration-disabled; separate CPU-0/CPU-2 bank delivery and the shared
  timer clock.
- Module-local reset versus whole-machine reset: CPU/MXCC isolation, retained
  AER/IOMMU/other module/RAM, and whole-machine reset of shared owner state.

`tests/io_control.rs` adds standalone AER/MID device-level cases and the
integrated SS-5 split. `tests/fault_capture.rs` covers SS-5 CPU S/BM capture
for DMA success, invalid PTEs, write protection and bypass, while standalone
device attributes remain S=0/BM=0. `tests/smp_initiator.rs` covers CPU-2 AFSR
MID capture, no-unlock reads, any-write acknowledgement, read-only AFAR,
retention/ME, cross-CPU reads, M-to-S source/per-CPU acknowledgement,
mask-at-event/reset behavior, and the SS-20/Axil async-fault paths.

## Evidence

Implementation-submission validation, 2026-11-14. Logs are under the ignored
`local/smp-slice2c-2026-11-14/` directory.

- `cargo fmt --all --check`: pass.
- Clippy `--all-targets -- -D warnings`: pass normally and with
  `perf-counters`.
- `cargo test --locked --offline`: 576 unique tests (577 raw successful
  summary lines including the isolated MXCC child), 0 failures.
- `cargo test --locked --offline --features perf-counters`: 578 unique tests
  (579 raw summaries), 0 failures.
- Independent review fixtures: original 9/9 and acknowledgement 8/8 pass.
- Built-in `demo`, `trap-demo --trace`, `serial-demo` and `timer-demo --trace`:
  pass.
- Heliacal via `distrobox enter my-distrobox` with the configured Xiri
  toolchain and `CARGO_HOME=/tmp/heliacal-cargo`: formatting, Clippy, 29 tests
  and `cargo build --release --locked --offline` pass.
- Fresh disposable single-CPU disk clones through OBP, root login, `uname -a`,
  `HELIOS-BOOT-ACCEPTED`, `sync` and clean halt:
  - SS-5: `SunOS helios 5.6 ... sun4m sparc SUNW,SPARCstation-5`, clean halt.
  - SS-20: `SunOS helios 5.6 ... sun4m sparc SUNW,SPARCstation-20`, clean halt.
  - Axil: `SunOS helios 5.6 ... sun4m sparc sun4m`, clean halt. First boot
    wrote `/helios-slice2c-test` (`1366120453 16`); a second boot on the same
    clone read the same checksum, and the existing
    `/helios-disk-test` remained `3034423527 17`.
  Source images were retained; only the completed-run `/tmp` scratch clones
  were removed. Serial transcripts, NVRAM, logs and commands remain under
  `local/smp-slice2c-2026-11-14/boot/`; the post-correction rerun is under
  `boot-fixed/`. The final fault-acknowledgement rerun is under `boot-final/`.
- Final independent acceptance adds two tracked field/combined-source tests:
  578/580 unique core tests, both independent suites (17 fixtures), formatting,
  both Clippy configurations, demos and Heliacal 29 tests/release build pass.
  Logs: `local/review-smp-slice2c-2026-11-14/accepted/`. The supplied final
  boot/persistence evidence was inspected rather than rerun.

## Limits

- No scheduler, CPU-count option, authoritative CPU-2 storage or two-CPU boot.
  CPU-2 integer state is still an independently constructed
  `ProcessorModuleState`; only its bus-installed MXCC participates in a
  whole-machine reset.
- AER parking delay is a functional instruction-boundary approximation, not
  cycle timing. Module reset signals and firmware wait loops are not separate
  emulator states.
- The MID register is a current-master read; no separate configurable MID
  register or split I/D requester variant is implemented.
- SS-5 still uses its integrated CPU/I/O TLB for DMA, now with explicit device
  attributes; this does not make it an MBus arbitration profile.
