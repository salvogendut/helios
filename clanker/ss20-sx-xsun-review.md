# Native CG14/Xsun: first SX commands and harness delays

Reviewed 2026-11-13 at `7c958b4`, including the other agent's uncommitted
`src/devices/sx.rs` and `tests/sx.rs` changes. No production behavior or guest
disk was changed by this review. Existing five SX tests pass. A separate
review crate reconstructs the captured instruction encodings and exposes
five failures without booting Solaris.

Evidence and runnable test/benchmark crate:
`local/review-2026-11-13-sx/`. Run from the repository root:

```sh
cargo test --locked --offline --manifest-path local/review-2026-11-13-sx/Cargo.toml
cargo run --locked --offline --release --manifest-path local/review-2026-11-13-sx/Cargo.toml
```

The manifest points to this local Helios checkout. Five behavioral tests are
expected to fail until the corresponding production paths are implemented;
the encoding check passes. These are review reproducers, outside the normal
core suite. `reviewed-source.txt` records hashes of the reviewed files.

## The first memory command is already recorded

`/tmp/sx-xsun13.err:1958` and the preceding runs contain:

```text
SX REG 0x1010 <- 0xffffffff
SX REG 0x1324 <- 0x00000000
SX REG 0x1020 <- 0xb1e00509
CG14 FB 0x000000 <- 0x8fcec500
CG14 FB 0x000080 <- 0x8fcec500
CG14 FB 0x000100 <- 0x8fcec500
...
SX REG 0x1014 <- 0x000000cc
SX REG 0x1010 <- 0xffffffff
SX REG 0x1324 <- 0xfafafafa
SX REG 0x1020 <- 0xb1e00509
```

The gray queued value is **0xfafafafa**, including the high byte. The current
logger drops the high physical address and labels SX memory writes as ordinary
framebuffer writes, so the existing text cannot independently prove which
aperture received each write. Preserve full physical addresses in future
records. Nevertheless, the word encoding and 128-byte spacing identify a
specific memory-command path to implement and test now:

| Word | Decode from NetBSD's SX instruction definitions |
| --- | --- |
| `0xb1e00509` | `SX_ORS(0, 9, 10, 1)`: vector/scalar OR, A=R0, B=R9, destination R10, encoded count 1 (two results) |
| `0x8fcec500` | Memory operation, encoded count 31 (32 elements), operation 9 = store-select plus plane mask, type `SX_LONG`, source register 10, displacement zero |

With R0/R1 zero and both selected source registers holding the same color,
the minimal expected result is 32 consecutive 32-bit color words (128 bytes).
The current model instead puts the instruction word `0x8fcec500` into VRAM.
Waiting for another full Solaris boot to discover a first fill is unnecessary.

## Implement the path in dependency order

1. Share appropriate state between SX supervisor and user register pages.
   Xsun writes the user page (`+0x1010`, `+0x1020`, `+0x1324`); the current
   byte-array implementation treats it as unrelated storage. Enforce user-page
   restrictions for protected registers rather than blindly mirroring all bits.
2. Implement the common 128-register execution bank, direct versus queued
   access, R0 initialization, reset, GO, synchronization and observable status.
   A functional synchronous executor is sufficient for the first path; do not
   claim execution complete while silently discarding commands. NetBSD identifies
   MT as `0x4000` and busy as `0x8000`; its header explicitly warns that some
   older manual bit assignments disagree with the hardware/SunOS definitions.
3. Execute the observed ORS and masked store-select/long operations. Writes to
   `0x800000000 + target PA` submit **SX memory instructions**, not ordinary
   pixel data. Split this dispatch before ordinary CG14 raw/BGR/X16/X32
   aperture decoding. Apply the destination aperture's semantics to the
   resulting memory operation. Later RAM targets need normal bus access too.
4. Capture complete PA, value, width/mask, operation/read result and ordering
   into a buffered trace; keep necessary starting device state or initialization
   writes. The current 80-framebuffer-write cap is consumed by early commands.
   Replay against the real device/bus API as in the previous CGsix investigation.
   A replay validates only the captured path; boot again to discover the next
   path once this one works. Report unsupported instructions before mutation.

The current `sx_vram_alias_is_shared_with_the_vsimm_window` regression asserts
plain data-write aliasing and should be replaced with command semantics tests.
It currently protects the wrong behavior.

The isolated tests cover page coherence, queued R9 visibility, ORS execution
and the 32-word memory command. The store test seeds both possible source
registers equally so it does not depend on an assumed selector-bit ordering.
The ALU test uses zero R0 and R1 to isolate it from R0 vector special cases.

There is also a separate BGR narrow-write bug: writing byte `0x44` at
`VSIMM_VRAM_BASE + 0x01000001` leaves raw `0xaa112233` unchanged, although
the B byte should change, producing `0xaa442233`. `write_vram` shifts the
byte/mask into its word lane but then reads `raw[index]`/`mask[index]` starting
at zero instead of the selected lane. The full-word BGR regression misses this.
The five passing tests do not establish full XLUT/blending or mode support.

## Remove the experiment-loop overhead first

The review benchmark uses a two-million-step synthetic CPU loop and a 3 KB
transcript. It is an overhead experiment, **not a measured Solaris boot gain**:

| Polling style | Host seconds | Million steps/second |
| --- | ---: | ---: |
| Core stepping only | 0.0670 | 29.87 |
| Drain output and scan entire transcript each step | 2.5734 | 0.78 |
| Poll every 4096 steps; scan only new output plus overlap | 0.0690 | 28.98 |

`/tmp/probe_sx_xsun.rs:23-29` searches the entire accumulated transcript for
`login:` on every instruction until login appears. The result above makes
this a credible contributor to the reported twenty-minute checkpoints.
Run emulation in bounded batches; match prompts only when new bytes arrive,
keeping at most `pattern_length - 1` bytes of overlap. Bound each wait and
propagate every `Machine::step` error with PC and step count.

Other concrete harness issues:

- The probe waits two million instructions between each input character. A
  100-character command consumes roughly 200 million extra instructions before
  considering its execution. Use bounded pacing and receiver backpressure,
  checking guest echo when needed; avoid flooding the Solaris tty queues.
- `/tmp/sx_daemon.rs:6` accumulates `n += 2_000_000` and then calls `pump(n)`
  after **every** accepted byte. For 100 bytes that is 10.1 billion steps,
  rather than 200 million. Use a per-byte interval or absolute deadline.
- That same sender sets its prompt-search mark **after** pumping the final
  carriage return. A quick command may already have printed its prompt, making
  the following wait hang. Establish the response boundary before sending the
  command, and use a stateful parser or a unique completion marker. Avoid a
  marker match on the shell's echoed command itself.
- Single-user probes must transition out of the maintenance-password state
  after sending Return. Searching old transcript contents every step can
  repeatedly inject the same response.
- The probe configurations omit the keyboard and therefore take the full
  no-keyboard POST path. Match the intended Heliacal keyboard/NVRAM setup for
  representative comparisons; do not change CPU timing to speed up these tests.

Keep one guest alive for shell/setup experiments. A general machine snapshot
feature is not required to solve this trace problem. Device replay allows
editing/rebuilding SX between tests without a full OS boot.

## Control dtlogin through the guest's service interface

As root on the disposable development disk, from the serial console:

```sh
/usr/dt/bin/dtconfig -d
/usr/dt/bin/dtconfig -kill
ps -ef | grep '[d]tlogin'
ps -ef | grep '[X]sun'
```

`-d` disables future automatic startup; `-kill` stops the current display
manager and its sessions. Wait for both processes to exit before starting a
single manual Xsun. `/etc/init.d/dtlogin stop` is the equivalent fallback
verified in this Solaris 2.6 disk's script. Repair/verify the framebuffer link,
or use the verified `/dev/fbs/cgfourteen0` node, before the manual launch.
Only remove a leftover X lock after establishing there is no owning server.

The last probe hard-codes guessed PIDs and misses the actual dtlogin/Xsun
pair (246/273 in run 13). Renaming S99dtlogin after startup does not stop that
pair. Reading `/tmp/solaris26-sx2.raw` without modifying it showed K99dtlogin
already present and no dtlogin respawn entry in `/etc/inittab`; restarting the
whole machine for each kill attempt is unnecessary. No guest commands were
executed as part of this review.

## Sources and limits

- Local NetBSD `sys/arch/sparc/dev/sxreg.h` revision 1.22 (2024-05-12): register
  aliases, memory-instruction aperture, operation/type/count encodings.
  [Upstream source](https://github.com/NetBSD/src/blob/trunk/sys/arch/sparc/dev/sxreg.h).
- Local NetBSD `cgfourteen.c` revision 1.102 (2026-10-13), especially the
  `sxm` macro and `cg14_rectfill`: ASI 0x28 command writes and count-minus-one.
- [Oracle CDE administration guide, Starting the Login Server](https://docs.oracle.com/cd/E19455-01/806-1361/6jaldfjjf/index.html):
  `dtconfig -d` and `dtconfig -kill`. Also checked the actual disk's
  `/etc/init.d/dtlogin`, `/etc/rc2.d` and `/etc/inittab` read-only.
- The five isolated behavioral failures were reproduced in milliseconds;
  the separate existing five SX tests passed. No new Solaris boot, full Xsun
  desktop validation, core-wide suite, or Heliacal rebuild was performed.

## Applied implementation

The review crate now passes all six behavioral tests. The production SX model
shares user/supervisor register pages, maintains the 128-register bank and
queued writes, executes the observed `SX_ORS` and masked store-select
long/byte/short commands, and rejects unknown instruction/data types before
mutation. The BGR narrow-store lane bug is fixed. Nine local SX regressions
and the six review regressions pass; Clippy and the four built-in demos pass.

The first real fill was replayed through the fast, incremental-polling harness
after `/usr/dt/bin/dtconfig -d` and `-kill`. A manual Xsun on `/dev/fb`
programmed 32-bit mode, executed the `0x8fcec500` store command, and showed the
root window plus cursor instead of OBP pixels. Starting `xterm` then reached
the next unsupported instruction, `0xe4252020`; that remains the next trace
point. The manual Xsun root window and serial transcript are local evidence,
not an OS-support claim. Heliacal desktop interaction is still unverified.

## Execution follow-up: actual shift command and hidden corruption

The next review reran the six original review tests and nine production SX
tests successfully. It then reproduced the Xsun/xterm stop independently using
an isolated core source copy and a disposable clone of `/tmp/solaris26-sx2.raw`.
The source disk and production code were not modified. Evidence, the probe,
and five additional isolated regressions are in
`local/review-2026-11-13-sx-execution/`.

**Correction to the preceding report:** `0xe4252020` is the SPARC CPU's
instruction word, not the rejected SX command. At PC `0xee6715e8`, it decodes
as `st %l2, [%l4 + 0x20]`. The translated physical write is `0xf80001020`,
the SX user-page instruction register. The independent probe captured:

```text
SPARC instruction = 0xe4252020
%l2 (store value) = 0xc0a10106
%l4 = 0xef7a0000
physical address = 0xf80001020
```

The actual SX word is **`SX_SLLI(4, 6, 2, 0)`** using the NetBSD `sxreg.h`
encoding: logical left shift, source R4, immediate shift 6, destination R2,
encoded count 0 (one result). R2 is the store-select mask register. This is
the next instruction to implement after correcting the existing execution
semantics. Log the incoming MMIO value before dispatch, or include it in the
failure diagnostic: the current trace prints register writes only after a
successful operation and therefore hides the failed SX word.

The previous small tests exercised all-zero/all-equal data and only the
intended results. They missed corruption outside those results. Five new
regressions fail on the reviewed production code:

1. **ALU count includes opcode bits.** Both 0x9 and 0xb groups compute
   `usize::from((instruction >> 24) as u8) + 1`. For `0xb1e00509`, this is
   178, rather than 2. Repeated `& 127` indexing wraps through the entire
   register bank: even R0 becomes `0xfafafafa`. Decode the four-bit count field
   (`((instruction >> 24) & 0xf) + 1`) and separately validate architectural
   register ranges. Test that every non-destination register is preserved.
2. **Plane mask destroys unselected destination bits.** With destination
   `0xaabbccdd`, source `0x11223344`, and mask `0x00ff00ff`, a masked long store
   produces `0x00220044` instead of `0xaa22cc44`. Merge the selected source
   bits with the old destination; zeroes in a write mask preserve old bits.
3. **Memory-command displacement and width are lost.** The actual trace also
   contains `0x81480505`: three masked selected byte stores at displacement 5.
   The executor ignores the displacement, then passes `u32::MAX` as the
   `write_vram` width/mask even for byte and halfword operations. The test
   requires bytes 5, 6 and 7 to change and their neighbors to remain intact;
   the current implementation starts changing byte 0. Decode the memory
   instruction's offset/channel/type and pass the actual access width.
4. **Store-select ignores R2.** The executor chooses source registers with
   `index & 1`, unconditionally alternating colors. A uniform selector mask
   must select a uniform source. The test sets every R2 bit to one and the
   two source colors differently, checking uniform output without assuming
   selector polarity or bit order. This matters for the next SLLI, which
   computes R2 for subsequent drawing.
5. **Unsupported target can leave a partial write.** A three-word command
   crossing the end of the implemented aperture changes VRAM before returning
   `Unsupported("SX command target")`. Validate every target and supported
   operation before committing register/VRAM changes. A retryable host stop
   must preserve state as required by HACKING.md.

Run the additional tests without booting Solaris:

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-execution/Cargo.toml \
  --test execution
```

Recommended order: correct and test the current executor, add the confirmed
SLLI operation with distinct source/destination/neighbor test values, then
repeat the same manual Xsun/xterm scenario. A complete root fill is useful
progress, but the all-equal source data hides selector and register-corruption
bugs. Preserve original firmware and source disk images throughout. No full
desktop, Heliacal interaction or clean guest shutdown was established by the
follow-up probe, and no full core suite was rerun for this review-only change.

## Execution corrections applied

The five execution regressions and the recorded SLLI are now implemented and
tested. ALU counts use the four-bit count field, memory commands use the
five-bit count field, plane masks merge selected bits with the old destination,
byte/halfword stores honor displacement and width, store-select uses R2
bitwise, and all command targets are validated before any VRAM write. The
captured `0xc0a10106` now executes as `R2 = R4 << 6` without changing R1/R3/R5.

The repeated manual Xsun/xterm run passed SLLI and the following ORV/ORS
commands; xterm then reached a new unsupported SX word, `0xe322859c` (top
group `0xe`, group op 1), at the SX user instruction register. The core still
reports that instruction explicitly and leaves state unchanged. This is the
next trace point; no graphical login, window interaction, or Heliacal UI
validation is claimed.

## PLOT follow-up and mixed store selection

The next review reran all five execution regressions and the ten production
SX tests: all passed. `/tmp/sx-fast6.err:45361` now logs the incoming command
correctly: **`0xe322859c` is an SX word**, unlike the previous CPU store word.
Its fields are group E, operation 1, A=10, B=28, D=11, encoded count 3.

This is the **PLOT/Bresenham offset-generation path**. NetBSD's `sxreg.h`
defines only gather/scatter in group E; it does not define this operation.
Use the original Sun description and Xsun's code as additional references:

- [Sun, Pixel Processing in a Memory Controller](https://www.sunhelp.org/history/pdf/smc.pdf),
  sections 4.1.8 and 4.2.5: PLOT generates offsets from a starting offset/error
  and major/minor offset/error increments. A subsequent indexed/array store
  writes the pixels. PLOT itself is register arithmetic, not a VRAM renderer.
- Original Solaris 2.6 `ddxSUNWcg14.so.1`, functions
  `sl_ddx_line_fill_8` (ELF address `0x13dc8`) and `sl_ddx_line_fill_32`
  (`0x13c44`). Both construct opcode base `0xe022859c` and combine it with
  their count field. This ties group E/op 1 to the documented PLOT path.
- [SunVTS sxtest documentation](https://docs.oracle.com/cd/E19620-01/805-4443/6j477v0ik/index.html)
  also names `spam_plot`/`spam_delt` among its miscellaneous instruction tests.

The original library was read from an already extracted local Solaris
package. It and the downloaded reference files remain under ignored
`local/review-2026-11-13-sx-plot/`, together with hashes, disassembly and a
trace excerpt. No production code or disk was changed, and no guest was
booted during this follow-up.

### Captured fixture and inferred post-state

The writes immediately before the stop initialize:

```text
R10 = 0x0001cfe7   starting pixel offset (x=103,y=103 at stride 1152)
R11 = -5           starting error
R28 = 1            major offset increment
R29 = 0            major error increment
R30 = 0x481        minor/diagonal offset increment (1152 + 1)
R31 = -10          minor/diagonal error increment
IQ  = 0xe322859c
```

The library seeds R10 with the first point, then requests the remaining
points beginning at R11. Its initial instruction uses `(length - 2)` in
the encoded count field. Thus this fixture represents five total points:
the existing seed plus four newly generated offsets.

For this negative-error horizontal case, the independently inferred result is
R11..R14 = `0x1cfe8`, `0x1cfe9`, `0x1cfea`, `0x1cfeb`, with final error `-5`
in R15. R10 and unrelated registers remain intact. Snapshot all input state
before writing: the destination overlaps the input error register R11.

The continuation loop supports the final-error placement: after generating
the first 16-point batch it submits `0xef26451c` (A=25, B=28, D=10, count=15),
using the last offset in R25 and error in R26 to generate the next batch.
This is an inference from Sun's original driver plus the architecture paper,
not a hardware-observed register dump. Exact error==0 tie behavior and other
octants still need independent validation. Avoid encoding uncertain cases
from a guessed implementation into their own tests.

A proposed horizontal fixture is saved in `tests/plot.rs`. It currently
stops at the expected unsupported PLOT operation.

### Implement the indexed store that follows PLOT

The line routine's next operation stores colors through the generated offset
vector. For five 8-bit pixels, the original routine constructs `0x8248100a`:
the data source starts at R32 and the offset vector starts at R10. The raw
VRAM byte offsets should be the five positions described above, relative to
the supplied framebuffer base.

The current executor always treats the low field as a literal displacement
and walks consecutive addresses. That cannot implement the offset-vector
store. Confirm the direct/indexed mode encoding and all address-generation
rules before accepting this traffic; do not silently interpret its R10 field
as a displacement of ten bytes. Test the two operations together, including
an oblique line whose generated offsets are not consecutive, while preserving
destination validation and fault atomicity.

### Store-select is still incomplete for a mixed mask

The previous uniform-selector regression passes with the new implementation,
but it cannot distinguish whole-element selection from a bitwise blend.
The new expression

```text
(Rsource & R2) | (Rsource_plus_1 & ~R2)
```

mixes the colors and repeats that mixture for every element. The mask must
select source elements across the output vector. Sun's section 4.1.10
distinguishes element selection from plane masking within an element;
NetBSD's `cg14_rectfill`/`cg14_putchar` use the source-pair form for fills and
monochrome expansion.

A regression with R2=`0x55555555`, colors `0x11111111` and `0x22222222`,
and four long stores currently writes `0x33333333` four times. Each output
should instead equal one of the two original colors, alternating with the
selector bits. The regression intentionally does not assume bit order or
selector polarity; those must be checked against the source/trace separately.
This matters for the SLLI and R2 preparation already observed during text
drawing, not just future device completeness.

Run the new isolated checks:

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-plot/Cargo.toml --test plot
```

One fails on the mixed-color output; the proposed PLOT fixture fails at the
currently unsupported command. Recommended next slice: fix mixed store
selection, implement PLOT and its indexed-store path with independently
derived register/VRAM expectations, then repeat xterm and validate text and
line rendering in Heliacal. No full desktop claim or additional core-wide
validation was made by this review.

## Mixed selection, PLOT and indexed stores applied

Mixed store-select now chooses a whole source element from the R2 selector
bit for each output element; the 0x55555555 regression passes. PLOT/Bresenham
generation now snapshots R10/R11/R28..R31 before writing, emits the generated
offsets through R11.. and writes the final error after them; the proposed
horizontal fixture passes. The indexed byte store path recognizes the captured
source-R32/offset-R10 form, validates every effective target before writing,
and applies the generated offsets instead of treating R10 as a literal
displacement. A synthetic three-pixel array-store regression passes.

The repeated Xsun/xterm run no longer stops at PLOT or the indexed byte store
and reaches the line loop's subsequent PLOT batches. The resulting Xroot
window still appears as the magenta fill with no visible xterm client, so
line/text presentation is not yet accepted. The direct/indexed mode encoding
is inferred from the captured source-R32 form and Sun's array-store
description; it still needs independent hardware confirmation. No graphical
login, terminal scrolling, window interaction, or Heliacal UI validation is
claimed.

## Flat-display follow-up: retrace, color tables and scanout

The follow-up reviewed `sx-fast8` against SX source hash
`8925151483a35e21a9ed4f84a7fbc661d37016ec2f53cf5eebca86e3304ca7dc`.
The two previous PLOT review tests and all eleven current production SX tests
pass. Six new independent display/execution checks fail. No production code
was changed, no guest boot was run, and no source disk was opened for writes.
New artifacts live under ignored `local/review-2026-11-13-sx-scanout/`.

The next milestone needs the CG14 display pipeline as well as SX execution:
**vertical retrace -> guest interrupt handler -> color-table programming ->
pixel routing**. Continuing opcode coverage alone cannot make this run display
correctly.

### Solaris is waiting for the missing retrace path to update its tables

Original Sun sources make the dependency explicit:

- [Solaris 2.6 cgfourteen.c](https://github.com/Arquivotheca/Solaris-2.6/blob/2.6/os_net/src_ws/usr/src/uts/sun/io/cgfourteen.c):
  `MDI_SET_XLUT`/`MDI_SET_CLUT` update shadow tables and enable vertical
  interrupts. `mdi_poll()` requires `MDI_MSR_INTPEND`, and `mdi_intr()` pushes
  the pending XLUT/CLUT/gamma/cursor changes when `MDI_MSR_VINT` is set.
  See downloaded source lines 1422, 3690, 3730, 4896 and 4925.
- [Sun cg14reg.h](https://github.com/Arquivotheca/Solaris-2.6/blob/2.6/os_net/src_ws/usr/src/uts/sun/sys/cg14reg.h):
  MCR bit `0x80` enables interrupts; MSR at control offset 4 reports
  `INTPEND=0x20` and `VINT=0x10`.
- `mdi_intr()` at downloaded source line 5104 explains the acknowledgement:
  a write to MSR clears vertical pending; MSR is read-only status, not a byte
  of arbitrary guest-writable storage.
- [NetBSD intreg.h](https://github.com/NetBSD/src/blob/trunk/sys/arch/sparc/sparc/intreg.h)
  defines `SINTR_VI=0x00100000`, and the
  [pinned QEMU interrupt controller](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/intc/slavio_intctl.c)
  maps source bit 20 to PIL 8. This is the onboard video input, distinct from
  the CGsix SBus interrupt route.

Current `Sx` has no retrace clock or IRQ state. `SystemBus::advance_devices_ns`,
`advance_seconds` and `interrupt_levels` service CGsix but never SX/CG14;
`Source` and `hardware_levels()` omit video bit 20. MSR is ordinary storage.
The latest trace ends with MCR `0xf0`, yet there is no modeled source that can
perform the requested retrace service. Implement timing from emulated device
time, independently of rendering/frontend calls, with pending, mask, enable,
acknowledge and reset behavior tested at the bus level.

### The handler uses the common autoincrement port

Adding the interrupt alone will expose another missing path. In `mdi_intr()`:

| Operation | Select starting table/index | Stream data |
| --- | --- | --- |
| XLUT | byte write at `0x3800 + index` | repeated byte writes at `0xf000` |
| CLUT1 | word write at `0x4800 + 4*index` | repeated word writes at `0xf000` |
| CLUT2 | word write at `0x5800 + 4*index` | repeated word writes at `0xf000` |

These are offsets within CG14 control space. Preserve the table selection and
increment its index with each stream item; expose the resulting entries through
normal XLUT/CLUT reads. CLUT words are `AABBGGRR`, including the alpha byte.
The current device writes unrelated bytes in its `control` array instead.
The XLUT and CLUT2 stream regressions both leave the intended entries zero.
Extend MMIO logging to include the selection apertures, all CLUTs and `0xf000`;
the current trace filter excludes these writes.

### X is a pixel-routing selector, not always the visible color index

[Sun cg14io.h](https://github.com/Arquivotheca/Solaris-2.6/blob/2.6/os_net/src_ws/usr/src/uts/sun/sys/cg14io.h)
defines the XLUT source/table choices. For example, `0x44` selects left CLUT1
with its index from B; `0x84` selects left CLUT2 from B. The X byte selects an
XLUT entry; that entry determines the source components, color tables and blend
paths. The right path supplies blend alpha. The current `pixels()` special case
for XLUT value `0x40` uses CLUT1[X], with all other values treated as direct RGB.
That does not implement the documented routing.

A two-pixel fixture uses the same X=`0xfa`, different B indices 1 and 2, XLUT
`0x44`, and deliberately unrelated G/R bytes. Expected colors come from CLUT1
entries 1 and 2. Current scanout instead emits the unrelated raw BGR data.
Implement the configured routing; do not force the B plane globally or replace
the guest's table values to make one screenshot appear.

The offline replay of `/tmp/sx-fast8.err` accepts all **45,580 logged writes**:

```text
X plane: 0xfa at all 1,036,800 visible pixels
B plane: 0 at 1,036,469 pixels; 1 at 331 pixels
G/R planes: zero at every visible pixel
```

The B-plane diagnostic image contains a small terminal glyph/cursor shape near
the requested `(100,100)` location. The unchanged X plane explains why the
current X-indexed output hides these writes. The supplied framebuffer dump has
1,036,740 magenta pixels and 60 black cursor pixels. This replay is evidence
about logged drawing operations, **not a full machine snapshot**: the trace
omits most direct VRAM writes and CLUT/other control writes. Its diagnostic
palette is the replay's default CLUT, not the guest's complete color state.
Capture raw VRAM plus XLUT/CLUT state in the next guest run to confirm the
complete result.

### Two indexed-store problems remain

1. `array = width == 1 && source >= 32` also misclassifies legal direct stores.
   NetBSD `cg14_bitblt` uses ordinary `SX_STB` with R32. A fixture for
   `SX_STB(32, 2, 5)` (`0x81001005`) should place three source bytes at offsets
   5, 6 and 7. It instead repeatedly writes offset zero and leaves only `0x33`.
   This disproves the broad source-register heuristic; the exact array format
   still needs decoding independently.
2. The array branch bypasses store-select and consumes R32, R33, R34, ... .
   The actual constant-color line routine initializes only R32/R33 using
   `0xb1e01009`. Combining the captured PLOT fixture with `0x8248100a` now
   writes **`[1, 1, 0, 0, 0]`**, where all five points should have color 1.
   The synthetic array test with three independently initialized colors does
   not validate the actual constant-color command. Decode addressing and data
   selection separately. The combined fixture is inferred from the original
   driver, not measured on hardware.

### Reproduction and next run

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-scanout/Cargo.toml --test display
cargo run --locked --offline \
  --manifest-path local/review-2026-11-13-sx-scanout/Cargo.toml -- \
  /tmp/sx-fast8.err local/review-2026-11-13-sx-scanout
```

The six failures are retrace status/IRQ, XLUT stream, CLUT stream, XLUT B-source
scanout, direct R32 store, and PLOT followed by constant-color store. Sources,
hashes, tests, replayed VRAM/control and per-plane diagnostic images are saved
locally. The new tests do not establish full PLOT timing/ISA or blend rounding.

Implement retrace/interrupt delivery and table streaming together, then validate
XLUT/CLUT routing. Keep the two store regressions in the same acceptance set.
Trace the guest's handler updating the tables and acknowledging MSR, compare
raw planes to rendered pixels, then repeat xterm and Heliacal interaction.
Fix the harness process check too: this Solaris `/usr/bin/grep` rejects `-E`,
as the transcript reports. Use `ps -ef | egrep '[X]sun|[x]term'`. No window
manager is started by this harness, so title bars are a later, separate check.

## Retrace, tables and routing applied

The CG14 now advances a vertical retrace clock from emulated device time,
latches MSR INTPEND/VINT while MCR interrupt enable is set, clears pending on
an MSR write, and wires source bit 20 to PIL 8. XLUT/CLUT selection and the
common `0xf000` autoincrement stream are implemented, including CLUT alpha
bytes. Scanout uses the X XLUT entry to choose the left color table and B/G/R
index, so the captured X=`0xfa`, B-indexed pixels now render through CLUT1.

Direct `SX_STB` with R32 is no longer classified as indexed; indexed byte
stores are limited to the captured select-store form and use the R10 offset
vector with R2 whole-element color selection. All six scanout regressions,
the earlier SX/PLOT regressions, and the production SX tests pass.

The manual Xsun/xterm repeat now shows a white terminal rectangle and cursor
instead of a uniform magenta field. That is the first visible client
milestone, not an accepted desktop: text, scrolling, wrapping, interaction,
window-manager decorations and Heliacal validation remain unverified.

## Visible-client follow-up: selector order and polarity hide the prompt

The six previous display regressions pass against the new display-update
implementation. `/tmp/sx-fast9.png` shows the terminal rectangle and root
stipple, but its prompt is a solid black block. The latest trace supplies the
selector bit order and polarity that the earlier uniform/alternating-mask
tests intentionally left open.

### Exact text failure and an isolated replay

`/tmp/sx-fast9.err:46579` initializes R8=`0` and R9=`0x01010101`, then draws
the two-character `# ` prompt through the B32 aperture using `0x85c80407`
(12 selected, masked byte stores from the R8/R9 pair, displacement 7).
The 13 row masks include `0x50000000` and `0xf8000000`; their significant bits
are at the **most-significant** end of R2. The glyph is:

```text
............
............
............
.#.#........
.#.#........
#####.......
.#.#........
#####.......
.#.#........
.#.#........
............
............
............
```

[NetBSD cg14_putchar](https://github.com/NetBSD/src/blob/trunk/sys/arch/sparc/dev/cgfourteen.c)
independently establishes the convention: R8 is background, R9 foreground;
8-bit glyph rows are shifted left by 24 (16-bit rows by 16), and `SX_STBS`
starts from R8. The inspected local revision is 1.102, 2026-10-13.

The current implementation reads R2 from bit 0 upward and picks the second
color when that bit is zero. Thus these high-bit masks all produce a solid
foreground block. For the observed selected stores the corrected choice is:

```rust
let selector = ((self.regs[2] >> (31 - index)) & 1) as usize;
let selected = self.regs[(source + selector) & 127];
```

Apply the convention consistently to selected direct and indexed stores.
The register pair used for the constant-color PLOT fixture contains identical
colors, so it could not expose this error. Distinguish element selection from
plane masking as before.

An isolated copy of `Sx` changes only these two selector expressions in each
branch. Replaying the same 46,673 logged writes with it makes the `#` prompt
and outlined terminal cursor visible. The unmodified replay matches the guest
dump **exactly within the terminal interior** `(101,101)..(585,417)`; the
selector experiment changes 180 pixels within the 18x13 prompt region.
The full replay does not reproduce all root-window pixels because the trace
omits direct VRAM writes after the first 80. This is a focused replay result,
not a fresh guest boot or complete desktop validation.

Local artifacts under `local/review-2026-11-13-sx-text/` include:

- `glyph-comparison.svg`: enlarged diagnostic chart of the prompt before/after.
- `baseline/` and `selector-corrected/`: raw replayed VRAM/control and scanout.
- `src/sx-experiment.rs` and `selector-experiment.diff`: isolated selector change.
- `tests/text.rs`: independent byte-glyph and long-element selector expectations,
  plus the two remaining display-update edge cases below.
- `sha256.txt`, `trace-excerpts.txt`, and test/replay logs.

### Two display-update edge cases also fail

1. **CLUT start selection stops after index 63.** Each CLUT autoincrement
   selector has 256 **word** entries, so its range is 0x400 bytes. The current
   `base..base+0x100` recognizes only 64 entries. CLUT1 starts must cover
   `0x4800..0x4bff`, CLUT2 `0x5800..0x5bff`. A fixture checking indices 63,
   64 and 255 fails at 64. Initial full-table streams from zero pass, which
   explains why the existing test and current screenshot did not detect it.
   The original Sun `mdi_clut_address` defines `c_clut_inc[256]` as words.
2. **MCR interrupt disable does not gate an already-pending IRQ.** After a
   retrace, changing MCR from `0xf0` to `0x70` leaves PIL 8 asserted because
   `irq()` returns `retrace_pending` unconditionally. Gate delivery with MCR
   interrupt enable while preserving the separately defined pending/ack state.
   Sun's driver temporarily disables this interrupt while updating its shadow
   tables. The regression checks that delivery drops; it does not prescribe
   unverified latch behavior on disable or re-enable.

The six earlier display tests remain green. All four new tests fail against
production; the isolated selector experiment makes the two selector tests
pass while the CLUT-range and IRQ-gating tests continue to fail. Production
source remains untouched by this review.

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-text/Cargo.toml --test text
# Isolated experiment, not the production core: two pass, two fail.
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-text/Cargo.toml \
  --features selector-corrected --test text
```

### Acceptance sequence after these corrections

Promote the display/text fixtures from ignored `local/` into the tracked
device regression suite before committing the milestone. Then run a terminal
that automatically prints a deterministic character grid, followed by numbered
lines long enough to trigger wrapping and scrolling. This separates rendering
from keyboard delivery and provides expected output for comparison. Capture
the next unsupported instruction or first incorrect copy when it occurs.

After text and scrolling work, start a window manager explicitly and validate
move/resize/expose redraw. The current manual harness starts only Xsun and
xterm; missing title bars are expected in that setup. Rebuild Heliacal against
the corrected core and test focus, keyboard input, cursor movement and redraw
there. The `sx-fast9` transcript still contains the old rejected `grep -E`
command although the harness source now says `egrep`; ensure the harness
binary is rebuilt as well.

## Selector order, full CLUT ranges and IRQ gating applied

Selected direct and indexed stores now take R2 from bit 31 downward: a clear
bit selects Rsource and a set bit selects Rsource+1. CLUT autoincrement
selectors cover all 256 word entries (`0x400` bytes, indices 0–255), and video
IRQ delivery is gated by MCR interrupt enable while pending/MSR acknowledgement
state remains separate. The four text/display regressions are now tracked in
`tests/sx.rs`; all four pass, along with the prior review suites.

A deterministic manual Xsun run with `olwm` and an automatically wrapping
xterm was attempted after rebuilding the harness. It reaches the next
unsupported SX form, `0x82afc501` (type `SX_PACKED`, operation 5), while
drawing terminal content. The visible white terminal rectangle test remains
the last accepted screen milestone; deterministic text, scrolling and window
manager interaction are still open. The replayed selector comparison is under
`local/review-2026-11-13-sx-text/glyph-comparison.svg`.

## Packed conditional glyph-store review

The three corrections above are present and all 15 tracked `tests/sx.rs`
tests pass independently. The latest `/tmp/sx-fast10.ppm` also shows readable
`LINE-...` and `WRAP-...` terminal text, an outlined terminal cursor, and a
partly drawn olwm frame. This advances the visual evidence beyond a white
rectangle; scrolling and interactive redraw remain unverified.

### The stop is a transparent glyph, before the scroll-copy test

The fault PC `0xee65d264` corresponds to ELF address `0x1d264` in the original
Solaris `ddxSUNWcg14.so.1`, inside `sl_rect_glyph_8`. Its SPARC instruction is
`st %o0, [%o5]`; the SX command delivered by that store is `0x82afc501`.
The effective destination is in the title-bar region of the screenshot.
This identifies a glyph-rendering failure; the trace does not yet demonstrate
a scrolling failure.

| Field | Captured value | Meaning |
| --- | --- | --- |
| Count, bits 27:23 | 5 | **Six bytes/pixels**, not six words |
| Operation, bits 22:19 | 5 | Conditional store (`4`) plus plane mask (`1`) |
| Data format, bits 18:14 | 31 | `SX_PACKED`, four byte pixels per source register |
| Source, bits 13:7 | 10 | Source begins at R10 |
| Displacement, bits 6:0 | 1 | Add one byte to the aligned target |
| R2 | `0x7c000000` | First six enables: `0,1,1,1,1,1` |
| Plane mask | `0xffffffff` | All destination bits enabled |
| Target PA in trace | `0xff41ebe0` | Effective address `0xff41ebe1`, B32 aperture |

The preceding `0xb8e00509` prepares source registers using color
`0x01010101`. For this six-byte case the relevant R10/R11 packed bytes are
all `0x01`. Replaying the 50,708 preceding logged writes confirms R10/R11
and R2 at the stop; `register-replay.txt` records the register dump. The
target maps to screen `(353,109)`. Preserve that first pixel;
write B byte `0x01` at `(354..358,109)`, preserving X/G/R and neighboring
pixels. Conditional enable and store-select are different operations: a
clear enable leaves the old destination intact; it does not select a
background source. Do not compress the source byte sequence around skipped
elements.

This decode has independent support:

- NetBSD `sxreg.h` revision 1.22 defines `SX_STORE_COND`, `SX_STORE_MASK`,
  `SX_STORE_SELECT` and `SX_PACKED` separately. The local checkout is under
  `../NetBSD-xiri/sys/arch/sparc/dev/`; see the
  [pinned source](https://cvsweb.netbsd.org/bsdweb.cgi/src/sys/arch/sparc/dev/sxreg.h?rev=1.22&content-type=text/x-cvsweb-markup).
- Sun's [Pixel Processing in a Memory Controller](https://www.sunhelp.org/history/pdf/smc.pdf),
  tables 2/3 and section 4.2.2, distinguishes conditional element enables,
  plane masks and packed storage, with four byte pixels per register.
- The original Solaris glyph routine constructs the count from **glyph width
  minus one**. Its wide-glyph loop emits `0x8fafc500` plus displacement,
  advances the destination by **32 bytes**, and advances the glyph-mask
  pointer by four bytes. This independently establishes that the packed
  count measures bytes. Relevant ELF addresses are `0x1d1e0..0x1d200` and
  `0x1d354..0x1d414`. The binary and disassembly remain ignored under
  `local/review-2026-11-13-sx-plot/`.

### Regressions and remaining alignment uncertainty

Three fixtures are in `local/review-2026-11-13-sx-packed/tests/packed.rs`:

1. The captured unaligned B32 glyph command, with distinct existing X/B/G/R
   bytes, checks the five enabled pixels and preservation of all other VRAM.
2. An **aligned** six-byte packed store uses R10=`0x11223344`,
   R11=`0x55667788`, destination bytes `0xaa`, and plane mask
   `0x0f0f0f0f`. All-enabled output is `a1 a2 a3 a4 a5 a6`; mixed enables
   `0xa8000000` produce `a1 aa a3 aa a5 aa`. This detects word-sized counts,
   repeated low-byte extraction, source compression, and omitted masking.
3. R2 zero preserves every destination byte even with nonzero sources.

All three currently stop at `Unsupported("SX memory command type")` as
expected. Their expected output is derived from the sources above, not from
an experimental handler. Promote them into the tracked suite when adding
packed conditional stores, and retain prevalidation before any VRAM mutation.
The review made no production changes and did not boot another guest.

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-packed/Cargo.toml --test packed
```

The **unaligned nonuniform packed source-lane rule remains unresolved**.
Repeated color bytes in the captured glyph do not distinguish restarting at
the register MSB from aligning source lanes to the destination word. The
driver preloads nine registers for up to 32 glyph bytes, and its copy routines
adjust for source/destination alignment; these are reasons to investigate the
general rule before declaring packed copies supported. The nonuniform fixture
above deliberately uses an aligned target. Do not present an unaligned
implementation guess as hardware-confirmed or silently accept unresolved forms.

The same ignored directory contains `guest-stop.png`, the final trace excerpt,
the failing test log, a register-state replay program and its output, and
hashes of production SX, the trace, framebuffer, harness source and original
Solaris library used for this review.

### Make the next rendering run an actual wrap/scroll test

The harness starts an 80-column, 24-row xterm, but its `LINE-...` and
`WRAP-...` strings are only 42 characters long. Neither wraps. It prints two
rows per second and then requests `sleep 8`; that wait alone does not promise
a full screen of output, and the run currently faults earlier. The infinite
loop also repeats identical rows, making scroll correctness harder to judge.

After implementing the packed glyph path, use a bounded producer with at
least 40 numbered rows and a known line longer than 80 columns, then leave the
terminal open for a capture. First run without olwm to isolate terminal text,
wrapping and scrolling. Then start olwm and validate title text, move, resize
and expose redraw. Capture the next actual unsupported command or incorrect
copy with its inputs; a packed glyph success does not validate packed loads
or overlapping copies. Finish with the rebuilt Heliacal keyboard, pointer
and redraw checks after those core rendering cases pass.

## Conditional packed glyph store applied

`SX_PACKED` conditional stores now consume four byte pixels per source
register, count bytes rather than words, apply the R2 enable bits from bit 31
downward, honor displacement, preserve disabled/unselected destination bytes,
and merge the plane mask into the existing destination byte. The captured
six-byte B32 glyph, aligned nonuniform source, mixed enables, zero enables,
and masked-channel fixtures pass and are promoted into `tests/sx.rs`.

The corrected deterministic xterm run prints its 40-row producer and long
WRAP line, then stops at the next unsupported form: `0x80500507`, an SX load
(operation 10, byte type, source R10, displacement 7). Text and scrolling are
therefore still not accepted; this is a read/copy path, not a store bug.

## First-scroll load review

The 18 tracked `tests/sx.rs` cases pass independently after the packed-store
addition. `/tmp/sx-fast12.png` shows **24 readable numbered terminal rows**,
`ROW-1` through `ROW-24`, in the 80x24 xterm. The harness now supplies a
40-row producer and a genuinely long WRAP line, without olwm. The capture
demonstrates the first screen of text; it does not show all 40 rows or the
WRAP line having been rendered.

### Captured load and the following copy path

The stop at PC `0xee657de4` maps to ELF `0x17de4`, inside the original
Solaris `ddxSUNWcg14.so.1` routine `sl_rect_copy_8`. The SPARC instruction is
`st %o4, [%o5]`, delivering SX command `0x80500507` to alias PA
`0x8ff420a60`.

| Field | Value | Meaning |
| --- | --- | --- |
| Count, bits 27:23 | 0 | One element |
| Operation, bits 22:19 | `0xa` | Load |
| Format, bits 18:14 | 0 | `SX_UBYTE_0`, unsigned byte, no shift |
| Register, bits 13:7 | 10 | **Destination R10**, not a source register |
| Displacement, bits 6:0 | 7 | Effective physical source `0xff420a67` |

The source is B32 pixel `(103,116)`, backing VRAM byte `0x8299d`. The load
must zero-extend that byte into R10, leave neighboring registers unchanged,
and preserve VRAM. R2 and the plane mask do not control this read.

The source lies one 13-pixel text row below the first terminal row. The
routine handles the unaligned destination with a byte prefix, then switches
to aligned word vectors. Together with the 24-row screenshot, this places
the stop at the start of the first vertical scroll. Original-driver evidence
is at ELF `0x17d64..0x17e48`; aligned bulk loads/stores and the tail follow
in the same routine through `0x185cc`.

Implement **direct unsigned-byte and long loads together** for the next
slice. The following bulk form is `0x8fd6c500`: 32 long elements into
R10..R41, reading 128 adjacent B32 pixels. A matching `0x8f8ec500` masked
long store is already in the implemented subset. The aligned word read
through B32 returns four consecutive B-channel pixel bytes in big-endian
order; it does not read one raw XBGR pixel.

The instruction definitions are independently recorded in NetBSD `sxreg.h`
revision 1.22: `SX_LOAD`, `SX_UBYTE_0`, `SX_LONG`, `SX_LDB` and `SX_LD`.
The Sun paper's data-format table distinguishes one byte per register for
ordinary byte loads from four bytes per register for packed data. Sources
and local original-driver artifacts are linked in the preceding review.

### Separate load dispatch from store controls

Operation `0xa` includes bit 3. The current expression `select = op & 8 != 0`
is a store control, so letting loads fall through that code would incorrectly
activate store-select and, for byte destinations R32 or above, the inferred
indexed-store path. Dispatch the full load operation before interpreting
store flags.

For the two verified direct formats:

1. Resolve the effective source through the existing CG14 aperture mapping.
2. Read and stage the complete vector before changing any SX register. If a
   later target is unsupported, preserve all register and VRAM state.
3. Commit one zero-extended byte or one full word to each successive
   destination register. Ignore R2, plane mask and ROP controls on loads.
4. Leave other formats explicit errors until their signedness, shifts,
   channel strides or packing rules are established. The current store width
   table alone is not a sufficient description of every load format.

### Five regressions and an isolated implementation proof

`local/review-2026-11-13-sx-loads/tests/loads.rs` contains:

- The exact byte-load command with a distinguishable B32 source `0x81`,
  neighboring register sentinels, zero store masks and unchanged VRAM.
- A four-byte load into R32..R35 crossing an eight-byte boundary, checking
  unsigned extension and preventing accidental indexed/store-select dispatch.
- A 32-word B32 load followed by the existing masked long store, checking
  all 128 destination pixels, register neighbors and other pixel channels.
- A synthetic 480x299 pixel rectangle copy upward by 13 scanlines, using
  byte prefixes/tails and aligned long vectors. Expected output comes from
  the original VRAM snapshot, so the fixture checks overlapping-source
  preservation and surrounding pixels independently of the copy loop.
- A vector that starts within the aperture but ends outside it, requiring
  an error without partial register or VRAM updates.

Against production, four tests stop at `Unsupported("SX memory command")`;
the failed-load state-preservation case already passes because the entire
operation is rejected. An isolated copy of `Sx`, with a separate staged
load path for formats 0 and `0x1b`, passes **all five**. Its patch is
`local/review-2026-11-13-sx-loads/load-proof.diff`; feature `load-proof` selects
the isolated implementation. This is a device-level proof, not another
guest boot or acceptance of general SX load/copy behavior. Production source
was not modified by this review.

```sh
# Production: four unsupported-load failures, one rejection-state pass.
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-loads/Cargo.toml --test loads
# Isolated implementation: all five pass.
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-loads/Cargo.toml \
  --features load-proof --test loads
```

The same directory holds the captured screen, trace excerpt, source/trace
hashes and both test logs. Integrate the tested load slice and promote the
fixtures, then repeat the guest run. Accept scrolling only after the capture
shows the end of the numbered sequence and the correctly wrapped long line.
Then resume olwm title/move/resize/expose and Heliacal input validation.
The earlier unaligned packed-copy uncertainty remains separate; these loads
and the synthetic scroll use ordinary byte and aligned long formats.

## Direct byte and long loads applied

SX load dispatch now runs before the store-control interpretation. Unsigned
byte and long loads resolve their effective CG14 aperture source, stage the
whole vector, and only then commit destination registers; R2, plane masks and
ROP controls are ignored. Source/target failures leave registers and VRAM
unchanged. The captured byte load and 32-word B32 load are implemented, and
the five load-review fixtures are promoted into tracked `tests/sx_loads.rs`.

The no-window-manager deterministic run now completes without an SX fault.
The capture shows ROW-20 through ROW-40 after scrolling and the long WRAP line
wrapped across two terminal rows. This establishes text and basic vertical
scrolling at the device level, not yet interactive redraw, olwm, or Heliacal
input validation.

## Scrolling checkpoint independently verified

Reviewed `/tmp/sx-fast13.png`, `/tmp/sx-fast13.err` and the matching serial
transcript after the production load change. The screen contains ROW-20
through ROW-40, the long WRAP line across two rows, and the final cursor row.
The trace contains no `CMDERR`, `FAULT` or `Unsupported` entries. The serial
script returns to its prompt after checking the running Xsun/xterm processes
and their empty logs. The 18 tracked SX tests and five tracked load tests pass
independently. This accepts **this Solaris Xsun text, wrap and vertical-scroll
case**; it does not establish interactive window redraw or frontend input.

The reviewed screenshot and hashes are preserved under
`local/review-2026-11-13-sx-scroll/`. Production code was not changed by this
review. General indexed addressing and unaligned packed-copy uncertainties
from the earlier sections remain open.

### Next acceptance run: olwm, then Heliacal interaction

Keep one Xsun session running while adding the following checks. Replace
the producer's final `sleep 30` with an interactive shell, or open a separate
normal xterm for keyboard tests. The current producer exits after the sleep;
it is not a useful interactive-shell test during that wait.

1. Start olwm explicitly and open two terminals with distinguishable titles
   and contents. Check complete title text, borders and resize corners.
2. Move windows left/right/up/down, including horizontal changes of one,
   two and three pixels. The accepted vertical scroll keeps horizontal
   alignment fixed; these moves exercise different source/destination lanes
   and copy directions. Capture after releasing the drag.
3. Overlap the windows, raise/lower them, uncover text, resize smaller and
   larger, and iconify/restore. Require intact text and cleared old window
   areas. Compare stable terminal interiors before/after expose, accounting
   for focus and cursor changes separately.
4. With rebuilt Heliacal using `--machine ss20 --display sx`, capture input,
   focus an interactive terminal and check typed text, Shift/Ctrl combinations,
   Enter/Backspace and the three mouse buttons. Exercise capture release with
   Ctrl+Enter and host focus loss while a key or mouse button is held; no
   modifier or drag should remain stuck after recapture. These controls and
   release paths were checked in `../heliacal/src/main.rs` and
   `../heliacal/src/emulator.rs`; their behavior in Solaris remains to be tested.
5. Save native-resolution captures with F4 at each accepted stage. If an
   action fails, retain its before/after screen, the first failed SX command
   and the associated source/register state. For corruption without an
   explicit fault, retain the command sequence for that action as a replay
   fixture. A successful startup screenshot alone does not accept these cases.

## Interactive shell and olwm checkpoint

The producer now ends with `exec /bin/sh`, keeping TERM-A interactive after the
40 rows. A second shell was opened as TERM-B under olwm. The before capture
shows both decorated, overlapping terminals with readable `TERM-A`/`TERM-B`
titles and the producer text/shell prompt in TERM-A.

Attempts to drag TERM-A by one, two and three pixels produced identical
before/after captures; no SX fault was reported. The core-side synthetic mouse
events were therefore not accepted by Solaris/Xsun, or the title-bar hit point
was wrong. This is an input-path issue, not a rendering fault. The before and
after PPMs are local evidence only; olwm move/resize/uncover/iconify and
Heliacal input capture remain unaccepted.

## Mouse delivery works after Solaris's startup hold

Two checked headless probes on separate clones of `/tmp/solaris26-sx2.raw`
reproduced the stationary pointer and then established the cause. Production
source was unchanged. The SCC B receiver is enabled and drains its input;
the probe's `pump` calls complete successfully. With only a few movement
events, the before/after captures remain byte-identical.

The original Solaris 2.6 mouse STREAMS module explains the missing events.
[`ms.c` 1.84](https://github.com/Arquivotheca/Solaris-2.6/blob/2.6/os_net/src_ws/usr/src/uts/sun/io/ms.c)
opens at B9600, sets `msd_rcnt_baud_chng`, and initializes
`MS_HOLD_BAUD_STUP` to 48. `msrserv()` withholds buffered movement events
until that count is reached, discards those initial events, and then starts
delivery. The observed SCC B WR12=`14`, WR13=`0` agrees with this initial
baud setting; keyboard channel A has WR12=`126` instead. The functional SCC
currently accepts decoded bytes independently of configured baud/framing.

Sending 64 alternating +1/-1 X movements, each followed by 1,000,000 guest
steps, crosses this startup hold. Stationary packets are insufficient because
`msinput()` discards events that change neither position nor buttons.

| Probe stage | CG14 cursor X | CG14 cursor Y |
| --- | ---: | ---: |
| Before movement | 575 | 449 |
| After event 47 | 575 | 449 |
| After event 48 | 575 | 449 |
| After event 49 (+1 X) | 576 | 449 |
| After event 50 (-1 X) | 575 | 449 |
| After event 64 | 575 | 449 |
| Then +20 X | 615 | 449 |
| Then +20 wire Y | 615 | 409 |

These are the high/low halfwords of CG14 cursor register `0x1104`. Captures
change after the subsequent motions, with no CPU/SX fault. The second probe
syncs its clone and reaches OBP `ok` after `/usr/sbin/halt`. This proves mouse
motion reaches Xsun; window dragging, button actions and keyboard interaction
are still separate acceptance work.

The experiment also establishes two positioning issues in the old harness:
pointer acceleration doubles the tested 20-count motions, and **positive wire
Y moves upward**. `Mouse::motion` currently documents positive Y as down, but
passes it unchanged; Heliacal compensates by calling `mouse_motion(xrel,
-yrel)`. Sun `msinput()` explicitly subtracts wire Y. The headless
`mouse_motion(-450,-340)` therefore cannot be treated as a precise move to
the title bar even after the startup hold.

### Two independent mouse-device regressions

`local/review-2026-11-13-sx-mouse/tests/mouse.rs` exposes two additional defects
through the real Machine/SCC path:

1. **Nanosecond scaling:** `Mouse::advance_ns` multiplies an already-nanosecond
   interval by 1,000,000,000 before dividing by a nanosecond byte duration.
   The receiver obtains bytes after just **one nanosecond**, and long bursts
   are limited by FIFO backpressure rather than the configured mouse rate.
   Use a consistent nanosecond/rational accumulator, as the keyboard does.
2. **Reserved payload values:** splitting at -128 emits `0x80..0x87` as
   movement bytes. The captured large negative move begins
   `87 80 80 00 00`. NetBSD's
   [Sun mouse manual](https://man.netbsd.org/NetBSD-9.4/sparc/ms.4)
   reserves that range for headers, and
   [pinned MAME `extract_delta_byte`](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/sunmouse/hlemouse.cpp)
   splits deltas into -120..127 while preserving the remainder. The fixture
   checks header-safe payloads and preservation of the complete motion.

Both fixtures fail against production. These are separate defects from the
48-event startup hold. In particular, Solaris 2.6's parser accepts signed
payload bytes while in a data state, and this review demonstrates working
motion with the current production endpoint after the hold is satisfied.
The existing mouse timing test does not assert arrival boundaries and reads
zero-valued packet tails without first checking receive-ready status.

### Packed op 9 review and the next experiment

The new packed op 9 was triggered by `0x824fc504` at PC `0xee670860`, ELF
`0x30860` in `sl_span_fill_8`. It fills five B32 pixels at effective address
`0xff47a5e4`; R10 and R11 both contain repeated color `0x05050505`.
`tests/packed_select.rs` in this review directory reproduces that fill with
distinct surrounding pixel channels and passes. This evidence covers the
observed identical-color fill, while generic mixed-source packed selection,
unaligned source lanes and nonuniform plane-mask lanes remain unconfirmed.
The 23 tracked SX/load tests also pass independently.

Continue the acceptance harness in this order:

1. Check every `pump` result. The old drag blocks use `let _ = pump(...)`,
   which hides CPU/SX failures; a matching screenshot cannot establish an
   input failure by itself.
2. Perform paced startup motion and wait for observed cursor movement before
   doing hit tests. Keep this startup sequence in the experiment harness;
   preserve the guest driver's behavior.
3. Disable acceleration for pixel measurements with
   `DISPLAY=:0 /usr/openwin/bin/xset m 1/1 0`, then position the pointer using
   the measured coordinates and correct wire-Y sign. Use small deltas while
   the negative-range fix is outstanding.
4. First demonstrate a clear larger title-bar drag and release. Then exercise
   final window offsets of one, two and three pixels, measuring the resulting
   frame position. This avoids treating a tiny attempted gesture as proof
   that a drag began. Resume resize/expose/iconify and Heliacal input checks.
5. Integrate the timing/range fixes with the two independent mouse regressions
   and promote the observed packed-fill fixture before claiming these paths
   covered by the tracked suite.

Artifacts under `local/review-2026-11-13-sx-mouse/` include the checked harness,
two cloned disks, before/after PPMs, `cursor-evidence.txt`, probe/serial logs,
source and capture hashes, the failing mouse regressions and the passing
packed-fill fixture. Downloaded original Sun sources remain ignored there.

## Mouse timing/range fixes and current acceptance state

`Mouse::advance_ns` now accumulates nanoseconds without the extra 1e9 factor,
so a 1200-baud byte requires the configured byte interval. Negative deltas are
split into the header-safe `-120..=127` payload range while preserving the
remainder. The two independent mouse regressions are promoted to tracked
`tests/mouse.rs`, and the captured packed-op-9 fill is tracked in `tests/sx.rs`.

The local `/tmp/probe_sx_olwm2.rs` harness sends 64 paced alternating one-pixel
startup movements and checks the mouse-stage pump results. Its comment says
acceleration is disabled, but it contains no `xset` command. Inspection of
`/tmp/sx-olwm4.err` shows the larger attempted drag ends at cursor image origin
`(38,248)`, outside TERM-A's title bar. The following attempts end at `(39,248)`,
`(41,248)` and `(44,248)`. Each capture differs from the before image by 100
pixels representing the old/new cursor; the windows remain fixed. Equal `nz`
counts in the dump log do not mean equal images. The two-terminal capture
still shows complete titles/borders and overlap; actual dragging, resize,
uncover, iconify and Heliacal input capture remain open.

## Calibrated olwm drag reaches a packed outline operation

Independent review of the timing/range fixes passes all 26 tracked tests in
`mouse`, `sx` and `sx_loads`. Production source was unchanged throughout this
review, checked against hashes of `mouse.rs`, `serial.rs` and `sx.rs`.

The corrected probe boots a fresh clone of the previously cleanly halted
review disk, starts Xsun, olwm and the two persistent terminals, then runs:

```sh
DISPLAY=:0 /usr/openwin/bin/xset m 1/1 0
DISPLAY=:0 /usr/openwin/bin/xset q
```

The serial transcript confirms `acceleration: 1/1` and `threshold: 0`. After
64 paced startup movements, the probe reads CG14 register `0x1104` after each
small movement and reaches cursor image origin `(350,112)`. Cursor shapes and
hotspots change when crossing windows, so measuring again is necessary even
with acceleration disabled. `xwininfo -root -tree` independently places
TERM-A's frame at `494x346+100+100`; the saved positioned capture visibly puts
the pointer in its title bar.

Left-button press followed by +20 X motion reaches a new **SX error**, proving
that the mouse gesture now reaches the window manager's drawing path:

```text
SX REG 0x1014 <- 0x00000055
SX REG 0x1010 <- 0xffffffff
SX REG 0x1324 <- 0x00000000
SX REG 0x1020 <- 0xb8e00509
SX CMD PA 0xff01c260 <- 0x8fd7d504
PC=0xee66fc34, SPARC instruction=0xd6224000
Unsupported("SX load format"), bus address=0x8ff01c260
```

This is the first observed fault. The original `ddxSUNWcg14.so.1` disassembly
maps that PC to ELF offset `0x2fc34`, in `sl_rect_fill_8_rop`. Its continuation
identifies a useful complete implementation slice:

| Status | SX word | Operation |
| --- | --- | --- |
| Captured fault | `0x8fd7d504` | Packed load, operation `0xa`, format `0x1f`, 32 bytes, R42, displacement 4. |
| Derived next instruction | `0x9822952a` | ROP_M, A=R10, B=R42, D=R42, nine registers, ROP control `0x55`. |
| Derived following store | `0x8f8fd504` | Packed store with plane mask, operation `1`, 32 bytes from R42, displacement 4. |

The memory address is **X32**, effective PA `0xff01c264`, corresponding to
the X bytes of pixels `(100,100)` through `(131,100)`. This is the outline
operation encountered when the drag begins; the run has not yet completed a
window move or reached the one/two/three-pixel release checkpoints.

Implement this load/ROP/store sequence together:

- A packed load places four bytes in each register. The captured aligned
  32-byte transfer fills R42..R49, rather than 32 separate registers. Validate
  the packed register span, resolve every source through its CG14 aperture,
  stage reads, and commit only after all targets are valid. The fixtures also
  require a 32-byte transfer into R120..R127 to succeed.
- The ROP_M instruction is a raster operation using the ROP control register;
  it is not one of the SELECT opcodes. With the captured `0x55` control it
  complements source B. R10..R18 were cleared by the preceding ORS command.
  Preserve overlapping-source semantics and neighboring registers.
- The following packed store has a plane mask but **no conditional R2
  enable**. R2 is zero in the captured state. Reusing the conditional op-5
  path without separating its enable test would silently suppress the
  outline. Preserve the other B/G/R channels and surrounding pixels.

Independent references are the [NetBSD SX encodings, revision 1.22](https://cvsweb.netbsd.org/bsdweb.cgi/src/sys/arch/sparc/dev/sxreg.h?rev=1.22&content-type=text/x-cvsweb-markup),
Sun's [Pixel Processing in a Memory Controller](https://www.sunhelp.org/history/pdf/smc.pdf)
table 3 and sections 4.1.7/4.1.10, and the original DDX instruction stream.
NetBSD `cgfourteen.c`'s `0x3c` ROP comment (`~src a / src a`) independently
establishes the nibble truth-table ordering used for the `0x55` fixture.
The two later commands above are derived from disassembly, not claimed as
executed guest commands. Partial packed tails and unaligned lane placement
remain separate questions; the new fixtures use complete aligned words.

### Reproduction and fixtures

`local/review-2026-11-13-sx-hit-test/` contains the corrected harness, source
hashes, original failing harness/trace, pointer-position evidence, before and
positioned PPM/PNG captures, checked serial/trace logs, the fault capture and
DDX excerpt. Both guest reruns used private clones; the faulted clone was not
cleanly halted and is diagnostic evidence, not the baseline for the next run.

The six independent fixtures in `tests/packed_outline.rs` use a synthetic
nonuniform X-plane pattern and the captured/derived command words. Five fail
against production at the corresponding unsupported load, ROP and store;
the invalid-target atomicity fixture passes. These are intentionally isolated
under ignored `local/`, ready to promote once the implementation passes:

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-hit-test/Cargo.toml \
  --test packed_outline
```

Run the harness with explicit paths to a fresh private disk clone and output
prefix. It checks all gesture pumps, stops setup on a failed command, records
serial progress, and marks the expected response before transmitting each
command. The source additionally saves raw VRAM on future fault captures;
the existing checked run saved register state and the rendered image, not
raw VRAM.

An initial review attempt put `xset` before any persistent client: `xset m`
returned, then `xset q` stalled and the cursor was disabled. That attempt
did not validate any gesture. A last-client server reset is a plausible
explanation, consistent with the [X server reset documentation](https://www.x.org/archive/X11R7.5/doc/man/man1/Xserver.1.html);
its precise stall was not diagnosed. Starting olwm and the terminals before
the setting/query avoids this in the successful fault-reproduction run.

Next: implement and promote the outline regressions, rerun the calibrated
drag through release, then test small final offsets, resize/uncover/iconify
and Heliacal capture/release. Do not infer acceptance from unchanged windows
or equal nonzero-pixel counts. Also correct `Mouse::motion`'s misleading Y
documentation: the implemented core API uses positive wire Y upward, while
Heliacal already supplies `-yrel`.

## Packed outline load/ROP/store applied

The fault capture's complete outline slice is now implemented: packed loads
gather four X-plane bytes per destination register (including R120..R127),
ROP_M applies the 4-bit truth table from the ROP control register with source
snapshots for overlap, and packed masked stores accept operation 1 without
requiring R2 enables. The six outline fixtures are promoted into tracked
`tests/packed_outline.rs` and pass, along with all prior SX/load/mouse tests.
The calibrated harness's window move still needs a fresh guest run after this
change; no move is yet accepted.

## Outline operations applied and calibrated drag result

Packed loads into R42..R49 and R120..R127, ROP_M with the captured `0x55`
control, and packed masked stores with operation 1/nonzero source registers
are implemented. The five failing outline fixtures plus the invalid-target
atomicity fixture are promoted into tracked `tests/packed_outline.rs`; all
pass. The calibrated harness runs the outline commands and submits the larger
drag gesture; before/positioned/large-drag captures are saved under
`local/review-2026-11-13-sx-outline/`.

Independent inspection of the complete serial log finds an earlier SX bus
fault at `0xee66f104` and Solaris panic, before the `ESP stacked interrupt`
during panic dumping. Held and released images are identical and TERM-A has
not moved. The `STD` to the SX instruction queue is incorrectly split across
offsets `0x20` and `0x24`; the second word causes a user `ReadOnly` bus fault.
The harness missed `Ok(Step::TrapTaken { .. })` by checking only `Err`.
See [paired SX instructions and the first panic](ss20-sx-std-panic.md) for the
exact CPU/MMU reproduction, doubleword/atomicity regressions, two additional
outline gaps and a harness that captures the first graphics bus trap. Move,
resize/uncover, iconify and Heliacal input checks remain unaccepted.

## Paired-IQ fixes reviewed; funnel and copy alignment next

The paired-IQ, packed-tail and mask-selected ROP regressions now pass; the
independent focused run passes all 41 SX/load/outline/queue/mouse tests.
The larger gesture's held/released captures differ, but its final frame
geometry is still `494x346+100+100`; only the pointer differs from the initial
released image. This does not yet accept a window move.

The following small gesture stops on release at PC `0xee65a44c`, in
`sl_rect_copyrl_8`, submitting `0xc8e28518`: funnel-left immediate 24,
R10..R19 to R10..R18. The driver sequence also exposes a packed-byte alignment
constraint for its following unaligned store. See the
[funnel and move acceptance review](ss20-sx-funnel-review.md) for eight
isolated fixtures, captured versus inferred operands, harness diagnostic
gaps, and the remaining merge gate. Production was not changed by the review.

## Real moves verified; unaligned packed loads corrupt rendering

The funnel/store follow-up passes its eight promoted fixtures, and the new
guest capture proves successive 20/1/2/3-pixel geometry changes. Independent
image comparison finds text corruption after move2 and repeating title-bar
stripes after move3. Captured unaligned X-plane load/ROP/store commands fail
both inversion and draw/erase restoration because loads still begin at lane
zero and omit the ninth register. See the
[unaligned-load review](ss20-sx-unaligned-load-review.md) for the reproduction,
six isolated regressions and remaining acceptance work. All 483 tracked core
tests and required static/demo checks pass; visual correctness remains a
merge blocker. Production was not changed by this review.

## Packed-load fix independently accepted for window moves

Accepted-run4 confirms all four geometry deltas with zero translated-text
differences. Independent comparison also preserves more than 60,000 visible
TERM-A pixels per move, including borders and title bar outside the pointer;
TERM-B is unchanged and vacated strips return to the desktop background.
The six load regressions and all 489 core tests pass, as do formatting,
Clippy and demos. See the latest section of the
[unaligned-load review](ss20-sx-unaligned-load-review.md) for exclusions,
metrics and the remaining resize/uncover/iconify, Heliacal input and SS-5/Axil
boot checks before merge. The observed movement/rendering defect is resolved;
production was not changed by this independent review.

## Run 21 resize/uncover/iconify reviewed

Independent pixel checks accept the recorded moves, uncover, Full Size/Normal
Size and iconify/restore. The entire normal-sized client returns identically;
the shrink report's 1,392 non-desktop pixels are exactly its surviving bottom
border, and all pixels below that border are desktop. All 491 core tests,
formatting and demos pass; a redundant cast in the new load test was removed
to make Clippy pass too. Stale human documentation has been reconciled.

The new unaligned store path still partially mutates VRAM when a later byte
or vector element crosses the aperture boundary. See the
[resize and final-store review](ss20-sx-resize-review.md) for two isolated
failure reproductions, two passing controls and the complete-span prevalidation
needed before merge. Heliacal rebuild/input and SS-5/Axil boot checks remain.
