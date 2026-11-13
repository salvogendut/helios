# SS-20 moves change geometry but corrupt packed pixels

Independent review of the 483-test funnel milestone, 2026-11-13. Production
code is unchanged. Evidence and six isolated regressions are in
`local/review-2026-11-13-sx-unaligned-load/`.

## Geometry passes; images do not

The supplied serial transcript confirms TERM-A client positions `+105+125`,
`+125+125`, `+126+125`, `+128+125`, and `+131+125`. These are real successive
20/1/2/3-pixel moves, and the run ends in a clean sync/halt to OBP.

However, `outline-run3-move2.ppm` damages text and borders, and
`outline-run3-move3.ppm` shows repeating vertical stripes across the title
bar, damaged edges, and corrupted text. The original captures are under
`local/review-2026-11-13-sx-funnel/accepted-run3/`. PNG copies are saved with
this review.

An independent pixel comparison uses the initial unobscured text rectangle
`[110,570) x [128,158)` and compares its translated position after each move.
TERM-B starts below this region and the pointer is above it. The stationary
text should preserve all 13,800 pixels:

| Capture | Total X translation | Different pixels |
| --- | ---: | ---: |
| large-drag | 20 | 0 |
| move1 | 21 | 0 |
| move2 | 23 | 2,030 |
| move3 | 26 | 1,676 |

These measurements establish corruption beyond visual impression. The first
two results accept only this text region, not the entire frame. Coordinates
and metrics are in `translated-text-comparison.json`.

## Exact captured outline reproduction

During `STAGE move1 RELEASE`, with last ROP control `0x55`, the trace records:

```text
SX REG 0x1010 <- 0xffffffff
SX REG 0x1324 <- 0x00000000
SX REG 0x1020 <- 0xb8e00509
SX CMD PA 0xff01c278 <- 0x8fd7d501
SX REG 0x1020 <- 0x9822952a
SX CMD PA 0xff01c278 <- 0x8f8fd501
```

This loads 32 X32 bytes at effective PA `0xff01c279` into R42 onward,
inverts them with ROP_M, then stores to the same bytes. The outline begins
at pixel `(121,100)`, byte lane 1. ROP_M processes nine registers R42..R50.
The analogous release sequences for move2/move3 use displacement 3/6
(byte lanes 3/2).

Replaying the captured sequence on synthetic nonuniform pixels fails without
any funnel instruction: its first X byte becomes `0xf0` instead of `0xf7`.
Drawing and erasing the same unaligned outline twice does not restore VRAM.
The aligned control does. These are recorded command sequences with an
independent, self-inverse pixel oracle.

## Missing packed-load alignment

Operation-1 stores now skip source prefix lanes according to destination
alignment, but packed loads still place every first byte in the high byte of
the first register and calculate `registers = count.div_ceil(4)`. The load
and store paths therefore disagree at unaligned addresses.

For the captured 32-byte transfer at lane 1, R42 must contain the first three
requested bytes in lanes 1/2/3, R43..R49 contain 28 bytes, and R50 contains
the final byte in lane 0. This takes **nine registers**, not eight:

```text
start_lane = effective_address & 3
register_count = ceil((start_lane + byte_count) / 4)
position = start_lane + byte_index
register = first_register + position / 4
shift = 24 - 8 * (position % 4)
```

Stage all reads into up to nine words, validate the complete destination
range, then commit. R119..R127 fits; the same transfer starting at R120
exceeds the supported range and must fail without mutation. Preserve aligned
load behavior and the corrected operation-1 stores.

The fixtures impose no values on unused prefix/suffix lanes; their exact
hardware padding behavior is not established. Do not read beyond the
requested memory bytes to manufacture padding. Operations 5/9 still need
their own evidence before changing their lane rules.

The original `sl_rect_copyrl_8` also assumes address-relative source lanes
when computing its funnel amount. A synthetic fixture exercises that DDX
algorithm across all 16 source/destination alignments with counts 1, 3, 14,
31 and 32. Current production first fails for source lane 1, destination
lane 0, count 1: it copies `0x00` instead of `0x4f`. The earlier promoted
copy fixtures only used aligned sources, so they missed this case.

Encoding and original driver provenance are in the
[funnel review](ss20-sx-funnel-review.md): Sun's SMC paper, the NetBSD SX
definitions and original Solaris DDX. The excerpt is saved at
`local/review-2026-11-13-sx-funnel/original-ddx-copyrl8.dis`. The byte-lane
constraint is derived from that driver and the captured load/store pair;
no physical-hardware differential validation is claimed.

## Regressions and validation

The isolated `tests/unaligned_load.rs` covers:

1. Captured outline inversion with all other VRAM preserved.
2. Drawing and erasing an unaligned outline restores the entire framebuffer.
3. An aligned outline control.
4. A 32-byte unaligned load occupies nine registers, including R119..R127.
5. A transfer requiring R128 fails before changing registers or VRAM.
6. The DDX copy sequence across all source/destination alignments.

Five fail; the aligned control passes. Parameterized tests currently stop
at their first failing case; later cases are available to validate the fix.

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-unaligned-load/Cargo.toml
```

Independently verified **483 core tests**, formatting, Clippy with
`-D warnings`, and all four built-in demos. The core suite used loopback
socket permission for existing network/serial fixtures. All 49 focused
SX/funnel/copy/outline/queue/mouse tests pass. The six new regressions remain
under ignored `local/`; no production or tracked tests were edited. No new
guest boot or disk modification was necessary. Heliacal's reported checks
and rebuild were not repeated by this review.

## Next acceptance run

**Geometry is accepted; correct rendering and merge readiness are not.** Fix
packed-load lanes, promote the regressions, and repeat from a clean clone.
Require both correct coordinates and preserved translated text/window pixels,
plus exact outline restoration. Account explicitly for pointer, focus and
legitimate exposed-region changes; they cannot explain this text corruption.

Then complete resize, uncover, iconify/restore and actual Heliacal keyboard
and mouse capture/release. SS-5/Axil boot regressions remain required before
merge. Geometry changes and no unsupported-opcode stops are insufficient to
claim correct desktop rendering.

## Applied implementation and rendering acceptance

Packed loads now derive their starting byte lane from the effective address,
place requested bytes at `start_lane + byte_index`, and compute the register
span as `ceil((start_lane + byte_count) / 4)`. A 32-byte load at lane 1 uses
nine registers; loads requiring R128 fail before register or VRAM mutation.
Aligned loads keep their previous eight-register behavior. The six isolated
regressions are promoted into tracked `tests/unaligned_load.rs`, and the
existing store alignment and funnel tests still pass.

The calibrated harness now asserts both geometry and the supplied translated
text rectangle after every release. A fresh run completed with:

```text
large-drag  TERM-A +125+125  dx=20  text_differences=0
move1       TERM-A +126+125  dx=21  text_differences=0
move2       TERM-A +128+125  dx=23  text_differences=0
move3       TERM-A +131+125  dx=26  text_differences=0
```

Captures, serial, trace and the geometry/pixel report are in `accepted-run4/`.
Validation: **489 core tests**, formatting, Clippy `-D warnings`, all four
demos, and Heliacal's 22 tests pass; the Heliacal release was rebuilt.
Resize/uncover/iconify, interactive Heliacal input, and SS-5/Axil boot
regressions remain before merge.

## Independent accepted-run4 verification

Reviewed the new production load path and reran the complete checks:
**489 core tests**, formatting, Clippy `-D warnings`, and all four demos pass.
Loads stage all requested bytes in nine words before committing and validate
the complete register span; the six promoted regressions now pass. Production
source hashes remained unchanged during this review.

The supplied serial transcript independently confirms all four geometry
deltas and a clean sync/halt. The text rectangle has zero differences after
each move. A broader comparison of the visible TERM-A frame also finds zero
differences, including its borders and title bar outside the pointer:

| Capture | Visible window pixels compared | Differences |
| --- | ---: | ---: |
| large-drag | 62,287 | 0 |
| move1 | 62,019 | 0 |
| move2 | 61,483 | 0 |
| move3 | 60,679 | 0 |

The broader comparison excludes pixels covered by TERM-B in either image,
the title-bar pointer area and the terminal caret. TERM-B's complete fixed
rectangle is also unchanged, and every pixel in the vacated left strip has
returned to the desktop background. Exact rectangles, exclusions, metrics,
source hashes and check logs are saved under
`local/review-2026-11-13-sx-unaligned-load/independent-run4-review/`.

**The tested move/rendering milestone is accepted.** This resolves the
previous packed-load corruption for the observed workload; it does not
establish all SX modes or the still-occluded TERM-A content.

Next, resize TERM-A both larger and smaller, regenerate the known text pattern,
move TERM-B away to expose previously hidden contents, and iconify/restore.
Check geometry and actual pixels after each operation. Then validate typing,
clicking/dragging and capture/release through Heliacal, and boot the known-good
SS-5 and Axil images to login using private clones. Those checks still precede
merge. No new guest boot or Heliacal interaction was performed in this review.

Before describing the final milestone, also reconcile stale status statements
in README.md and HACKING.md that still say installed/SS-20 OS boot or SX/VSIMM
devices are unimplemented. Retain the real limits on pipeline/cache accuracy,
multiprocessors and unverified graphics operations.
