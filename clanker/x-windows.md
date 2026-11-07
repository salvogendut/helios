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
- The X server expects a Sun Mouse Systems device on SCC channel B. Helios now
  emits five-byte packets at 1200 baud (`0x80|~buttons`, dx, dy, 0, 0), and
  Heliacal forwards relative mouse motion and left/middle/right buttons while
  captured. Focus loss and capture release send button releases.

The probe did not interact with the desktop login; mouse/login behavior still
needs an interactive frontend check. Do not claim full X desktop, applications
or audio support from this startup evidence.
