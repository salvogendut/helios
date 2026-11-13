# SS-20 SX funnel shift and window-move acceptance

Independent review of the 475-test paired-IQ milestone, 2026-11-13. Production
code is unchanged by this review. Evidence and eight isolated regressions are
in `local/review-2026-11-13-sx-funnel/`.

## The larger gesture has not moved TERM-A

The supplied serial log contains `xwininfo -root -tree` before and after the
larger gesture. Both report frame `494x346+100+100` and TERM-A client
`484x316+5+25 +105+125`. TERM-B is also unchanged.

Compared with `before.ppm`, the held image differs at 3,432 pixels, while the
released image differs at only 88 pixels, at the old/new pointer locations.
The outline appears and is erased; the window does not move. Consequently,
different held/released captures establish outline activity, not acceptance
of a 20-pixel window move. The subsequent small gesture reaches the copy
routine on **release** and stops at the funnel instruction below.

Require an actual frame-coordinate delta after release. For the large gesture,
try a second paced motion while held and observe outline coordinates; the
reason the first gesture produces zero displacement is not yet established.
For 1/2/3-pixel cases, validate the final geometry against the geometry before
that gesture, with both X server acceleration and WM drag thresholds considered.
Do not infer movement from pointer position or image inequality.

## Decode of the next command

The supplied trace ends with:

```text
SX REG 0x1010 <- 0xffffffff
SX CMD PA 0xff42cdc0 <- 0x86d7c584
SX REG 0x1020 <- 0xc8e28518
FAULT PC=0xee65a44c ... Unsupported("SX shift instruction")
```

The original Solaris DDX maps that PC to `sl_rect_copyrl_8`, ELF `0x1a44c`:
SPARC instruction `0xd0258000`, `st %o0,[%l6]`. This is the right-to-left
8-bit rectangle copy routine.

`0xc8e28518` decodes as `SX_FUNNEL_I(10,24,10,8)`: source R10, immediate
24, destination R10, **nine output words**, using the four-bit ALU count
field. For this nonzero shift the vector operation is:

```text
new[R10+i] = (old[R10+i] << 24) | (old[R11+i] >> 8), i=0..8
```

It reads R10..R19 and writes R10..R18. It is not an independent shift of
each word. Snapshot the complete source range before mutation, including
the extra source word. Operation 6 is the corresponding scalar-register
funnel: snapshot that shift operand too when the destination overlaps it.
Handle shift zero without a host shift by 32. Preserve neighbors and the
paired-IQ rollback contract. Validate the complete supported operand range
before writes; do not invent wrap or large-shift behavior from these fixtures.

Independent references:

- [NetBSD SX encodings, revision 1.22](https://cvsweb.netbsd.org/bsdweb.cgi/src/sys/arch/sparc/dev/sxreg.h?rev=1.22&content-type=text/x-cvsweb-markup),
  shift-group operations 6/7 and `SX_FUNNEL_S/I`. A local copy is available at
  `../NetBSD-xiri/sys/arch/sparc/dev/sxreg.h`.
- Sun's [Pixel Processing in a Memory Controller](https://www.sunhelp.org/history/pdf/smc.pdf),
  section 4.1.4: funnel shifts cross source-vector elements using a scalar or
  immediate shift. Table 3 describes four packed bytes per register.
- Original `ddxSUNWcg14.so.1` and `ddx.dis` in
  `local/review-2026-11-13-sx-plot/`. The exact copy routine is saved in this
  review's `original-ddx-copyrl8.dis`. Its shift is calculated from the
  difference between source and destination byte alignment.

`tests/funnel.rs` contains six isolated fixtures: the captured word/count,
8/16/24-bit alignment, zero/1/31-bit shifts, overlapping destinations, a
scalar shift register overwritten by the result, and the final source pair
ending at R127. The oracle concatenates an independent bit stream; source
values are synthetic and nonuniform. All six stop at the unsupported shift.
These tests do not establish hardware semantics for shifts >=32 or register
wrap past R127.

## Check packed alignment together with the funnel

The preceding captured command `0x86d7c584` loads 14 B32 bytes from effective
PA `0xff42cdc4` into R11 onward. The driver deliberately starts at R11 on
this branch and funnels from R10. At ELF `0x1a064..0x1a074` it computes
`8 * ((source & 3) - (destination & 3))`, adding 32 for a negative result at
`0x1a3d4`. The captured immediate 24 and aligned source therefore imply
destination byte lane 1.

For a synthetic one-pixel rightward copy at these coordinates, the derived
next store is `0x868fc505` to PA `0xff42cdc0`: 14 packed bytes from R10,
plane-masked, destination displacement 5. After the funnel, R10's high byte
is an unused prefix from the old R10, followed by the first three source
bytes. Correct copying requires skipping that prefix lane at the unaligned
destination. Current production always takes R10's high byte first.

The isolated **store-only** fixture seeds the expected post-funnel stream,
so it can expose this independently of the unsupported opcode:

```text
expected: 12 83 e4 05 56 a7 38 c9 4a fb 6c 9d 2e 7f
actual:   e1 12 83 e4 05 56 a7 38 c9 4a fb 6c 9d 2e
```

This is a driver-derived alignment constraint, not a recorded post-funnel
hardware result. The guest trace stops before that store; the exact following
PA and register operands were not dumped. Confirm them on the next run. The
second copy fixture composes load, funnel and store for rightward moves of
1/2/3 pixels and checks all VRAM bytes, including other channels and guards.
It currently stops at the funnel. The store-only fixture fails first at
delta 1; its later delta 2/3 cases will execute after that is fixed.

Investigate packed load/store byte lanes together. Do not blindly change
conditional glyph stores based only on this operation-1 copy case. Existing
unaligned glyph fixtures use repeated source bytes and cannot distinguish
source lane selection; the nonuniform six-byte fixture is aligned. Add
nonuniform unaligned coverage backed by the DDX before generalizing.

## Harness, validation and merge gate

The supplied harness now stops on graphics `Step::TrapTaken`, but its `Err`
branch only prints the CPU fault and returns. This particular unsupported
instruction takes that branch, so the run has no complete fault-time SX/CPU
register or raw-VRAM dump. Share one capture routine between both stop paths,
recording the original fault PC/instruction, CPU registers, SX registers,
raw VRAM, frame and pending serial output. The prior review's
`local/review-2026-11-13-sx-std/src/main.rs` already captures both stop paths.
Keep normal demand-page and window traps distinct from graphics bus faults.

Independently reran all 41 focused tracked tests: `mouse`, `sx`, `sx_loads`,
`packed_outline`, `queue_pair`, and `outline_followups`; all pass. All eight
new isolated tests fail as described (seven at the missing funnel, one at
the derived packed alignment expectation). They are kept under ignored
`local/`; no failing tracked tests or production edits were added.

The complete core suite also passes independently: **475 tests**, with
loopback sockets enabled for the existing UDP/DNS/serial tests. The first
sandboxed attempt stopped on socket-bind `PermissionDenied`, not a device
regression. Formatting and all four built-in demos pass.

The required Clippy command does **not** pass on this tree using the documented
Xiri Rust 1.101.0-nightly toolchain: `clippy::collapsible_match` at
`src/devices/sx.rs:189`, the nested CLUT `table_index < 256` check. Resolve that
small match-guard cleanup while preserving the exhausted-stream no-op behavior,
then rerun `cargo clippy --locked --offline --all-targets -- -D warnings`.
The warning is included in `clippy.log`; production source hashes remained
unchanged throughout this review. Heliacal's reported checks/rebuild were not
repeated here, and interactive frontend acceptance remains outstanding.

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-funnel/Cargo.toml \
  --no-fail-fast
```

Saved evidence includes the supplied serial/trace/harness, before/held/released
PPMs, geometry and pixel-difference metrics, original DDX excerpt, test logs
and production source hashes. No new Solaris boot was necessary for this
review, and no disk image was modified.

**Not ready to merge the desktop milestone.** Resolve the funnel and packed
copy constraints, then require real geometry changes and correct redraw for
small moves, resize/uncover and iconify/restore. Validate Heliacal keyboard,
mouse capture/release and window interaction. Finally run the required checks
and SS-5/Axil boot regressions on the proposed commit. Completing every SX
opcode is not a prerequisite, but observed desktop operations must succeed
without guest faults or rendering corruption.

## Applied implementation and accepted move

The funnel shift group now implements operations 6 and 7. Source vectors are
snapshotted with the extra adjacent word before any destination writes, so
overlapping in-place and scalar-shift cases preserve original inputs. Shift
zero copies the source vector; shifts of 32 or more are rejected rather than
inventing wrap behavior. Register and destination ranges are validated before
mutation, preserving the paired-IQ rollback contract.

Packed operation-1 stores now select source lanes from the destination byte
alignment, skipping the funnel prefix lanes. Operations 5 and 9 keep their
previous lane mapping until evidence covers them. The six funnel fixtures and
both load/funnel/store copy fixtures are promoted into tracked
`tests/funnel.rs` and `tests/packed_copy.rs`.

A calibrated guest run on `/tmp/solaris26-sx2.raw` accepted the actual
geometries rather than pointer-only image differences:

```text
before    TERM-A +105+125
large     TERM-A +125+125  (20 pixels)
move1     TERM-A +126+125  (1 pixel)
move2     TERM-A +128+125  (2 pixels)
move3     TERM-A +131+125  (3 pixels)
```

The larger gesture includes two paced held motions. Both hold and release
completed without an SX fault. Captures, serial transcript and trace are in
`accepted-run3/`.

Validation after the change: **483 core tests**, formatting, Clippy with
`-D warnings`, all four demos, and Heliacal's 22 tests pass. Heliacal release
was rebuilt. Resize/uncover/iconify and interactive Heliacal input remain
unaccepted, and the SS-5/Axil boot regressions were not rerun for this slice.

## Independent rendering follow-up

The accepted-run3 geometry is confirmed, and all 483 tests plus formatting,
Clippy and demos pass independently. Its move2/move3 captures nevertheless
corrupt text, borders and the title bar. Packed loads still discard address
alignment and reserve only eight registers for an unaligned 32-byte transfer;
the new stores require address-relative lanes and nine source registers.
Captured outline inversion/erase sequences reproduce the corruption without
a funnel. See the [unaligned-load review](ss20-sx-unaligned-load-review.md)
for six isolated regressions and pixel comparisons. Acceptance currently
covers geometry changes, not correct rendering or readiness to merge.
