# TurboGX and keyboard console milestone

- The user's target remains SS5/85, 128 MiB RAM, 1 MiB TurboGX. GUI is the
  separate `helios-ui` crate, with its own lockfile and pinned minifb 0.27 X11
  backend. Root crate is still dependency-free and headless. Do not tie device
  time to window refresh or add a host-rendered firmware console.
- Original SS-5 ROM has no built-in CGsix driver. The original 501-2325 FCode
  ROM is essential. `tools/fetch_firmware.py turbogx` verifies the MAME hash
  against a pinned archive. Firmware stays ignored under local/.
- Physical `0x30000000` is OBP SBus slot **1**, not slot 3. FHC is at +300000;
  FBC is +700000. MAME's partial alias is not a correct FHC implementation.
- TurboGX FCode programs extended THC +90/+94/+a0..b0/+c0..cc and the alternate
  oscillator latch. Linux's old THC timing layout alone is insufficient.
  Monitor sense bits select 1152×900. Programmed timing is retained but the
  functional raster is fixed; arbitrary modes and the analog oscillator are
  not implemented.
- FCode writes FHC reset `8000`, then `01ff`, including when video is already
  on. Reset the drawing engine without erasing VRAM or disabling scanout.
  It restores the resolution/pitch before drawing. Rejecting intermediate
  FHC resolution bits as soon as they are written prevents normal startup.
- The shared Sun FCode writes DAC control address 9 for BT467. The 1 MiB
  TurboGX has BT458; this particular write is not decoded and has no effect.
- Rectangle NONOVERLAP upper edges are exclusive; blit endpoints are inclusive.
  Linux cg6_fillrect/cg6_copyarea are useful independent evidence. MAME's drawing
  code explicitly calls itself untested; do not copy its off-by-one loops.
  Source snapshotting preserves overlapping scrolls, with bounded geometry.
- Sun keyboard responses are serialized at 1200 baud. An immediate ff/04/7f
  bundle can be lost during SCC initialization. A 16-byte keyboard FIFO and
  retryable host input preserve releases under SCC backpressure. The final
  release sends 7f. Type 5 identifies as type 4, layout 21.
- Keyboard presence changes the firmware's initial POST path. Do not infer
  that serial-only diagnostic gaps are fixed because a graphical boot skips
  them. Fresh NVRAM still enables diagnostics and automatic boot.
- The graphical 128 MiB proof completed memory testing, sent RARP, accepted
  Stop-A and displayed `1 2 + .` / `3` / `ok`. The native frontend must be run
  with desktop access; sandboxed XOpenDisplay fails in this environment.
  Its error path retains the last frame until the window is closed.
- Optional saved NVRAM for desktop testing was configured with real keyboard
  commands `setenv auto-boot? false` and `setenv diag-switch? false`, then
  exported; no firmware/NVRAM bytes were patched to bypass initialization.
- Local evidence: `local/probes/graphics-ss5/`. Human documentation and primary
  source links are in `docs/graphics.md` and `docs/firmware.md`.
- Final 128 MiB probe: 1061371506 steps; ordinary instructions 1061361284,
  sync traps 3, IRQ entries 10218, software resets 1. Full framebuffer shows
  `3`, accepted auto-boot?/diag-switch? settings and another `ok`.
- User exercised the live native window. `probe-scsi-all` stopped at
  `ffd16898`, ESP command address `7880000c`: selection/data transfers remain
  unsupported. Frontend title now exposes the reason rather than only saying
  to check the terminal. Do not claim this SCSI probe is implemented.
