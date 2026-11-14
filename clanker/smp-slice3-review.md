# SMP slice 3 review: accepted after corrections

**Accepted for the synthetic SMP milestone on 2026-11-14.** The feature branch
is based on accepted slice 2C at `6b11d3e`. Independent CPU ownership, per-CPU
deadlines, one shared device clock, guest AER/IPI/local-shootdown execution,
reset scope and default UP compatibility are now reviewable. Issue #2 remains
open: original-OBP secondary startup, two-CPU Solaris and CPU-count UI are
subsequent milestones, not established by this acceptance.

## Final correction review

All five original independent fixtures pass. The implementation corrects
shared-time scaling, isolates per-CPU PROM waits and clears the standalone
IOMMU on whole-system software reset. Tracked guest code now issues AER/IPI
stores and acknowledges/flushes/publishes completion in the CPU-2 handler.

The final review found and fixed two additional boundary defects:

- AER re-admission assigned the current time even when the processor still
  had an outstanding PROM deadline. It now clamps expired deadlines while
  retaining future deadlines. Both host and scheduled guest re-grants are
  covered.
- Scheduled software reset omitted the requesting store's completion time.
  It now stops both processors and charges that store's base quantum/PROM
  wait before clearing shared device/scheduler state. RTC rollover tests
  cover both stepping APIs, SS-5/SS-20/Axil, normal/waiting stores and CPU-2
  reset attribution.

`tests/smp_scheduler_boundaries.rs` additionally runs both authoritative CPUs
with distinct contexts and different mappings of the same VA while an I/O
flush is pending. CPU execution preserves the standalone invalidation; DMA
then observes the new IOPTE without changing either CPU MMU or leaking the
bus initiator context. These three tracked tests pass alongside the twenty
submitted scheduler tests. Seven independent review fixtures pass.

Final independent validation is under
`local/review-smp-slice3-2026-11-14/accepted/`:

- **601 normal / 603 `perf-counters` unique core tests**, all passing. The
  isolated MXCC child adds one rerun per suite; concurrent child/parent output
  interleaves one feature-run summary, so counting summary regex matches
  alone undercounts that log.
- Formatting, both Clippy configurations with `-D warnings`, all four demos,
  and `git diff --check` pass.
- Heliacal: 29 tests, formatting, Clippy and release rebuild pass. Its source
  tree remains unchanged.
- Supplied fresh SS-5/SS-20/Axil UP boot transcripts were inspected: root,
  `uname`, acceptance marker and clean halt. Axil write/reboot checks retain
  `2085311958 15 /helios-slice3-test` and
  `3034423527 17 /helios-disk-test`.
- The review also independently booted SS-20 on both baseline and final core,
  performed the matched Solaris measurement below, then synced and halted.
  Source media were preserved; the review's disposable disk clones were
  removed after completion. Logs, harness sources and NVRAM captures remain.

## Matched Solaris phase

Compared a clean archive of `6b11d3e` with the final core using identical
release/LTO settings, harness source, ROM, initial NVRAM, source disk clones
and guest commands. After login and 100M settling steps, each run measured
three successive 204.8M-step serial-idle windows without framebuffer decoding.
These are paired windows within one boot per version, not three independent
boot replicates.

| Version | Window times (seconds) | Median |
| --- | --- | --- |
| `6b11d3e` | 14.730, 14.782, 14.693 | 14.730 |
| Final slice 3 | 14.753, 14.964, 13.892 | 14.753 |

The median difference is about +0.16%, within the observed variation; this
establishes no meaningful UP throughput change and makes no SMP speedup claim.
Each paired window has identical instruction/trap/interrupt counts and sampled
PCs. Both runs halt after 1,955,700,000 steps; their serial transcripts and final
NVRAM images are byte-identical. `performance-summary.json` records the exact
values. The final saved NVRAM is `ss20-final-output.nvram`.

The high SS-20/Axil PROM aperture still lacks hardware wait accounting; tests
inject waits through the existing controlled low window. SCC remains
event-driven without a free-running baud clock. These documented limits do not
establish two-CPU firmware/OS support.

## Initial review record

The findings and return gate below describe the original rejected submission.
They are retained as history; the final correction review above resolves them.

## 1. Shared time is still charged once per CPU event

`src/machine.rs:858` advances the shared clock by the selected processor's
entire quantum and sets that processor's readiness to the resulting global
time. Starting from reset, every readiness remains at or behind global time.
Consequently the future-deadline branch does not implement independent CPU
cadences: the scheduler alternates processors while summing their durations.

Independent result on **both SS-20 and Axil**:

| Work | Shared time | System counter readback |
| --- | --- | --- |
| One CPU, 1,000 instructions | 100,000 ns | `0x19000` |
| Two CPUs, 1,000 instructions each | 200,000 ns | `0x32000` |

Installing the second running CPU halves each processor's functional rate.
The submitted timer test compares equal *aggregate event counts*, so it
passes while missing this requirement. Its 1,000-event trace assertion also
encodes the faulty timing policy.

The same defect makes one CPU's PROM wait stall the other. A controlled wait
fixture produces processor/time pairs `(0, 1229)`, `(2, 1329)`, `(0, 2558)`.
After CPU 0 issues the slow read, CPU 2 must have successive 100 ns execution
opportunities before CPU 0 is ready again. The third selected processor should
still be 2, not 0.

The wait fixture maps a synthetic ROM into the bus's existing low-address wait
window. This deliberately exercises a nonzero wait independently of the
missing SS-20/Axil high-PROM timing model; it does **not** assert that this low
alias or the SS-5 wait constant describes MBus hardware.

Required correction:

- Track each processor's next deadline independently, charging its base
  quantum and wait only to that processor. Advance shared devices to the next
  eligible deadline, never past another runnable CPU's earlier opportunity.
  Resolve equal deadlines by hardware ID, without advancing devices for each
  tied CPU. Preserve the UP `100 ns + wait` behavior.
- Define event timestamps and the clock position at API return. Reconcile
  AER admission/re-admission and module reset with the current clock so stale
  readiness does not replay missed opportunities. Preserve precise faults and
  bounded stop behavior.
- Define DMA service ordering at time advancement and tied deadlines. Newly
  due device work must be serviced at the intended boundary; adding a CPU
  must not accidentally multiply a time-based DMA service rate.
- Replace the aggregate-count timing oracle with equal per-CPU work/equal
  virtual-time assertions. Cover system and both processor timers, SCC and
  retrace, an arbitration-disabled CPU and nonzero per-CPU waits on both
  profiles. A controlled wait fixture is sufficient to test scheduling while
  the high-PROM hardware timing gap remains explicitly documented; do not
  copy SS-5 timing constants into SS-20 without evidence.

## 2. Whole-system software reset preserves stale IOMMU translations

`Machine::software_reset` at `src/machine.rs:953` resets the CPUs and bus
devices but never resets `standalone_iommu`. The bus clears the I/O controller
configuration and flush queue, so no queued invalidation repairs this later.

On both profiles, the independent fixture:

1. Caches DVMA `0xff000000 -> PA 0x8000`, containing `0x11111111`.
2. Requests whole-system software reset and services it through
   `step_scheduled`.
3. Reprograms the same table base with a PTE targeting `0x9000`, containing
   `0x22222222`, and reenables DMA.
4. Reads **`0x11111111`**, the stale pre-reset mapping.

Power-on reset clears the cache and module-local reset preserves it, as
expected. The software-reset omission already exists in the accepted
single-CPU helper; this is a pre-existing defect exposed by the required
slice-3 reset review, not a newly introduced translation algorithm bug.

Reset the standalone owner during whole-system software reset. Add tracked
coverage through both `step` and `step_scheduled`, including a reset-triggering
guest store and a preexisting bus request. Preserve local-reset isolation,
RAM, configured identities and the documented retained CPU/RTC state. Check
reset event attribution/counters as well. Avoid clearing the board IOMMU on a
module-local reset.

## 3. Complete the required guest integration evidence

The following submitted tests do not establish the behavior their names or
handoff descriptions imply:

- The AER test calls `cpu.mmu.store` directly from the host before scheduling,
  then disables CPU 2 with a host bus write. It contains no scheduled guest
  AER instruction and does not assert the exact admission boundary.
- The IPI test sets and acknowledges the software interrupt with host bus
  writes (`tests/smp_scheduler.rs:203,215`). Only handler entry, increment and
  return execute in the guest.
- The TLB test programs both CPUs in context 0 and directly invokes their
  MMUs. It verifies local cache separation, but no guest IPI/handler shootdown
  or distinct-context execution occurs.
- The timer test checks only the system counter. SCC/retrace and processor
  timer parity are absent from this scheduler suite.

Add bounded guest programs on both profiles. Host setup may load code, page
tables and initial state; the measured protocol must execute through the
scheduler. CPU 0 must issue the AER enable and directed IPI itself. CPU 2 must
acknowledge in its handler, perform its local TLB flush, publish completion
through shared RAM and return normally. Prove the stale mapping before the
IPI and the updated mapping after completion, while the other CPU's local
translation/context remains independent. Check handler completion and
post-return progress, not just an interrupt-entry counter.

Retain the existing separate component tests. Also exercise the required
standalone DMA invalidation/identity isolation with the authoritative scheduled
CPU owners, and cover processor-tagged trap/host-stop/reset outcomes. Use
bounded loops in all failure paths.

## Independent validation and reproduction

Review artifacts: `local/review-smp-slice3-2026-11-14/`.

```sh
cargo test --locked --offline \
  --manifest-path local/review-smp-slice3-2026-11-14/Cargo.toml \
  --test review -- --nocapture
```

Five independent fixtures: **two pass, three fail**. Every fixture checks
SS-20 and Axil. Failures are shared-time scaling, per-CPU wait isolation and
software-reset DMA mapping. Passing controls cover the denied-CPU cadence and
power-on/module-local IOMMU reset scope. The source and exact observed values
are in `review.rs` and `review-fixtures.log`. Promote these expectations into
tracked tests with the corrections.

Independently rerun against the submitted production tree:

- 589 unique normal core tests and 591 with `perf-counters`, all passing
  (590/592 successful summaries include the isolated MXCC child rerun).
- Formatting, Clippy `-D warnings` in both configurations and all four demos:
  pass. Socket tests were rerun outside the restricted sandbox.
- Heliacal: 29 tests, formatting, Clippy and release build pass. No frontend
  source changes; no interactive two-CPU frontend claim.
- Inspected the supplied SS-5/SS-20/Axil boot transcripts: correct `uname`,
  `HELIOS-BOOT-ACCEPTED`, clean halt back to OBP; Axil write/reboot reads retain
  `2085311958 15 /helios-slice3-test` and
  `3034423527 17 /helios-disk-test`. These guest boots were **not rerun** by
  this review, and source disks/NVRAM were not modified.

## Return gate

Fix the clock and software-reset scope, complete the required guest protocols
and clock/device cases, update the timing documentation and handoff, then rerun
full validation and disposable UP boots on the corrected tree. Include the
required matched **guest phase** against `6b11d3e` in addition to fixed UP
work. The submitted `ba .; nop` benchmark measures `Machine::step`, and its
numbers do not establish a matched guest-phase comparison or scheduler speed.

Return uncommitted with repeatable traces and passing tracked regressions.
Keep issue #2 open. No merge or two-CPU firmware/OS startup in this correction
round.
