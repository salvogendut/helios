# Issue #5 device-service review — 2026-11-17

Status: **accepted after correction**, still uncommitted on `d97cf63`.
The production IRQ correction matches the suggested patch, and all six
independent fixtures are promoted into `tests/device_service_review.rs`.
No production device code was changed by this final review; the promoted
fixture's disposable image was moved from the repository root into the system
temporary directory, retaining its SCSI ID in the name and automatic removal.

## Final validation

- Independent full rerun: **727 unique normal tests / 730 with perf-counters**
  (728/731 successful lines, including the isolated MXCC child).
- All six promoted review fixtures and the existing idle differential pass.
- Formatting, both Clippy configurations with `-D warnings`, and all four
  demos pass.
- The three post-correction guarded/reference boot pairs have identical
  phase outcomes, MMU counters and serial transcripts.
- A fresh pinned SS-20 guest run completed login, disk I/O, eight alternating
  idle windows and clean halt. Guarded/reference idle medians were
  6.945680/7.376830 seconds per 100M steps (5.84% lower with guards).

The implementation note now separates initial and corrected boot timings and
records the slower corrected SS-20 pair. The controlled idle follow-up supports
a local idle benefit, not a uniform boot-speed claim. Timing and counter logs,
source hashes and final validation are retained under
`local/review-issue5-device-service-2026-11-17/final/`.

## Original finding: interrupt synchronization moved an observable boundary

The original `Machine::service_devices` implementation called `sync_interrupts()`
whenever either device poll was skipped. The unconditional path does not
necessarily synchronize in those circumstances: `service_scsi_once()` returns
immediately when SCSI DMA reset is held, and an idle Ethernet service returns
without synchronizing.

Reproduction uses normal host serial delivery, with the SCC receiver and its
PIL-12 interrupt enabled, CPU ET enabled, Ethernet idle and SCSI DMA held in
reset. Both machines start with identical snapshots. After injecting one byte:

```text
guarded:   InterruptTaken { pc: 0x1010, npc: 0x1014, level: 12, vector: 0x1c0 }
reference: Executed      { pc: 0x1010, npc: 0x1014, instruction: 0x10800000 }
```

The discrepancy reproduced through `step()` on SS-5 and through
`step_scheduled()` at the initial shared deadline on SS-20 with two CPUs. It
changes CPU state, interrupt entry and timing, despite the matching ordinary
boot runs. The optimization should preserve the reference boundary; a separate
intentional change to IRQ semantics would need its own rationale and validation.

The compensating synchronization is needed when **SCSI was skipped and SCSI DMA
reset is not held**. Skipping Ethernet alone does not require it. This preserves
the ordinary idle serial wakeup that the earlier naive ESP guard lost, and
retains the early-return behavior while reset is held.

The original suggested patch is retained in
`local/review-issue5-device-service-2026-11-17/suggested-fix.patch`. It was tested
first in a disposable source copy, which passed all six independent review
fixtures and library Clippy; the initial candidate passed four and failed the
two reset/IRQ fixtures. The production implementation now contains this
correction and passes the promoted fixtures.

## Independent fixtures and validation

`local/review-issue5-device-service-2026-11-17/tests/review.rs` contains:

- ordinary idle serial wakeup;
- held SCSI reset versus serial IRQ entry on the UP path;
- the same boundary through the two-CPU scheduler;
- missing-target timeout and IRQ acknowledgement;
- Ethernet initialization, queued RX filtering, payload DMA, TX poll expiry
  and transmitted payload, including SS-20/Axil with both CPUs enabled;
- SCSI PIO CDB execution, DMA arbitration denial, write-protected target,
  retained request/residual, error acknowledgement and successful completion
  on all three profiles.

The fixtures use guest WRPSR setup and checked execution outcomes, with full
diagnostic snapshots at relevant boundaries and independent payload/status
assertions. The SCSI fixture creates a new read-only 512-byte `scsi3` test image
and deletes it on completion. Existing firmware, disks, NVRAM and catalogs
were not used or modified by these tests.

Before correction, the candidate passed this independent full rerun:

- formatting and both Clippy configurations with `-D warnings`;
- 721 unique normal tests / 724 with `perf-counters` (722/725 successful lines,
  including the isolated MXCC child rerun);
- all four demos.

Those initial checks did not cover the failing boundary above. At that point,
the only new tracked test in `tests/device_service.rs` compared idle SS-5
machines. The promoted fixtures now cover the missing boundaries.

## Retained boot evidence

The initial review checked all three guarded/reference log pairs in
`local/issue5-perf-2026-11-15/device-service-2026-11-16/`. Phase execution counts,
traps, interrupts, resets, serial byte counts and MMU counter deltas match;
the complete serial transcripts are byte-identical. Reported total and idle
timings agree with the logs. These are individual local pairs, not repeated
median measurements. No fresh guest boot was performed in that initial review.
The final review also checked all three `*-fixed` / `*-ref-fixed` pairs and
ran the fresh SS-20 alternating-window benchmark described above.

## Completion

The IRQ correction, tracked fixtures and final validation are accepted. This
slice is ready to commit and push; issue #5 remains open. WP, page walks, JIT
and idle parking are unchanged. No commit or push was performed by this review.

Evidence directory: `local/review-issue5-device-service-2026-11-17/`.
It retains the fixtures, source hashes, `review-tests.log`,
`suggested-fix-tests.log`, `suggested-fix.patch`, `boot-evidence-check.json`
and `validation.log.gz`. Review-owned build directories and the disposable
patched source copy were removed; source media and the production implementation
were preserved.
