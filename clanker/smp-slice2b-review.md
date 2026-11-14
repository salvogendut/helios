# SMP slice 2B review — 2026-11-14

Status: **slice 2B independently accepted at `4555aa2` and user-approved for
an incremental merge to `main` on 2026-11-14**, based on `d799ac2`. Both
correctness findings are resolved. Production execution remains single-CPU;
this does not complete issue #2 or the two-CPU acceptance gates.

## Final acceptance

The MBus IBAR field now preserves physical bits 35:14; unsupported high table
addresses cannot alias low RAM. Standalone page flushes match FA bits 30:12,
and the reserved high/low bits do not change which page is invalidated.
SS-5 retains its integrated field widths and flush interpretation. The useful
review cases are promoted into tracked tests, including profile masks,
high-address rejection, reserved-bit flushes, LRU and populated CPU mappings.
The ownership and explicitly unsupported disabled-mode behavior are documented.

Fresh independent checks on the corrected tree pass:

- All eight original review fixtures, including both MBus profiles.
- 560 unique normal core tests and 562 with `perf-counters`; successful
  summaries include one extra MXCC child execution in each configuration.
- Formatting, Clippy with `-D warnings` in both configurations and all demos.
- Heliacal's 29 tests and a release build against the corrected core.

Logs are under `local/review-smp-slice2b-2026-11-14/accepted/`. No production
code was changed during this re-review.

The implementer's final `local/smp-slice2b-2026-11-14/boot-review/` logs and
serial transcripts were inspected: SS-5, SS-20 and Axil reach root, print
`uname -a` and the acceptance marker, then sync and halt cleanly. The SS-20
write and subsequent read use the same disposable disk path; the post-reboot
checksum is `1354101890 16 /helios-slice2b-test`. Axil retains
`3034423527 17 /helios-disk-test`. These boots were not repeated by this review.

Next assignment: [slice 2C — MBus initiator identity and arbitration](smp-slice2c.md).
The findings, failing reproductions and return gate below describe the
original submission and completed correction work, not current blockers.

## 1. The MBus table-base register still truncates to the SS-5 field

`IoControl::write` uses `BASE_MASK = 0x07ff_fc00` for every profile. Under the
selected Sun-4M model, the standalone register's IBA field occupies bits
31:10 and supplies physical bits 35:14. The integrated SS-5 field is narrower.

Independent bus-level reproductions:

- Write `0xffffffff` to SS-20's `0xfe0000004`: readback is `0x07fffc00`,
  rather than the standalone field value `0xfffffc00`.
- Put a valid writable shadow IOPTE at RAM `0x10000`, pointing to `0x8000`.
  Write `0x08001000` to IBAR. Its intended physical base is `0x80010000`,
  outside the supported DRAM aperture. Instead, register bit 27 is discarded
  and the implementation reads the shadow table at `0x10000`.
- A DMA write to `0xff000000` then returns `Ok(())` and changes RAM `0x8000`
  from `0x11112222` to `0xdeadbeef`. This should reach the existing explicit
  unsupported-table-address stop without writing data through a low alias.

Make IBAR masking profile-specific and retain the full standalone address
through the existing DRAM-range check. Preserve the SS-5 mask. This does not
require expanding the supported RAM or DMA target range. The same register
decoder is used by Axil; cover both profiles and the SS-5 boundary in tracked
tests.

Primary evidence: [Sun-4M System Architecture](https://www.netbsd.org/~mrg/Sun4M_SystemArchitecture_edited2.pdf),
section 5.8.2, printed page 35; section 7.2.3 describes table-address formation.
The field diagram was inspected directly. The pinned QEMU implementation
also retains the narrower base mask, so copying that mask is not independent
evidence of an SS-20-specific exception. Any intended narrower hardware
profile needs a target-specific primary reference and explicit documentation.

## 2. Standalone page flushes compare reserved bit 31

`IoControl::write(+0x18)` queues `value & !0xfff`.
`StandaloneIommu::sync_flushes` then compares it against the entire cached
virtual page, including bit 31.

For the standalone address-based flush, FA is bits 30:12; bit 31 and the
bottom twelve bits are reserved. The tag comparison also uses VA bits 30:12.

Reproduction:

1. Cache DVMA `0xff000000` and `0xff001000`, mapped to RAM `0x8000` and
   `0x9000` respectively.
2. Change both backing PTEs to new pages without an automatic invalidation.
3. Write `0x7f0005a5` to `0xfe0000018`.
4. The first DMA read incorrectly returns the old `0x11112222` instead of
   the new `0x55556666`. The flush missed because bit 31 was clear.

Compare the standalone flush tag using FA bits 30:12. Both `0x7f0005a5` and
`0xff0005a5` must invalidate the same first page while leaving the second
cached page intact. Keep the integrated SS-5 flush interpretation separate.

Primary evidence: the same Sun-4M manual, section 7.2.4.2, printed pages
45–46, and the tag field in section 7.2.6.2, printed page 47. Page 46 was
inspected visually. Pinned QEMU records flush-register writes without modeling
this guest-visible standalone translation cache, so its stored register value
does not establish the tag-match behavior.

## Original independent coverage and positive results

Eight review fixtures are in
`local/review-smp-slice2b-2026-11-14/review.rs`: **five pass, three fail**.
Two failures cover the base-register issue and one covers flush matching.
The current assertions fail on SS-20 before proceeding to Axil; the fixtures
are parameterized for both profiles, which share these code paths.

The passing fixtures independently demonstrate:

- SS-5 retains its integrated IBAR mask.
- Exactly sixteen cached mappings and hit-driven LRU replacement: touching
  page 0 before inserting page 16 evicts page 1 and retains page 0.
- Big-endian byte/halfword DMA writes and preservation of upper standalone
  IOPTE PPN bits, with unsupported high targets rejected without low-RAM
  mutation.
- Populated CPU-0 and CPU-2 page translations, distinct contexts and live
  stale mappings survive DMA and I/O flushes. CPU-2 issues the controller
  flush through its ASI access path; interleaved translated CPU reads do not
  consume it. A CPU-0 local flush refreshes only CPU 0, leaving CPU 2 and the
  standalone cache stale until their own flushes.
- DMA request attributes do not inherit CPU supervisor or boot-mode state.

Promote the useful independent fixtures into tracked tests. The submitted
suite does not test LRU capacity/recency, despite the documentation claim.
Its initial CPU-isolation fixture changes context-table pointers but does not
populate distinct CPU translations; the stronger review fixture covers that
gap. Keep the positive cases when fixing the two failures.

## Original documentation corrections (resolved)

- Update `HACKING.md`'s DMA ownership paragraph: its assertion that the CPU
  MMU consumes all I/O flushes and no separate IOTLB exists is now specific
  to SS-5.
- `docs/io-control.md` still says SCSI DMA is pending; reconcile that with
  the existing SCSI implementation and the submitted disk-boot evidence.
- The disabled standalone mode can remain an explicit host stop within this
  slice, but document it as an unimplemented specified behavior. Sun-4M
  section 5.8.1, printed page 34, explicitly defines disabled ME as physical
  pass-through. Describing pass-through as invented or unspecified is wrong
  for the standalone profile. This is not a request to expand this slice.

## Original validation and return gate

Fresh independent core checks pass: **554 normal / 556 perf-counters unique
tests**, formatting, Clippy with `-D warnings` in both configurations and all
four demos. These tests do not cover the failing boundary cases above.

The implementer's SS-5, SS-20 write/reboot and Axil logs were inspected:
all contain the shell marker and clean-halt result; the Axil checksum is
`3034423527 17 /helios-disk-test`, and the SS-20 post-reboot checksum is
`1354101890 16 /helios-slice2b-test`. These boots were not repeated during this
review. Heliacal's tests/rebuild remain submitted evidence rather than a fresh
independent frontend run.

Review artifacts are under `local/review-smp-slice2b-2026-11-14/`:
`fixtures.log`, the normal/feature core check logs, two primary-manual page
renderings and the pinned `qemu-sun4m_iommu.c` comparison. Run the reproductions:

```sh
cargo test --locked --offline \
  --manifest-path local/review-smp-slice2b-2026-11-14/Cargo.toml \
  --test review -- --nocapture
```

Correct the two profile boundaries, promote the coverage, reconcile the
documentation, and return slice 2B for review with the required full checks,
Heliacal build/tests and disposable boot/persistence evidence. Preserve the
accepted SS-5 path. Do not commit, merge or start scheduling/startup yet.
