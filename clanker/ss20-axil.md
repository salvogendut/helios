# SPARCstation 20 and Axil 320 bring-up

Implemented 2026-11-08 on branch `ss20-axil320`. Goal from the user: add new
machine profiles for a single-processor SPARCstation 20 and the Axil 320 clone,
reuse the SS-5 device models, and reach the original firmware `ok` prompt.

## Identity and map

- IDPROM machine type `0x72` for both machines (Sun NVRAM/hostid FAQ and the
  physical Axil banner). SS-5 stays `0x80`.
- Axil default identity from the user's physical banner and earlier strings:
  MAC `00:00:3c:13:de:ad`, serial `c0:ff:ee`, hostid `72c0ffee`. SS-20 default
  uses the existing Sun MAC `08:00:20:c0:ff:ee`, also hostid `72c0ffee`.
- Sun-4M architecture: 36-bit PA; boot-mode instruction fetch uses
  `PA[35:28]=0xff`; ASI `0x20` is 32-bit bypass and `0x21..=0x2f` form
  `(ASI&0xf)<<32 | VA`; SBus slots use `PA[35:28]=0xEn`.
- QEMU SS-20 hwdef supplied the concrete bases: IOMMU `0xfe0000000`, PROM/SLAVIO
  `0xff0000000`, keyboard/TTY/NVRAM/timers/intctl under `0xff1...`, MACIO
  SCSI/Ethernet under `0xef0...`, ECC `0xf00000000`, VSIMM probes `0x8c..0x9c`.
- DRAM banks are 64 MiB each at `0x00000000`, `0x04000000`, ... `0x1c000000`;
  128 MiB default uses two 64 MiB modules, low slots first.

## Implementation choices

- `Sun4mLayout` now carries every device base, the PROM base and the SS-20
  empty CPU timer/intctl and VSIMM probe windows. `MachineProfile` selects
  layout, `MmuConfig`, IDPROM identity and DRAM geometry.
- `MmuConfig` parameterizes microSPARC-II vs SuperSPARC-II reference behavior:
  31/36-bit PTEs, boot mask `0x0fffffff` vs `0x7ffff`, boot bit 14 vs 13,
  CTPR/TRCR masks, MMU version and control mask.
- ASI `0x21..=0x2f` bypass, `0x02` MXCC register storage, `0x30..0x32/0x36/0x37/
  0x39` diagnostics, `0x38/0x49..0x4c` breakpoint counters, and `0x05/0x06`
  I/D-TLB diagnostic storage were added for the reference profile. TLB diag
  storage is keyed by full address so POST entry/tag writes do not alias;
  flushes clear it. SS-5 microSPARC-II paths keep their existing strictness
  (cache doubleword diagnostics remain reference-only).
- SMC ECC at `0xf00000000` is a register subset (implementation 2, version 0),
  extended to 0x40 bytes for the Axil probe; no parity/error injection/IRQ.
- Empty VSIMM windows and CPU1–3 timer/intctl slots return zero and ignore
  writes. The ESP accepts SS-20/Axil 32-bit register lanes and test-mode probe
  commands only when the layout is non-SS-5; SS-5 byte-only tests stay intact.
- The IOMMU block gained version 0x13000000 for SS-20/Axil plus the AER at
  `+0x1008`; the shared model retains the register on SS-5 without using it.

## Evidence and remaining gaps

`ss10-20-v2.25r-rom` (SHA-256 `b4f15bfa...b48ca8`): POST 3.45 identifies
`TI, STP1021PGA(1.x)` and CPU 1–3 absent. The ICACHE_TLB bit-pattern and
context/region/segment/page/entire flush tests now pass, including the former
region-flush mismatch. The D-TLB cached level-2 PTP check also passes: the
model fills a diagnostic PTP2 view (ASI 6 SEL 5/6) from the level-1 table and
lets a matching 256 KiB PTP2 take the level-2 path instead of a 16 MiB leaf.
Diagnostic TLB writes through ASI 5/6 now install real entries on the reference
profile and allow a reference-profile detached write to set M locally.

POST now passes the I/D-TLB bit-pattern, flush, PTP2, `TLB_HIT`, `TLB_MISS`
and `TLB_PROBE` diagnostics. The diagnostic model fills PTP2 slots, mirrors
normal fills into the ASI 5/6 view, installs ASI diagnostic entries as real
translations, indexes the D-TLB diagnostic PTE array by both the entry field
and consecutive low offsets, and lets the SS-20 probe return the level-2 entry
with the TLB R/M bits. PSR.DE (bit 15) now implements the SuperSPARC-II dual
byte ordering: halfword, word, integer doubleword and FPU loads/stores apply
the documented byte conversions, while instruction fetches and table walks
stay big-endian. The integer and FPU endianness tests pass.

The next diagnostic is the CPU/MBus module test:

```text
<<< CPU_00000000 on MBus Slot_00000000 >>>
 ERROR : Address = 00000200,
 exp = 00000100, obs = 00000000, xor = 00000100
 U-NUMBER : Suspect Viking Module
```

The CPU/MBus module test now gets past the MMU context-register check: the
reference-profile context register keeps its full written value (the SS-5
profile still masks to 8 bits). It then exercises 64-bit I/D-cache tag
diagnostics. A partial SuperSPARC-II tag model is in place for ASI 0x0c/0x0e
with per-address implemented-bit masks; the current failure is the tag-clear
semantics at `0x4000_0000` (`exp[31:0]=0`, observed `0x1e`). The
SX/DBRI/BPP placeholders now let OBP finish SBus enumeration and reach the
serial monitor; the exact windows and the superseding resolution are recorded
below.

Two further firmware-flow gaps surfaced once the SX fault was removed:

- With a blank/default NVRAM image, OBP reports an invalid configuration
  checksum, rewrites defaults, sets `diag-switch?` true and resets. It does not
  reach a stable `ok` within the tested step budgets.
- With the rewritten SS-20-format NVRAM image, OBP allocates SRMMU tables,
  probes both 64 MiB banks, then spins in its relocated RAM loop around
  `0xffef0000`; the cause is not yet identified (input, interrupt or memory
  probe handshake).

PROM behavior for this profile: OBP maps a 16 MiB alias at `0xf_f000_0000`
(`Mapping ROM @ 0x00000000` / `0xffd00000`). The first 1 MiB comes from the
image; the rest reads `0xff`. Reference-profile PROM write strobes are ignored,
which the `D_cached_level2_PTP` test requires when it rewrites the alias.

The Axil ROM `axil320-2-19.bin` (SHA-256 `bc231c32...e77425`) now reaches the
same NVRAM configuration-checksum reset path; it has not yet been re-verified to
a prompt after the VSIMM/TLB fixes. No POST-pass, OS-boot or full SuperSPARC
CPU/MBus claim is made.

Next steps: finish the SuperSPARC-II I/D-cache tag diagnostic clear/half
semantics, then select and validate an NVRAM image the SS-20 OBP accepts, and
investigate the post-memory-probe spin loop. No SuperSPARC pipeline, MBus multiprocessor, SX
graphics, DBRI, BPP or Axil custom memory-controller behavior is modeled.

## MAME/QEMU CPU survey (2026-11-08)

MAME's SS-20 (`src/mame/sun/sun4.cpp`) is a `sun4c_state` stub marked
`MACHINE_NOT_WORKING`; its SPARC core has only V7/V8/MB86930 and the MB86930
cache-tag ASIs return zero/ignore writes. MAME's sun4c MMU masks tag writes to
`0x03f8fffc`, so it is not a SuperSPARC II cache-tag oracle.

QEMU is useful for CPU and machine identity but not cache tags. Its SS-20 hwdef
matches our IOMMU/slavio/MACIO/ECC map; QEMU's `TI-SuperSparc-II` uses IU
`0x40000000`, MMU `0x08000000`, MXCC version `0x104` and 8 windows. The
`TI-SuperSparc-50/60` models use IU `0x40000000`, MMU `0x01000800` and no MXCC.
QEMU's ASI `0x0c`/`0x0e` tag accesses are deliberate no-ops; its MXCC support is
the ASI `0x02` stream/control register set (`0x1c00000`..`0x1c00f00`). QEMU also
sets the `%psr` IU version from `iu_version`; Helios still reports the
microSPARC-II `0x04000000` word for every profile.

Experiment: changing IU version to `0x40000000`, MMU version to `0x01000800`,
and MXCC MBus-port value to `0x08000000` made OBP report
`TI, TMS390Z50(3.x) 0Mb External cache`, skipping the external-cache test. It
then failed the on-chip tag test at `0x8000_0000` (`exp[63:32]=0x01010100`,
observed `0x01000000`), so no-MXCC is not a clean-POST shortcut.

## Current SS-20 blocker (2026-11-08)

Stop-A station `0x01` + A `0x4d` prints
`Skipping POST because of L1 keyboard command.` and skips POST. Station `0x4f`
is `L1-D` and sets `diag-switch?`; do not confuse them. With the valid
`/tmp/ss20-after160.nvram`, OBP initializes SRMMU, maps RAM/ROM and probes the
banks, then serial output stops. The CPU is not halted: the relocated Forth
inner interpreter at `0xffef0000` keeps executing. RAM strings at
`0xffef95aa` contain `/obio/zs@0,0`; recorded Forth stacks contain
`find-device`, `get-attribute`, `name`, `save-string` and `$vfind`. Runs of
1.1B, 5B and 1.2B instructions still show this path search. The next target is
the `/obio/zs@0,0` console/keyboard node, not the cache tags.

Keyboard command 7 now replies with the Type 5 US layout (`0xfe 0x21`), matching
QEMU's ESCC handler; command 15 already did. This did not by itself change the
device-tree loop. NVRAM config patches that change the `keyboard`/`screen`
strings are rejected by OBP's late configuration checksum; zeroing the
non-checksummed `0x1e00` area does not fix the loop.

## ZS console trace (2026-11-08)

Guests' physical RAM contains the OBP device tree and the aliases
`console-tty = /obio/zs@0,100000` and `kbdmouse-tty = /obio/zs@0,0`. The
`/obio/zs@0,0` node is therefore not missing. The OBP programs ZS channel A
(offsets 0/2 are channel B, 4/6 channel A), reads RR0 at offset 4 and receives
the keyboard ID `ff 04 7f` plus injected key bytes from offset 6. It then goes
back to polling RR0. The keyboard driver is alive; the remaining stall is in
console/device opening, most likely the missing `screen` output device.

Keyboard identification is now re-sent when the SCC receiver is enabled after
an empty FIFO, and command 7 returns the same Type 5 US layout reply as command
15. Tests cover both. Installing a CGsix at `0xe2000000` or `0xef1000000` did
not change the stall. The next useful target is the OBP `screen` device
(TCX/SX/VSIMM), or a valid NVRAM `output-device=ttya` image.

## ttya/console investigation (2026-11-08)

The NVRAM strings at `0x769`/`0x78a` are not the active console values.
Replacing them with `ttya`/`ttya//` and letting OBP recompute the configuration
checksum produced an accepted image, but OBP still built its RAM
`input-device`/`output-device` defaults as `keyboard`/`screen`. Patching the
corresponding RAM default table (`input-device`/`keyboard`,
`output-device`/`screen`) or the ROM default table at `0x6fbc1` also did not
change the active console. The device tree/console open still stalls after the
memory-bank probes.

`ok Level 14 Interrupt` output seen during these experiments is the level-14
timer interrupt handler, not a console prompt; masking/removing the keyboard or
CPU timer stops the message but does not reach `ok`. Do not treat that output as
a prompt.

## SX/DBRI console resolution (2026-11-09)

The post-memory stall was a missing physical decode, not the keyboard driver.
OBP faults at physical `0xf80000030` (SX); the existing `0xff8000000` alias is a
different address. Adding `0xf80000000..0xf80002000` as zero-read/write-ignore
advances SBus enumeration. The next fault is DBRI at `0xee0001000`; QEMU's
`ss20_hwdef` places an unimplemented DBRI PROM at `dbri_base+0x1000` (0x30
bytes, base `0xee0000000`) and registers at `+0x10000` (0x100 bytes). Adding
both stubs, plus the BPP placeholder at `0xef4800000` (0x20 bytes), reaches the
serial monitor. The old high VSIMM aliases must stay; removing them reintroduces
an unmapped access at `0xff8c00f04`. `zero_windows` is now 22 entries and
`device_ranges` has 41 entries.

With 128 MiB and the valid NVRAM image, Stop-A after 100M instructions aborts
the long memory test and reaches a real `ok`. The OBP's own commands then work:

```text
ok setenv output-device ttya
output-device =       ttya
ok setenv input-device ttya
input-device =        ttya
ok setenv auto-boot? false
auto-boot? =          false
ok setenv diag-switch? false
diag-switch? =        false
```

The resulting `/tmp/ss20-prompt.nvram` boots to `ok` without keyboard or
host-side patching. POST still prints the MBus cache-tag error at
`0x40000000`, so this is an interactive console, not a clean POST. A serial
`ok` is reached; no OS boot, DBRI audio, SX/VSIMM graphics or full POST claim is
made.

The SS-20-generated `local/ss20-ttya.nvram` is not accepted by Axil ROM 2.19:
Axil reports `screen not found`, falls back to tty, then rejects the SS-20
configuration checksum and resets to defaults. Axil needs its own prompt/setenv
session; do not reuse the SS-20 NVRAM image for an Axil clean-prompt claim.

## VSIMM/SX research (2026-11-09)

The SS-20's built-in video is the SX/cgfourteen, not an SBus CGsix. It is an
obio/mainbus device and requires a VSIMM in one of the wide DSIMM slots; with no
VSIMM there is no built-in frame buffer. NetBSD is the useful register source:

- cg14 control block: 0x10000 bytes at VSIMM register base; control at 0x0,
  hardware cursor 0x1000, DAC 0x2000, XLUT 0x3000, CLUT1/2/3 at
  0x4000/0x5000/0x6000, CLUT increment 0xf000. VRAM is a second register set
  whose size is the VSIMM size (4 or 8 MiB).
- VSIMM slots in QEMU: control/VRAM pairs 0x9c000000/0xfc000000 and
  0x90000000/0xf0000000; further control bases 0x94000000/0x98000000.
- SX mainbus registers at 0xf80000000, 0x1000 bytes; user register page at
  +0x1000. SX_ID=0x28 (arch bits 1:0, chip revision 7:3), page bounds 0x08/0x0c
  default 0xfc000000/0xffffffff, instruction queue 0x20, direct R0 0x100,
  queued R0 0x300. Rendering memory ops use ASI 0x28 at physical
  0x800000000+address.
- QEMU's SS-20 creates TCX at 0xe20000000 and only empty SX/VSIMM slots; it is
  not an SX reference. Helios still exposes SX/VSIMM bases as zero placeholders
  for OBP enumeration; the frame buffer/rendering engine is not modeled.

## VSIMM/SX implementation (2026-11-09)

`src/devices/sx.rs` now implements a functional built-in SX/cg14 subset for the
SS-20 and Axil profiles: the SX register block at `0xf8000000` (ID and page
bounds), the VSIMM control block at `0x9c000000` (cursor/DAC/CLUT storage), the
8/16/32-bit-addressable VRAM window at `0xfc000000`, and its SX-memory alias at
`0x800000000`. `Machine::with_profile` installs it for non-SS5 profiles, and
`Machine::framebuffer_pixels` exposes either CGsix or SX pixels to a frontend.

The OBP now probes the VSIMM control register without faulting and no longer
reports `screen not found` during the tested SS-20 boot. It still selects ttya
and does not initialize the cg14 console: `mctl` remains 0 and the framebuffer
only contains probe patterns, not a Sun logo or text. The 64 MiB VRAM window is
a bring-up aperture so the OBP's size probe cannot hit an unmapped page; the
modelled VSIMM is still functionally 8 MiB. SX rendering instructions, DBRI,
CLUT/DAC indirect sequencing and the OBP cg14 console binding are not complete.

The desktop frontend now accepts `--machine ss20`/`--machine axil320` and calls
`Machine::framebuffer_pixels`, which selects the built-in SX path when no CGsix
is installed. The OBP still opens `/obio/cgfourteen@2,0` unsuccessfully
(`open-dev` returns 0), so the screen falls back to ttya; the UI therefore shows
the SX VRAM state but not yet a firmware console.
