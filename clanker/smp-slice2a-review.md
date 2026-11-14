# SMP slice 2A review — 2026-11-14

Status: **slice 2A independently accepted and user-approved for an incremental
foundation merge**, together with slice 1. The reviewed core is `a94f0aa` on
`feat/sun4m-smp`, based on `2bed9ff`. All four blocking findings are resolved.
Scheduling, startup arbitration, standalone-IOMMU ownership and two-CPU boot
remain unimplemented; production execution remains single-CPU and issue #2
stays open. Continue with the bounded slice-2B assignment after this merge.

## Final acceptance

The corrections preserve the NCR target/acknowledgement policy on SS-5 and
separate MBus source status from per-CPU event capture. Held-source, masked
event, deassertion and independent acknowledgement cases pass. Interrupt
synchronization iterates existing timer banks without heap allocation.

The final review tightened the standalone helper limit from 15 to 3 to match
the hardware processor-number domain; machine profiles still permit only 0
or 0/2. Invalid-ID tests now cover the 3/4 boundary and reject 4, 15, 16, 32
and 255 without mutation. The masked-event regression now clears Mask All
and checks pending bits directly, so delivery masking cannot hide incorrect
capture. An additional tracked regression exercises repeated internal bus-error
pulses, masked events and an externally held source on both MBus profiles.

Fresh independent validation of the final tree passes:

- All eight original review fixtures.
- 546 unique normal core tests and 548 with `perf-counters`; the isolated
  MXCC child contributes one additional successful log line per suite.
- Formatting, Clippy with `-D warnings` in both configurations and all demos.
- Heliacal's 29 tests and a release rebuild against the final core.
- The release allocation probe: **0 allocations for 20,000 device advances**.
- Fresh disposable SS-5, SS-20 and Axil boots to root, `uname -a`, the
  acceptance marker, sync and clean halt. Axil retains
  `3034423527 17 /helios-disk-test`.

The final independent logs, serial transcripts and saved NVRAM are under
`local/review-smp-slice2a-2026-11-14/corrected/`, with boot evidence in `boot/`.
The run used private copies of the implementer's cleanly halted `final3-*`
disks. Only those new review copies were removed after acceptance; source
images and evidence are retained. The supplied corrected boot logs under
`local/smp-slice2a-2026-11-14/final-corrected-boot/` were also checked.

Next assignment: [slice 2B — standalone IOMMU ownership](smp-slice2b.md).
The findings and original validation below describe the rejected tree and
the completed correction assignment, not outstanding blockers.

## 1. Separate the SS-5 and MBus target-register policies

`InterruptController::set_target` and `write_system` retain four target bits
for every profile. This is QEMU's extended CPU-number range, not the register
defined by the selected hardware.

- SS-5: writing 2 to `0x71e10010` reads back 2 and redirects shared interrupts
  away from the sole CPU. NCR89C105 specifies that this register reads zero
  and ignores writes.
- SS-20/Axil: writing `0xffff_fffe` to `0xff1410010` reads back 14. Sun-4M
  defines TARGET in bits 1:0, with bits 31:2 reserved; the result must be 2.
  The current result prevents delivery to an installed processor-2 bank.

Make the controller policy profile-aware. Preserve the NCR target register
on SS-5, implement the two-bit target on SS-20/Axil, and retain the useful
behavior where a valid but absent target leaves shared sources pending.
Correct the low-map register table and the four-bit extension claim in
`docs/interrupts-timers.md`.

Sources: [NCR89C105 specification](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/NCR89C105.txt),
address-decode table and uniprocessor-controller description;
[Sun-4M System Architecture](https://temlib.org/pub/SparcStation/Hardware/Sun4M_SystemArchitecture_edited2.pdf),
section 5.7.2, printed page 32. The target-field diagram was inspected directly.
The pinned QEMU comparison is
[`slavio_intctl.c` at `f71afb409a87c4ecd0f9d63a5963eb645dfab780`](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/intc/slavio_intctl.c);
its larger emulated CPU range is not the board specification.

## 2. Separate source status, broadcast capture and per-CPU acknowledgement

On the MBus profiles, `set_module_error`, `clear_level15`,
`acknowledge_level15` and source-mask writes still couple distinct states.
Independent bus-level reproductions show:

1. Leave the module-error source masked, clear Mask All, then assert the
   source: both CPU banks acquire hard-pending bit 15. The source mask must
   suppress broadcast capture at the time of the event; enabling it later
   must not manufacture a past event.
2. Enable the source, assert it, acknowledge CPU 0, then deassert the source:
   CPU 2 loses its unacknowledged pending bit.
3. Enable the source, assert it, acknowledge CPU 0, then set the system
   module-error mask: CPU 2's pending bit and the system source both clear.
   A source-mask write must not acknowledge another processor's broadcast.

For the MBus controller, keep physical source status separate from each
processor's captured event. Source deassertion clears the system source;
each processor clears its own pending copy. Source masks govern broadcast
capture, not acknowledgement. In addition, the last CPU acknowledgement must
not fabricate deassertion of a source that is still held externally; the
current implementation does so by clearing `sources` when all banks are clear.

Distinguish the existing internally latched module-error producer from the
held `set_source` input when implementing this. Trace how the producer releases
its source instead of using a source mask or another CPU's acknowledgement as
a substitute. Preserve SS-5's NCR-specific acknowledgement behavior where
applicable. Do not convert the documented Mask All/software-interrupt policy
merely to match QEMU; that is separate from the demonstrated broadcast errors.

Source: [Sun-4M System Architecture](https://temlib.org/pub/SparcStation/Hardware/Sun4M_SystemArchitecture_edited2.pdf),
sections 5.7.1–5.7.3 and 6.2.1, printed pages 31–33 and 37. These specify
per-processor acknowledgement, live system-source status and source masking
at broadcast time. NCR's single-CPU acknowledgement rule and QEMU's simplified
deassertion path are not substitutes for the MBus event model.

## 3. Validate bank installation before mutation

`Machine::install_processor_banks` currently accepts processor 2 on SS-5,
and accepts processor 32 on SS-20. After the latter, writing the system timer
configuration at `0xff1310010` panics in `Timers::write_system` at the
`1 << processor` expression. This requires an invalid host-side installation;
the guest cannot create a processor bank through that register alone.

Reject invalid IDs before either bank map changes. At the machine/bus boundary,
enforce the selected profile and the current topology: SS-5 is processor 0
only; the supported MBus module layout uses 0 and 2. The lower-level timer and
interrupt types must also reject numbers outside their hardware domain before
shifts or address calculations. Duplicate installation must be an idempotent
no-op, including pending state. Test rejection, unchanged state, duplicate
installation and reset with both valid banks.

## 4. Remove allocation from interrupt synchronization

`Sun4mDevices::sync_interrupts` collects timer processor numbers into a new
`Vec` before updating interrupt banks. This runs during ordinary device
updates, DMA service and MMIO.

An isolated release-build allocator probe, after warmup, measured:

| Version | Device advances | Allocations during those advances |
| --- | ---: | ---: |
| Accepted `2bed9ff` | 20,000 | 0 |
| Proposed slice 2A | 20,000 | 20,000 |

These are allocation counts, not a measured percentage change in execution
time. Iterate the disjoint timer/controller fields directly, or use bounded
stack storage. Keep the cached interrupt-level path and avoid unrelated
container refactoring. The allocator instrumentation is confined to the
ignored review harness; it is not a request for production unsafe code.

## Original reproduction and validation

Review material is in `local/review-smp-slice2a-2026-11-14/`:

- `review.rs`: eight independent fixtures; seven fail on the proposed code.
  The passing fixture verifies that adding a bank does not multiply shared
  elapsed time and that timer reset retains the installed population.
- `results.log`: exact assertion failures and the caught host panic.
- `allocations.rs`, `new-allocations.log`, `baseline-allocations.log`: the
  allocation comparison, with an exported `2bed9ff` baseline under `baseline/`.
- `fmt.log`, `clippy.log`, `tests.log`, `clippy-counters.log`,
  `tests-counters.log` and the four demo logs: fresh independent validation.
- `sun4m-032.png` and `sun4m-033.png`: local reference-page renderings.

Run the focused reproductions with the normal local Rust environment:

```sh
cargo test --locked --offline \
  --manifest-path local/review-smp-slice2a-2026-11-14/Cargo.toml \
  --test review -- --nocapture
```

Independently rerun checks pass: formatting, Clippy with `-D warnings` in
both feature configurations, **540 unique normal / 542 perf-counter tests**,
and all four demos. The extra child-process MXCC test output is excluded from
those unique counts. These suites currently omit the failures above.

The submitted SS-5/SS-20/Axil logs in
`local/smp-slice2a-2026-11-14/final-boot/` were inspected: all contain the
acceptance marker and clean-halt result, and Axil contains
`3034423527 17 /helios-disk-test`. These were not fresh boots performed by this
review. Heliacal's 29 tests/release build remain the implementer's reported
evidence; they were not rerun after identifying these core blockers.

## Original correction assignment (completed)

Fix the four groups above within slice 2A. Promote the independent behavioral
fixtures into tracked tests, supplementing them with held-source/last-CPU-ack,
duplicate-install and invalid-ID atomicity coverage. Preserve profile-specific
behavior and document the primary references. Rerun the allocation probe,
normal/feature tests, formatting, both Clippy configurations, demos, Heliacal
tests/build and disposable single-CPU SS-5/SS-20/Axil boots with clean halt and
Axil persistence verification. Return the corrected slice for review before
committing or starting the next SMP stage.
