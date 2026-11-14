# SMP slice 2C independent review — 2026-11-14

Status: **slice 2C independently accepted for a feature-branch checkpoint**,
based on `27c582e`. MID attribution, SS-5 integrated metadata and standalone
fault acknowledgement are corrected. Production execution remains single-CPU;
issue #2 and the two-CPU acceptance gates are not complete.

## Final acceptance

Standalone AFSR/AFAR reads preserve the first fault. Any AFSR write clears
the captured status and releases the held M-to-S source at bit 29; AFAR is
read-only. Each CPU's captured level-15 pending bit remains independently
acknowledged. ModuleError bit 30 retains its own source state. SS-5 keeps its
integrated read/diagnostic behavior and actual CPU S/BM request metadata.

Fresh independent validation on the final tree passes:

- Original review fixtures: 9/9; acknowledgement fixtures: 8/8.
- 578 unique normal core tests and 580 with `perf-counters` (579/581 raw
  successful summaries include the isolated MXCC child run).
- Formatting, Clippy with `-D warnings` in both configurations and all demos.
- Heliacal's 29 tests and release build against the reviewed core.

Two additional tracked tests were added during final review: independently
encoded plain-store status/address fields and simultaneous M-to-S/ModuleError
capture, acknowledgement, reassertion and reset. They account for the increase
from the submitted 576/578 core tests. No production code was changed during
this final review; the documentation was reconciled.

Independent logs are under `local/review-smp-slice2c-2026-11-14/accepted/`.
The implementer's `local/smp-slice2c-2026-11-14/boot-final/` transcripts were
inspected: SS-5, SS-20 and Axil reach root, print `uname -a` and the acceptance
marker, and halt cleanly. Both Axil transcripts show `1366120453 16` for
`/helios-slice2c-test` and `3034423527 17` for `/helios-disk-test`. These boots
were not repeated by this review.

Next assignment: [slice 3 — deterministic execution and synthetic SMP](smp-slice3.md).
The findings and return gates below describe earlier submissions and resolved
correction work; they are retained as review history, not current blockers.

## Follow-up review findings (resolved): standalone fault acknowledgement

The new MID and plain-store field capture is present. SS-5 DMA again records
the CPU S/BM metadata while retaining DMA identity and restoring the previous
context. At this review stage the remaining issue was the standalone
AFSR/AFAR lifetime and interrupt wiring, previously requested under correction 1.

Sources: Sun-4M rev. 50 sections 5.2.1–5.2.2 (printed page 23),
5.7.3.1 (page 33, register diagram inspected visually), 6.2.1 and 6.4. The
cached primary manual remains at the path cited below. The online mirror was
unavailable during re-review.

Four reproductions failed on **both SS-20 and Axil**:

1. **An AFSR read releases the first-error lock.** Capture a CPU-2 word-store
   error, read AFSR, then generate a CPU-0 halfword-store error at a different
   address. The second error replaces the original MID/address/size instead
   of retaining the original capture and setting ME. `read_for(+0x1000)`
   still unconditionally clears `async_locked`. Standalone AFSR and AFAR
   reads must leave capture frozen until acknowledgement; SS-5 keeps its
   documented read-unlock behavior.
2. **Nonzero AFSR writes do not acknowledge.** Writing `0xffffffff` after a
   fault returns `0xff0e0000` on the next read, with ERR still set. The write
   path still installs SS-5 diagnostic fields and locks from the written ERR
   bit. On the standalone controller **any write value** acknowledges the
   fault, clears ERR and unlocks the capture. Written bits must not inject a
   synthetic fault. Preserve SS-5 diagnostic writes separately.
3. **AFAR is writable.** A guest write replaces the captured PA with
   `0x12345678`. The standalone AFAR is read-only. Preserve the full captured
   PA[31:0] until the next eligible fault; avoid guessing that SS-5's writable
   diagnostic-address policy applies to this register.
4. **M-to-S interrupt source/acknowledgement is absent.** Unmask only Mask All
   and SIPR/ITMR bit **29**, then produce a supported plain-store error.
   SIPR bit 29 remains zero. The producer still emits a ModuleError bit-30
   pulse. Sun-4M identifies M-to-S write-buffer errors as source **I, bit 29**,
   distinct from ModuleError bit 30. Hold this source until an AFSR write
   acknowledges it. Capture hard level 15 independently in each installed
   CPU bank when its source is unmasked at assertion time. Clearing CPU-0's
   pending bit must not clear CPU-2's bit or the underlying source; an AFSR
   write releases the source without acknowledging either CPU's already
   captured pending bit. Preserve SS-5's existing fault/interrupt policy and the
   accepted behavior of an independently asserted ModuleError source.

The source fix is confined to supported M-to-S plain-store errors. It does
not require ECC/VME producers, bursts, late errors, scheduling or a second
CPU execution loop. Retain source-mask-at-event-time semantics, Mask All and
independent per-CPU acknowledgement; do not clear every level-15 input when
acknowledging this one source. Update the old MBus bus-error pulse regression
and docs/interrupts-timers.md to distinguish this held source from ModuleError.

The documentation currently contradicts itself: docs/io-control.md says an
AFSR write clears the standalone interrupt/ERR, while docs/fault-capture.md
still documents read-unlock and diagnostic writes for both profiles. Align
both with the primary definition and the implementation. The implementation
handoff should also qualify its `supervisor=false` DMA description for SS-5.

New independent fixtures and logs:

```sh
cargo test --locked --offline \
  --manifest-path local/review-smp-slice2c-2026-11-14/Cargo.toml \
  --test acknowledgement
```

`acknowledgement.rs` contains four cases per profile; all eight fail on the
submitted correction. Its source/bank case also specifies the required
post-acknowledgement behavior, currently unreachable after its first failed
assertion. Promote these cases into tracked tests and add source-mask,
reassertion, simultaneous ModuleError, supported width/field and reset coverage.
Logs are under `local/review-smp-slice2c-2026-11-14/followup/`.

Fresh independent validation otherwise passes: original fixtures 9/9,
572/574 unique core tests (573/575 raw summaries including the isolated child),
formatting, Clippy in both configurations, all four demos and Heliacal's 29
tests/release build. The supplied `boot-fixed/` transcripts were inspected:
all three profiles reach the acceptance marker and halt; both Axil transcripts
retain checksums `1366120453 16` and `3034423527 17`. These boots were not
repeated by this review.

Return uncommitted after correcting this path and rerunning the fixtures and
required core/frontend/UP-boot checks. Do not start scheduling yet.

## Original review (the MID and SS-5 metadata reproductions below now pass)

The explicit initiator scopes, current-master MID register, AER reset/masks,
presence versus grants, and module-local reset structure are useful progress.
The existing suites pass. Independent fixtures expose incomplete MBus fault
attribution and a regression in integrated SS-5 memory-request metadata.

## 1. MBus asynchronous faults still identify CPU 2 as MID 8

`SystemBus::capture_sbus_write_error` forwards only the supervisor bit to
`IoControl::sbus_write_error`. The initiator is lost. `IoControl::read_for`
unconditionally ORs `0x0080_0000` into AFSR, retaining the SS-5's fixed field
on the standalone MBus profiles.

Reproduced on **both SS-20 and Axil**, with processor 2 configured and enabled:

1. Construct processor 2/MID A independently and install its MXCC/banks.
2. Issue a word store through its MMU to the floppy aperture using ASI `0x2f`.
   The existing width decoder returns a real `SBusError`; the plain store
   completes through the production asynchronous-fault path.
3. Read AFSR at `0xfe0001000`: the captured word is `0x90820000` and
   `(AFSR >> 20) & 15` is **8**, when the initiating module is **A**.

Primary source: [Sun-4M System Architecture rev. 50](https://www.netbsd.org/~mrg/Sun4M_SystemArchitecture_edited2.pdf),
sections 5.2.1–5.2.2, printed page 23, and 5.5.3.3. The local primary manual
was consulted at `/tmp/helios-page-list-review/local/sun4m.{pdf,txt}`; the
online copies were unavailable to the browser during this review. MID at
bits 23:20 identifies the owner of the faulted cycle. It is not the current
reader or a fixed bridge identity. The fixed `1000` field is an integrated
microSPARC-II definition and must remain confined to SS-5.

Pass the initiating module into capture and retain that MID with the fault.
Do not derive the captured MID from whoever later reads AFSR. Separate the
standalone register semantics from SS-5 while touching this path: the current
code also retains SS-5's 31-bit AFAR mask, size encoding and read-unlock
behavior. Sun-4M specifies PA[31:0] in AFAR, PA[35:32] in AFSR, distinct MBus
and SBus size fields, and write acknowledgement. Cover the supported plain
store/error subset and explicitly document any remaining unsupported fields.
Do not broaden this into ECC, burst or late-error implementation.

Track tests for CPU-0 and CPU-2 fault ownership on both profiles, reads by a
different CPU, first/subsequent-error retention and acknowledgement, physical
address preservation, and unchanged SS-5 fault-capture behavior.

## 2. SS-5 DMA capture incorrectly clears actual CPU pin metadata

The new `Machine::with_dma_context` sets supervisor=false on every profile;
the integrated branch of `dma_address` now hard-codes boot_mode=false for the
target request. This changes SS-5's previously intentional capture semantics.
An IOPTE walk still uses `Srmmu::request` and PCR.BM, making walk and target
metadata disagree as well.

Primary source: [microSPARC-II User's Manual rev. 1.1](https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf),
section 5.7.8, printed pages 96–98, especially page 97. MFSR records the CPU's
supervisor state and PCR boot-mode state. CP distinguishes CPU-originated
requests from I/O requests; C is zero for I/O. These integrated chip fields
must not be replaced with the standalone IOMMU's device attributes.

Independent reproductions:

- Reset SS-5 has PSR.S=1 and PCR.BM=1. Enable I/O translation/master 5 and
  point at an invalid IOPTE. The DMA request fails without a target access;
  MFSR is `0x00001010`, losing S. Expected capture is `0x01001010`.
- With PSR.S=1, PCR.BM=0 and a valid mapping, DMA succeeds but MFSR is
  `0x00000010`; expected is `0x01000010`. The same target path also discards
  BM when it is set.

Keep `Initiator::Dma { master }` and context restoration. For integrated SS-5
capture, supply the actual CPU S/BM values explicitly, independently of the
previous bus caller's context. Retain CP=0 and C=0. Preserve the accepted
standalone SS-20/Axil device attributes. Cover successful, invalid-PTE,
write-protected and bypass transfers with CPU S/BM variations, plus restoration
of an unrelated caller context.

Reconcile HACKING.md, docs/io-control.md, docs/fault-capture.md, the implementation
handoff and the changed expectations in tests/fault_capture.rs. HACKING.md
already retains the earlier CPU-derived SS-5 description, whereas the new
I/O-control text claims no CPU S/BM dependence. The primary chip definition
settles that disagreement; changing the remaining walk to BM=0 is not the fix.

## Independent validation and evidence

Review artifacts: `local/review-smp-slice2c-2026-11-14/`.

- Core normal and `perf-counters` suites pass: 569/571 unique tests; raw
  successful-summary totals are 570/572 due to the isolated MXCC child run.
- Formatting and Clippy with `-D warnings` pass in both configurations.
- All four demos pass.
- Heliacal's 29 tests and release build pass against this working tree.
- Independent `review.rs`: **5 pass, 4 fail**, covering the two findings
  above (MBus failure on each profile and two SS-5 capture cases).
- Passing review cases exercise actual CPU instruction dispatch/restoration,
  nested host/DMA contexts, doublewords/atomics/recursive MXCC errors, processor
  bounds/AER/reset transitions, and identity preservation across MMU/module reset.

Run the independent fixtures with:

```sh
cargo test --locked --offline \
  --manifest-path local/review-smp-slice2c-2026-11-14/Cargo.toml --test review
```

The supplied `local/smp-slice2c-2026-11-14/boot/` transcripts were inspected:
SS-5, SS-20 and Axil print the root acceptance marker and `uname -a`, then halt
cleanly. Both Axil write/read transcripts show `1366120453 16` for
`/helios-slice2c-test` and `3034423527 17` for `/helios-disk-test`. The two Axil
command files contain the stated write and subsequent reread. These boots
were not repeated by this review.

One additional documentation correction: docs/machines.md currently says
processor 2 is present on SS-20/Axil without qualifying configuration. The
production default still has only processor 0; CPU 2 becomes present only
after explicit `configure_processor(2)` in these fixtures.

## Return gate

Correct both profile boundaries, promote the fixtures into tracked tests,
reconcile the documentation, and rerun normal/feature checks, demos, Heliacal
tests/rebuild and disposable UP boot/persistence checks. Return uncommitted
for review. Keep scheduling, CPU-count UI and two-CPU boot outside this slice.
