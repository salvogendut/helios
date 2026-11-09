# Solaris X rendering

Updated 2026-11-08 on `x-windows`, based on 92a05d3. Worktree:
`/tmp/helios-x-windows`; the shared checkout is on `ss20-axil320` with unrelated
uncommitted work and must not be switched or overwritten.

## Root cause and primary evidence

The earlier line/rectangle queues, sticky relative dimensions, eager rendering
on coordinate writes and unconditional line clipping bypass were incorrect.
They mixed distinct primitives, applied drawing attributes at the wrong time,
and left extra fills or missing decorations. Tests asserting those behaviors
were replaced with hardware-derived expectations.

The primary source is **Sun GX FBC Reference Manual 3.0, Curtis Priem, January
1988**, downloaded from the Bitsavers mirror linked in docs/graphics.md.
Local PDF/text: `/tmp/helios-fbc.pdf`, `/tmp/helios-fbc.txt`. Page 8/table 3-1
was inspected visually (`/tmp/helios-fbc-index.png`). Relevant sections:

- 3.2: indexed aliases update the common X0/Y0 through X3/Y3 bank. Point writes
  replicate to four vertices, lines to three, triangles/rectangles to two,
  quadrilaterals to one. X advances the MISC index; rectangle Y advances too.
  Absolute coordinates add RASTEROFF at write time; relative coordinates add
  the previous indexed coordinate. Direct coordinates restore context.
- 5.1–5.2: DRAW reads submit the current bank. Plain STATUS does not draw.
  Overlapping polygons use balanced boundaries plus interior; nonoverlapping
  coverage excludes right/bottom edges and gives degenerate lines no area.
  Ties round to screen right/down. Lines also obey the clipping window.
- 7/9/10: font coordinates are already screen coordinates; RASTEROP two-bit
  selectors ignore zero writes, while the truth table loads normally.

The retained generic bank removes all primitive-specific pending queues.
Triangle/quadrilateral scan conversion handles two active edges per scanline,
including Xsun's CDE clock/decorations; unsupported four-edge cases stop before
changing VRAM. Overlap coverage is collected before the ROP, preventing double
XOR on shared vertices or edges. Raster loops are bounded to the screen.
Full geometric/exception status, indexed Z and TEC arithmetic remain pending.

## Reproduction and validation

Evidence directory: `/tmp/helios-xw-evidence`.
Durable captures and logs are under the shared repository's ignored
`local/probes/x-windows-fbc/`; guest disks and ROMs are not included there.
`base` is an instrumented copy of 92a05d3, capturing CGsix writes and DRAW/BLIT
reads to `cg6.bin` (16-byte little-endian records: operation, offset, value,
mask/width). The private source is `/tmp/helios-cg6-baseline`; none of its trace
instrumentation is included in the fix. The replay tool is in the ignored
external `harness` package and applies records through the real SystemBus API.
It reproduced the broken dtlogin and CDE images, then rendered the same traffic
correctly with the fix. Replaying record 573088 first exposed a genuine Xsun
quadrilateral/triangle, which motivated the polygon rasterizer.
The final replay consumed all 811,624 records without an unsupported operation.

`final` boots the fixed core independently from original SS5 OBP/TurboGX ROMs
with 128 MiB RAM, a copied Solaris 2.6 installation and the configured NVRAM.
Both graphical root login and serial root login use the existing empty local
test password. CDE, File Manager and an overlapping xterm show intact borders,
window controls, icons and backgrounds. Source disks/ROMs remain untouched;
each running machine owns a separate writable copy under the evidence tree.
Large injected mouse bursts in the baseline triggered a pre-existing
`zs3:ring buffer overflow`; no mouse code was changed. Guest X clients are also
controlled through the serial root shell for expose/repaint checks.

The second test xterm (`HELIOS-expose-check`, geometry `60x15+360+260`)
covered portions of both File Manager windows and the desktop. After closing
it, the covered icons, frame edges and background were restored. A subsequent
`DISPLAY=:0 /usr/openwin/bin/xrefresh` returned successfully and produced a
clean desktop. Durable image names are `before-login`, `before-desktop`,
`before-terminal`, `after-login`, `after-desktop`, `after-overlap`,
`after-expose` and `after-refresh` (PPM originals plus PNG conversions).
Both probe guests were synced and halted to OBP before stopping their host
processes. No mouse-drag claim is made from these tests.

Tests cover all indexed destination masks/indices and relative bases, command
submission/context restoration, ROP retention, clipping, midpoint ties,
triangle tiling/XOR coverage, quadrilateral scanlines in both windings
(including crossed edges) and failure atomicity.
27 CGsix tests; 412 total core tests and 4 UI tests passed. Both format/Clippy
checks, release builds and all four built-in demos passed. The full core suite
needs loopback bind permission for its existing network/serial-host tests;
the restricted sandbox initially rejected the two UDP/DNS test sockets, and
the suite passed with host loopback access.

Earlier startup work (retained): full TEC register storage, FSQRTd, Type 5
keyboard and the Sun Mouse Systems endpoint. These limited checks do not
establish full graphics, mouse timing or audio support.
