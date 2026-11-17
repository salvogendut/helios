# Issue #5: device-service readiness guards

Status: accepted after the held-reset IRQ correction and final review. Changes
remain uncommitted. This slice does not change WP/TRCR semantics, MMU behavior,
JIT or idle parking; issue #5 remains open.

## Profile

The existing SS-5 boot profile (`local/issue5-perf-2026-11-15/profile/`) shows
the relevant service paths in the Solaris boot phase:

| Path | Self samples |
| --- | ---: |
| `Sun4mDevices::sync_interrupts` | 6.1% |
| `Machine::service_scsi_batch` | 2.1% |
| `SystemBus::interrupt_levels_for` | 2.0% |
| `Machine::service_ethernet` | 0.9% |

The all-phase profile attributes 12.6% to device/IRQ work. `service_scsi_once`
polled ESP and synchronized interrupts even when its state machine was idle;
`service_ethernet` built a request opportunity on every CPU boundary. The
guard keeps those calls when the device state can progress.

## Implementation

- `Esp::needs_service()` is true for every non-idle state except the selection
  timeout, whose expiry is advanced by device time. It also keeps the DMA
  request, ESP IRQ and DMA IRQ paths serviceable.
- `Ethernet::needs_service()` mirrors the exact idle RX/TX entry conditions:
  active phase, receiver enabled with queued input, or transmitter enabled with
  burst/poll readiness and queue space. IRQ/DMA state is included.
- `Machine::service_devices()` skips a device poll only when its predicate is
  false. Compensating interrupt synchronization preserves the unconditional
  SCSI service path, including its early return while DMA reset is held.
- `Machine::service_scsi`, `service_scsi_batch` and `service_ethernet` remain
  unconditional for direct callers and device tests.
- When SCSI is skipped, `sync_interrupts` is called only if DMA reset is not
  held. This preserves the reference instruction/interrupt boundary under held
  reset; skipping Ethernet alone needs no compensating synchronization.
- `Machine::set_device_service_guards(false)` provides a host-only reference
  path for differential validation. It is not architectural state and does not
  enter snapshots.
- No device timing, DMA ordering, SCSI byte accounting, IRQ priority or
  virtual-time state changed.

## Differential validation

`tests/device_service.rs` steps an idle SS-5 machine with guards enabled and
with unconditional service, comparing `Step` results and machine snapshots.
The six independent review fixtures are promoted as
`tests/device_service_review.rs`: held SCSI reset versus serial PIL-12 entry on
UP and SMP paths, ordinary serial wakeup, timeout/IRQ acknowledgement, Ethernet
RX/TX poll and DMA, and SCSI PIO/DMA denial, faults and completion. All six
pass. The complete SCSI, Ethernet, interrupt, DMA and SMP scheduler suites pass.
Matching guarded/unguarded boots were also compared for SS-5, SS-20 and Axil:

| Machine | Steps | Traps | IRQs | Serial |
| --- | ---: | ---: | ---: | --- |
| SS-5 | 1,940,900,000 | 1,323,593 | 25,304 | identical |
| SS-20/1 | 1,906,700,000 | 1,381,220 | 30,881 | identical |
| Axil/1 | 1,841,700,000 | 1,273,344 | 23,281 | identical |

Each pair completed login, useful I/O, idle and clean halt with the same
executed count, traps, interrupts and serial transcript. Post-correction
guarded/reference pairs were also checked for SS-5, SS-20/1 and Axil/1 with the
same exact outcome and serial equality.

## Matching boot and idle measurements

Initial, pre-correction serialized runs, same disk/NVRAM/ROM, guard-on versus
guard-off:

| Machine | Guarded total | Unguarded total | Guarded 100M idle | Unguarded idle |
| --- | ---: | ---: | ---: | ---: |
| SS-5 | 225.64 s | 233.73 s | 10.84 s | 11.28 s |
| SS-20/1 | 147.55 s | 151.69 s | 6.82 s | 7.12 s |
| Axil/1 | 142.30 s | 149.48 s | 6.81 s | 7.27 s |

Those initial pairs used 2.7–4.8% less wall time end-to-end and 3.9–6.3% less
during the 100M-step idle phase. The corrected implementation was measured
again in the `*-fixed.log` / `*-ref-fixed.log` pairs:

| Machine | Guarded total | Reference total | Guarded 100M idle | Reference idle |
| --- | ---: | ---: | ---: | ---: |
| SS-5 | 231.135 s | 240.523 s | 11.106 s | 11.543 s |
| SS-20/1 | 168.298 s | 157.939 s | 8.095 s | 7.314 s |
| Axil/1 | 144.092 s | 151.867 s | 6.911 s | 7.393 s |

SS-5 and Axil improve in these later pairs, but the later SS-20 pair takes
6.6% more total wall time and 10.7% more idle wall time with guards. Both
datasets are individual, instrumented local pairs, not repeated medians. Their
matching architectural outcomes establish the checked execution equivalence;
they do not establish a uniform boot-time improvement across hosts or phases.

A review follow-up pinned one SS-20/Solaris session to host CPU 6, warmed up
for 50M steps after disk I/O, then alternated eight 100M-step idle windows in
ABBA BAAB order (A=guarded, B=reference). The guarded wall-time median was
**6.945680 s**, versus **7.376830 s** for the reference, a **5.84% reduction**.
Process CPU-time medians were 6.92 s and 7.35 s. This confirms a local idle
benefit in the controlled follow-up; it does not establish a boot-time
improvement or make the differing earlier SS-20 measurements disappear.
Successive windows have their own guest time and counter deltas, so execution
equivalence is established by the separate matched boot pairs and regressions,
not by treating these windows as identical starting snapshots.

The follow-up used the instrumented release harness (`perf-counters`, thin
LTO, one codegen unit), a disposable disk clone and a copied NVRAM image. It
reached Solaris root, performed 16 MiB write/checksum/read work and halted
cleanly. Source disk, firmware and NVRAM hashes were checked unchanged.
Evidence is under `local/review-issue5-device-service-2026-11-17/final/`,
including `abba-summary.json` and the raw phase/counter log. The benchmark
build and scratch disk were removed afterward.

## Validation

- `cargo test --locked --offline`: **727 unique tests** (728 successful lines)
- with `perf-counters`: **730 unique tests** (731 successful lines)
- formatting, both Clippy configurations and all four demos pass
- `tests/device_service.rs` guard-on/off differential passes
- `tests/device_service_review.rs`: 6/6 pass
- `tests/scsi.rs`, `tests/ethernet.rs`, `tests/interrupts.rs`, `tests/dma.rs`
  and `tests/smp_scheduler.rs` pass

Evidence logs are under `local/issue5-perf-2026-11-15/device-service-2026-11-16/`.
No guest media, live NVRAM or catalog files were modified.
