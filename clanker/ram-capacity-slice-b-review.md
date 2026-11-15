# Configurable RAM: slice B review

Initially reviewed 2026-11-15 on `feat/configurable-ram`, against the uncommitted
slices A/B following `66acae4`. The SEGA correction was independently rechecked
on the same date. No production source or submitted test was changed by this
review; changes remain uncommitted.

## Verdict

**Slice B is accepted for progression to Heliacal slice C.** The reported SEGA
aliasing is fixed, the independent fixture now passes, and no blocking issue
remains from this review. The translated high-RAM DMA, layout, overlap and CLI
work remains covered by the full core suites and independent boundary fixtures.

The slice A enumeration-oracle and Axil-documentation follow-ups are addressed.
Heliacal and maximum-capacity firmware/Solaris acceptance remain later work;
issue #3 is not complete and this is not a recommendation to merge the entire
feature yet.

## Correction verification

- Standalone slot writes retain selector bits 21:16, and bypass construction
  widens to `u64` before shifting them into PA35:30. The board-range check sees
  the complete address before any target access.
- SS-5 keeps its SA30-only mask. The shared onboard slot still cannot bypass.
- Tracked tests in `tests/standalone_iommu.rs` reproduce the PA31 case on both
  profiles, exercise PA30 and an upper selector, verify readback and unchanged
  low RAM, check reset, and preserve the SS-5 reserved-bit control. The added
  `tests/io_control.rs` fixture checks the standalone mask and onboard slot.
- `docs/io-control.md` now distinguishes the two encodings and explicitly calls
  out the generic six-bit SEGA model, unresolved board-specific hardwiring and
  unsupported slot-register-access forms. This resolves the source/behavior
  reporting concern without claiming verified SS-20/Axil hardwired-bit behavior.

Independent rerun results on the corrected tree:

| Check | Result |
| --- | --- |
| Independent review fixtures | 7/7, including the original failing reproduction |
| Normal full core suite | 620 unique tests, 621 success lines |
| Full suite with `perf-counters` | 622 unique tests, 623 success lines |
| Formatting | Pass |
| Clippy, all targets, `-D warnings`, normal and counters | Pass |
| All four demos | Pass |

Commands, counts and logs are retained under
`local/review-ram-slice-b-2026-11-15/corrected/`. The temporary fixture build was
removed after verification. Heliacal remains unchanged, and guest boot checks
were not repeated in this correction review.

## Initial P2 finding — resolved

The following records the original failure and requested correction. The old
SS-5 assumption was on the bypass path changed in slice B; the newly expanded
translated DMA path was not the cause.

Relevant code:

- `src/devices/io_control.rs:181`: the slot-write mask is `0x0001_0003` for
  every expansion slot on every profile.
- `src/devices/io_control.rs:323`: bypass address construction uses only slot
  bit 16, again on every profile.
- `docs/io-control.md:148`: the description applies SS-5's SA30 rule to the
  standalone profiles and cites section 5.8 for both.

The [Sun-4M architecture manual, revision 50](https://www.netbsd.org/~mrg/Sun4M_SystemArchitecture_edited2.pdf),
sections 5.9 and 7.2.5.1, defines SEGA in slot-register bits 21:16 and combines
it with DVMA bits 29:0. Slot bit 17 supplies PA31. Appendix A.II.4.1.6's Campus-2
option hardwires SEGA35:32 to zero, leaving SEGA31:30. This differs from the
integrated microSPARC-II's single SA30 field. The section 5.9 register diagram
was also inspected visually. Any narrower SS-20/Axil implementation needs its
own primary evidence and a documented profile-specific choice.

The independent fixture performs this sequence on both standalone profiles:

```text
RAM[0x8000] = 0x11223344
AER = 1 << 16                         # grant expansion master 0
slot0 = 0x00020001                    # BY=1, SEGA selects PA31
dma_write(master=0, VA=0x8000, word=0xdeadbeef)
```

Under the documented standalone mapping this addresses `0x80008000`, outside
either board's DRAM aperture. It must fail without changing low RAM. Observed:

```text
Ss20:    slot readback=0x1, result=Ok(()), RAM[0x8000]=0xdeadbeef
Axil320: slot readback=0x1, result=Ok(()), RAM[0x8000]=0xdeadbeef
```

The upper selector was discarded before the new aperture check could see it.
An SS-5 control fixture confirms that ignoring reserved bit 17 there remains
the existing correct integrated-profile policy.

Correction requested at the initial review (now implemented):

1. Separate the slot address-field masks and bypass address construction for
   integrated SS-5 versus the chosen standalone controller model. Preserve all
   implemented standalone SEGA bits before applying the board's target check.
2. Reject resulting addresses outside DRAM without performing the target access.
   Preserve SS-5 reserved-bit/readback behavior and the onboard masters' existing
   inability to use expansion-slot bypass. Do not globally widen the SS-5 mask.
3. Add both-profile regressions for retained selector readback, invalid high
   selectors, untouched low sentinels, reset and the SS-5 control. A primary
   board-specific specification proving PA31 is hardwired would instead require
   an explicit documented override and independently justified fixtures; the
   generic specification's optional hardwired-fields note alone is not that
   evidence.
4. Split the bypass documentation by profile. Cite Sun-4M sections 5.9/7.2.5.1
   for standalone encoding, distinguish the board's implemented aperture from
   architecture address width, and retain explicit limits for other unsupported
   bypass/register-access forms.

This does not require adding another SBus device, burst/coherence support or
implementing the separate bypass slot-register-access mode in this RAM slice.

## Initial independent fixtures and standard validation

`local/review-ram-slice-b-2026-11-15/independent.rs` contains seven review fixtures,
with a manifest and `independent.log`. Run serially to bound allocation. Six pass:

- Complete-span predicates distinguish small-module mirrors, empty banks,
  overflow, zero length, the aperture endpoint and SX's reserved bank 7.
- A high preexisting ROM prevents DRAM installation atomically; a ROM at the
  exclusive 512 MiB endpoint remains valid.
- A page table in a high small-module mirror resolves the correct storage;
  byte/halfword DMA preserves neighboring lanes and does not alias low RAM.
  Success and misalignment failures restore the caller's access context.
- Changing IBAR retains a cached high mapping until a page flush; afterward an
  empty-bank table stops without modifying the earlier target.
- All four expansion masters can bypass into high RAM on both standalone
  profiles and reject the aperture endpoint and a nonzero PA30 selector.
- SS-5 preserves its reserved-bit-17 policy.

The seventh fixture, `standalone_sega_pa31_must_not_be_truncated_into_low_ram`,
initially failed on both SS-20 and Axil as detailed above. It now passes, and
equivalent regression coverage is tracked with the correction.

| Check | Result |
| --- | --- |
| Normal full core suite | 618 unique tests, 619 success lines |
| Full suite with `perf-counters` | 620 unique tests, 621 success lines |
| Formatting | Pass |
| Clippy, all targets, `-D warnings`, normal and counters | Pass |
| All four demos | Pass |
| Independent review suite | 6 passed, 1 failed |

Full core suites used host permission for the existing loopback socket tests.
The extra success line is the isolated MXCC child, not another unique test.
The core suite includes the submitted CLI, high-memory SCSI-write and Ethernet
transmit/receive tests. Heliacal tests and original-firmware/OS boots were not
run in this review, and are not implied by these results.

## Evidence, cleanup and next step

Compact logs, fixture source, manifest/lockfile and source URL/SHA-256 metadata
are retained in `local/review-ram-slice-b-2026-11-15/`. The temporary review
build, downloaded manual and rendered page were removed after inspection. No
source media, guest disks or live configurations were modified.

Continue with Heliacal slice C in the
[shared plan](ram-capacity-plan.md#slice-c--heliacal-configuration-named-machines-and-f9).
Use the core's layout/choice API for `ram_mib` validation and F9 choices; pass the
actual size and installed video to construction; include RAM in cold-replacement
detection; preserve named-machine identity and transactional drafts. Add the
frontend CLI override and test the resulting allocation, not only labels.
Return slice C with frontend tests, Clippy, formatting, a rebuilt release and SDL
smoke evidence. Keep maximum-size firmware/Solaris and interactive acceptance in
the slice D matrix. Issue #3 remains open until that acceptance is complete.
