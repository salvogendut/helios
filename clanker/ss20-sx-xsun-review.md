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
