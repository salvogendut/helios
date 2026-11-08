# Solaris X server startup

Date: 2026-11-07. The local Solaris 2.6 installed disk boots with Helios and
starts `dtlogin` on the TurboGX console. A headless `disk-probe` run reached
`Starting Desktop Login on display :0...` and produced a nonblank 1152x900
framebuffer; raw serial is `local/probes/solaris-install/sol26-x5-serial.log`.

Observed missing behavior and fixes:

- The X server writes the TEC page at `0x701000`, including offsets beyond the
  three documented `cg6_tec` words (`0x701100` was the next stop). The whole
  4 KiB page is now retained and read back. TEC transformation arithmetic is
  still not modeled; only register storage is needed for the server's setup.
- The X server programs absolute line endpoints through FBC `0x840/0x844`, and
  fills rectangles through absolute `0x900/0x904` plus relative `0x910/0x914`.
  Both paths are implemented; line drawing uses a Bresenham walk through the
  existing ROP/pattern/clipping logic. Relative rectangles use inclusive
  `x + width`, `y + height` extents.
- `FSQRTd` is used during server startup. Exact integer square-root rounding,
  every rounding direction, negative-input NV, NaN and zero handling are
  implemented with directed tests.
- Xsun reuses the FBC relative-rectangle width/height while changing only the
  absolute origin. The first implementation cleared relative mode after each
  draw, so a later draw saw one queued absolute point and stopped with
  `CGsix incomplete rectangle`. Relative mode now persists until a line
  operation, and line draws take precedence over pending rectangle state.
  Later desktop redraws queue multiple absolute rectangle pairs while the
  relative state is still set; an even nonempty vertex queue now takes
  precedence over relative mode. Without that precedence, window backgrounds
  were skipped and desktop elements overwrote each other. Dragging a window
  queues more than sixteen vertices, so the rectangle queue drains on `draw`
  rather than reporting FIFO full. Xsun also uses `apoint` plus `arectx` for
  horizontal spans and connected/relative lines; without those paths, 3-D
  borders, arcs and many window details were missing. Unpaired line points and
  rectangle vertices are now discarded when a new sequence starts; otherwise a
  stale frame endpoint could join the next repaint and leave magenta lines
  across window contents. Complete absolute rectangle and non-anchored line
  pairs now execute on their second point write, so title-bar fills cannot be
  deferred past the bevel lines that frame them. Line drawing ignores the fill
  clip rectangle: Xsun left a text-row clip in place while drawing the vertical
  sides of the dtlogin OK/Start Over/Options/Help buttons, so the buttons were
  missing their sides.
- Heliacal originally forwarded SDL's down-positive Y delta directly; Sun's
  mouse convention is up-positive, so the frontend now negates Y.
- The X server expects a Sun Mouse Systems device on SCC channel B. Helios now
  emits five-byte packets at 1200 baud (`0x80|~buttons`, dx, dy, 0, 0), and
  Heliacal forwards relative mouse motion and left/middle/right buttons while
  captured. Focus loss and capture release send button releases.

Mouse motion was confirmed interactively. A headless keyboard/mouse injection
at the login screen then exercised the previous root-login crash; after the
relative-rectangle fix it ran without a host stop. Password/login completion
and applications still need a user-side check. Do not claim full X desktop or
audio support from this startup evidence.
