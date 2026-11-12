# TurboGX graphical console and Sun keyboard

Helios can run the original SS-5 OBP console in a native window with a 1 MiB
TurboGX (CGsix) and a Sun Type 5 US keyboard. The selected display is
1152×900, 8-bit indexed color. The firmware discovers the card by executing its
original SBus FCode ROM; Helios does not draw firmware text or substitute a
host terminal for the display.

## Running

From the repository root:

```sh
python3 tools/fetch_firmware.py sun-ss5
python3 tools/fetch_firmware.py turbogx
cargo run --release --manifest-path helios-ui/Cargo.toml -- \
  local/firmware/sun-ss5.bin local/firmware/turbogx.bin
```

The desktop crate has its own lockfile and dependencies. The headless root
crate still builds offline without GUI libraries. On Linux the frontend uses
X11, including XWayland: an accessible display, X11 development headers and
`pkg-config` are needed to build/run it. Other desktop platforms have not been
validated. Rust 1.74 remains the declared minimum.

Allow the firmware to finish its 128 MiB memory test. With fresh NVRAM it
initializes the OBP defaults, including diagnostic mode and automatic network
boot. Hold **Pause** and press **A** to send Sun **Stop-A**, interrupting the
network boot and returning to `ok`. Hosts providing F13 can use it as Stop too.
Type `1 2 + .` and Enter: OBP prints `3`. Escape is a guest key; close the window
to exit. Input uses a US Sun keyboard layout, including modifier, function,
cursor, navigation and numeric-keypad stations. Menu maps to Compose.
Sun Mouse Systems input is provided by the frontend through SCC channel B at
1200 baud. It uses the five-byte ``0x80 | (~buttons)``, dx, dy, dx, dy packet;
left/middle/right map to button bits 4/2/1. Clipboard paste and audible
bell/click output remain pending.

The console ports remain available in the machine. TTYA output goes to the
launching terminal; the window receives keyboard input independently. When an
unsupported operation stops the core, the last framebuffer remains visible
and the window title names the unsupported operation; the terminal reports the
full CPU fault. **`probe-scsi-all` lists attached disk/CD-ROM targets** when
launched with `--disk PATH` and/or `--cdrom PATH`. Use `--disk-rw PATH` instead
of `--disk` to allow guest writes to an existing disk; see [SCSI storage](scsi.md).
Close and restart the
window after a core stop; Stop-A cannot resume a stopped interpreter.
Window resizing affects presentation
only. A worker owns the machine; display refresh never advances guest time.
Key events retain order through bounded queues, and losing focus releases held
keys. Host key repeat is suppressed because guest firmware owns repetition.

Optional NVRAM persistence:

```sh
cargo run --release --manifest-path helios-ui/Cargo.toml -- \
  local/firmware/sun-ss5.bin local/firmware/turbogx.bin \
  --nvram local/nvram.bin --nvram-out local/nvram-next.bin
```

The output must be a new path. It is saved on normal window closure or a core
error. Existing input images are never rewritten. At `ok`, `setenv auto-boot?
false` disables automatic boot on subsequent starts using the saved NVRAM.
An imported NVRAM image may explicitly select a serial console; its settings
are honored. The default IDPROM remains MAC `08:00:20:c0:ff:ee`, hostid
`80c0ffee`.

## Device contract

On the SS-5 the card occupies SBus slot 1, physical base `0x30000000`, with a
32 MiB aperture; OBP names it `/iommu/sbus/cgsix@1,0`. On SS-20/Axil the same
card is installed in slot 2 at physical `0xe_2000_0000` (`cgsix@2,0`). Helios
selects the base from the machine layout; the register offsets below are
relative to either base.

| Offset | Implemented behavior |
| --- | --- |
| `0x000000–0x007fff` | Read-only 32 KiB FCode ROM |
| `0x200000–0x20000f` | BT458 address, palette, control and overlay ports, high byte of each word |
| `0x280000` | Alternate oscillator programming latch |
| `0x300000` | FHC: TurboGX revision 11, monitor sense for 1152×900; byte/halfword reads supported |
| `0x301090–0x3010cc` | Selected TurboGX strap, delay and extended timing registers used by the ROM |
| `0x301800–0x301818` | Legacy timing registers and video/vertical-blank control/status |
| `0x3018fc`, `0x301900–0x3019fc` | Signed cursor position and two 32×32 cursor planes |
| `0x700000–0x700fff` | FBC registers: coordinates, clipping, offsets/increments, colors, ROP/masks/patterns, font, polygon and blit operations |
| `0x701000–0x701fff` | TEC register page; values retained, transformation arithmetic is not modeled |
| `0x800000–0x8fffff` | 1 MiB packed indexed VRAM, big-endian byte lanes |

ROM and VRAM allow byte, halfword, word and aligned doubleword access. Register
accesses require words except FHC reads. Rejected narrow, doubleword and atomic
register operations have no side effects. Unknown registers and unsupported
drawing operations stop explicitly. ROM and device apertures reject conflicting
RAM mappings.

BT458 RGB components advance individually; after blue the palette address
increments. Reading/writing the address resets its component pointer. Scanout
uses palette/read-mask, palette enable, overlay enables and blink masks. Video
disable blanks the presentation without destroying VRAM. The shared Sun FCode
writes the BT467 command at DAC address 9; the selected BT458 does not decode
that register, so this write has no effect.

The drawing engine implements Boolean raster operations, foreground/background
selection, pixel/plane masks, 16×16 patterns, clipping, font writes with automatic
increments, and overlapping screen copies. RASTEROP's two-bit selectors preserve
their previous value when written as zero; its truth table always loads.

Indexed point, line, triangle, quadrilateral and rectangle writes update the
same four X/Y vertex registers. The MISC address index advances after each X
write and also after rectangle Y writes. Absolute writes add the raster offset
at load time; relative writes add the previous indexed coordinate. Direct
coordinate writes and FONT use screen coordinates without adding that offset.
Coordinates remain available for readback and context restoration.

Reading DRAW submits the current vertices and attributes. Coordinate writes
only prepare state; reading plain STATUS never submits work. Lines and points
render in overlapping mode and obey the clip window. Their midpoint ties round
toward screen right/down. Nonoverlapping fills exclude right/bottom edges;
triangles and quadrilaterals with two active edges per scanline are supported.
Overlapping fills combine the interior and balanced boundary lines before
applying the ROP, so XOR touches each covered pixel once. Rasterization work is
bounded to the visible raster even for offscreen coordinates.

Operations complete synchronously and report idle. Full geometric status and
exception classification, indexed Z/TEC clipping, picking, linear rasterops,
polygons with more than two active edges per scanline, TurboGX+ double buffering,
other resolutions and little-endian framebuffer mode remain unsupported. The
buffer selectors share the single store in this 1 MiB profile. TEC register
storage is retained; transformation arithmetic remains unmodeled.

Timing/oscillator programming is retained, but scanout currently has a fixed
1152×900 raster using MAME's 105.561 MHz / 1472×943 timing. It does not synthesize
arbitrary monitor modes from programmed counters or model the oscillator's
analog behavior. Vertical blank derives from guest time, latches its IRQ and
routes enabled requests through SBus level 5 (CPU level 9); writing the pending
bit acknowledges it. This is functional timing, not measured TurboGX throughput.

The optional keyboard attaches to channel A of the SCC at `0x71000000`.
It identifies as type 4, layout `0x21` (Type 5 US), and handles reset, layout,
LED, bell and click commands. Make codes are Sun key stations; releases set bit
7 and the final release also sends idle `0x7f`. Reset reports a held key when
present. A 16-byte FIFO applies backpressure without dropping transitions.
Decoded 8N1 bytes are delivered at 1200 baud using guest time. This transport
retains bytes while the SCC FIFO is full/disabled; electrical overrun, serial
waveforms and keyboard/mouse SCC interrupt generation is shared through the SCC receive path.

## Evidence and references

The original ROMs reached the graphical `ok` prompt with 128 MiB RAM and
evaluated `1 2 + .` as `3`. This establishes the graphical firmware console;
it does not establish OS boot or full CGsix compatibility. Firmware hashes and
the exact probe conditions are in [firmware.md](firmware.md#graphical-obp-console).

Solaris 2.6 has also been checked from a copied installed disk with the original
SS-5 and TurboGX ROMs. Graphical login, the CDE desktop, File Manager and
overlapping xterm windows render with intact decorations and backgrounds.
Closing the covering terminal restores the exposed File Manager contents;
OpenWindows `xrefresh` repaints the desktop cleanly. These checks cover the
observed Xsun drawing paths, not every CGsix mode or X application. Mouse
transport timing remains a separate limitation.

- [Sun GX FBC Reference Manual 3.0, January 1988](https://bitsavers.trailing-edge.com/components/lsiLogic/sparc/GX/Sun_GX_FBC_Jan1988.pdf): section 3.2/table 3-1 (visually checked) defines indexed vertex updates; sections 5.1–5.2 define DRAW submission and coverage; sections 7/9/10 define font coordinates, ROP selectors and the address index.
- [Linux v6.12 CG6 driver](https://github.com/torvalds/linux/blob/v6.12/drivers/video/fbdev/cg6.c): FHC/THC/FBC/TEC offsets, mode fields and independently observable rectangle/font/blit endpoint conventions.
- [MAME CGsix](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/sbus/cgsix.cpp) and [register definitions](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/sbus/cgsix.h): option-ROM hashes, family layout, ROP truth tables, cursor and raster timing. MAME's incomplete FBC alias at `0x300000` is not used: that is FHC on the actual card. Its commented-as-untested drawing loops are not a correctness oracle.
- [LSI L64845 SGX technical manual](https://bitsavers.org/components/lsiLogic/sparc/GX/M14015_L64845_SGX_SBus_Graphics_Accelerator_Technical_Manual_1992.pdf), chapters 2/4/6 and appendix C: single-chip graphics architecture, SBus byte lanes, monitor sense and BT458. This SGX-family document does not specify every later TurboGX register.
- [Original Sun FCode display-driver example](https://docs.oracle.com/cd/E19957-01/801-5123-10/fcode.DisplayDev.html): firmware discovery/display integration. The unmodified TurboGX ROM is the execution target; no replacement driver is installed.
- [SPARC International Keyboard Specification, version 1](https://raw.githubusercontent.com/DirtBagXon/sunkbd/main/datasheet/KBD.pdf), section 3: scan codes, command protocol, FIFO and serial framing. [MAME's Sun keyboard](https://github.com/mamedev/mame/blob/ed59c15fe78596ec6ff8fe04482ede78e6122e34/src/devices/bus/sunkbd/hlekbd.cpp) supplies the Type 5 layout and additional navigation stations; [QEMU ESCC](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/char/escc.c) is an independent protocol comparison.

The implementation and firmware-free tests are newly written Rust; no reference
implementation, font, firmware or framebuffer screenshot is bundled.

## SS-20 built-in SX/VSIMM

`helios-ui --machine ss20 local/firmware/ss10-20-v2.25r-rom --nvram local/ss20-ttya.nvram`
uses the built-in SX framebuffer path instead of the SS-5 TurboGX card. The
VSIMM/SX model exposes an 8 MiB framebuffer mirrored through the aperture,
SX registers/control and CLUT pixels. With monitor sense 4 and `00BBGGRR` CLUT
decoding, original OBP opens `cg14` and draws a native 1152x900 `ok` console.
The serial monitor remains available; SX acceleration instructions and the
full raw/CBGR/component-plane decoder are not implemented.
