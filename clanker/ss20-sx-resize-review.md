# SS-20 resize acceptance and final-store boundary review

Independent review of Run 21 and the 491-test core, 2026-11-13. Saved checks,
pixel measurements and four isolated regressions are under
`local/review-2026-11-13-sx-resize-review/`. CPU/device code was not changed.

## Visual results accepted for the recorded workload

The supplied PPMs and geometry/trace are in
`local/review-2026-11-13-sx-resize/`. Independent checks confirm:

- All four 20/1/2/3-pixel moves preserve the translated text rectangle.
- After TERM-B is iconified, the identical text suffix in each of ROW-20
  through ROW-40 matches the unobscured ROW-20 suffix pixel-for-pixel.
- Full Size displays the larger terminal and its scrollback. Returning to
  Normal Size restores the **entire** original 484x316 client with zero
  differences, a stronger check than the harness's small preserved-text crop.
- Iconifying TERM-A restores all 152,944 client pixels to the desktop.
  Restoring it reproduces the pre-iconify client with zero differences.

The shrink measurement's 1,392 non-desktop pixels are **not stale rendering**.
The measured rectangle is `[141,605) x [443,885)`. The normal frame extends
through y=445, so the first three rows belong to its bottom border:
`464 * 3 = 1392`. Every pixel at y>=446 matches the desktop. Compute vacated
regions from frame rectangles, rather than a client rectangle plus a guessed
margin, and use exact comparisons outside surviving windows/icons/pointers.
There is no need to accept a 95% background threshold for this captured region.

The visual acceptance covers the observed Full Size/Normal Size transitions,
uncover and iconify/restore. It does not claim arbitrary resize geometry or
interactive Heliacal input has been validated.

## Remaining core defect: rejected stores can partially commit

The new unaligned long write path splits a transfer into bytes. The memory
command prepass validates each element's **start address**, but not its full
width. Consequently the command can modify VRAM before discovering that a
later byte or element is outside the decoded aperture.

Two isolated reproductions use synthetic boundary addresses, not Run 21
commands. Both set R10=`0xaabbccdd`, R11=`0x778899ee`, GO, and an all-ones
plane mask; initial affected VRAM bytes are `0x44`:

| Command PA | Word | Operation | Failure side effect |
| --- | --- | --- | --- |
| `0xfffffff8` | `0x8006c507` | One unmasked long at effective `0xffffffff` | VRAM byte `0x7fffff` becomes `0xaa` before the invalid next byte returns an error |
| `0xfffffff8` | `0x808ec503` | Two plane-masked longs, at `0xfffffffb` and `0xffffffff` | The first valid long commits; the later invalid long errors, leaving VRAM changed from byte `0x7fffef` onward |

These are writes to `SX_MEM_BASE + PA` through the public `Sx::write` API.
The first is an error inside a single unaligned transfer. The second shows
why guarding just the final element is insufficient: the whole vector must
be validated before any writes. This violates the documented retry/fault
atomicity contract, even though the current Solaris workload remains within
the aperture and completes successfully.

In the existing target-resolution prepass, validate every element's complete
`[offset, offset + width)` span against `VRAM_APERTURE_BYTES` before calling
`write_vram` for any element. Loads already perform the corresponding width
check. Keep lower-level splitting from mutating before discovering an invalid
span. Preserve the legal unaligned raw and BGR cases, including the protected
X byte when crossing a BGR word boundary.

`tests/unaligned_stores.rs` in the isolated crate contains those two failing
atomicity fixtures plus two passing controls for legal raw/BGR unaligned
stores. Expected data comes from byte order, aperture boundaries and the
project's failed-access contract; no new hardware encoding is invented.

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-resize-review/Cargo.toml
```

The selected-store count=1 change passes its tracked fixture, but the broader
array/direct mode rule remains an inference from captured traffic. Do not
promote `source >= 32 && count > 1` into an architectural encoding claim.
Complete SX coverage remains outside this desktop milestone.

## Validation and documentation completed here

The full core suite passes **491 tests** with loopback sockets enabled for
the existing network/serial fixtures. Formatting and all four demos pass.
Clippy initially rejected a redundant `as usize` in the newly added
`tests/sx_loads.rs:207`; this review removed the cast. Clippy with
`-D warnings` then passes, and all six load tests were rerun successfully.
The cast removal is the only tracked Rust edit made by this review.

README.md, HACKING.md, docs/machines.md, docs/graphics.md and docs/references.md
now distinguish the demonstrated Solaris/SX checkpoints from remaining
pipeline/cache, multiprocessor, instruction-coverage and frontend limits.
Obsolete claims that SS-20/Axil cannot boot an OS or have no SX model were
removed. Earlier installed-disk login evidence is not presented as a fresh
SS-5/Axil regression against this final branch.

No new guest boot, Heliacal rebuild, or frontend interaction was performed.
The isolated boundary tests remain under ignored `local/`; they have not
been promoted as failing tracked tests. Firmware and disks were untouched.

## Remaining merge work

1. Fix complete-span store prevalidation and promote the four boundary
   regressions. Verify the core checks again after that device change.
2. Rebuild/retest Heliacal against the final core, then validate actual typing,
   mouse interaction and capture/release. Existing frontend unit tests alone
   do not establish input acceptance.
3. Boot fresh private clones of the known-good SS-5 and migrated Axil disks to
   login, then shut down cleanly. The aborted SS-5 attempt is not acceptance.
4. Review the final diff and commit/merge only after these checks pass.

The observed desktop milestone is strong; the remaining core failure is a
bounded error-path fix and does not require restarting SX bring-up.

## Takeover follow-up

The complete-span store defect above is fixed and its fixtures are tracked in
`tests/unaligned_stores.rs`. Fresh SS-5/Axil boot checks and actual Heliacal
input checks have since passed. See [desktop completion](ss20-desktop-completion.md)
for the additional VSIMM identity fix, final validation and merge record.
