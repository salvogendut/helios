# SS-20 desktop milestone completion

Takeover on 2026-11-13 after the independent Run 21 review. Local transcripts,
private disk clones, harness sources and screenshots are retained in
`local/completion-2026-11-13/`. Source firmware and installed disks were not
opened for guest writes.

## Core corrections and validation

Unaligned VRAM reads/writes now validate their whole byte span before
splitting it, and the SX store-vector prepass validates every element's full
width before any mutation. `tests/unaligned_stores.rs` promotes the review's
four fixtures and adds a direct-aperture boundary case. All five pass,
including legal raw/BGR unaligned writes with preserved guards/X bytes.

The final core suite has 499 passing tests. Formatting, Clippy with
`-D warnings`, and all four built-in demos pass. Network tests require local
loopback sockets outside this environment's restricted sandbox.

The Rust 1.74 CI pass flagged the old lazy aperture-offset closure. The
lookup now uses checked subtraction before the upper-bound test, preserving
rejection below the aperture without eagerly evaluating a subtraction that
could underflow. This also satisfies the older Clippy version.

The separate `helios-ui` CI target revealed a compile failure: the new shared
MXCC `Rc<RefCell<_>>` makes `Machine` thread-local. Constructing the machine
inside the worker fixes the transfer without adding synchronization to the
CPU hot path. A startup channel propagates construction errors to the UI.
Fresh NVRAM now comes from the selected profile. Its four tests, formatting,
Clippy and release build pass. An original-OBP SS-20 smoke run reached the
monitor with no disk attached and closed normally via WM_DELETE_WINDOW.
On the isolated Xvfb display, minifb needs `XMODIFIERS=@im=none`.

Heliacal's 22 tests, formatting, Clippy, SDL smoke test and release build
pass. The native-SX paused-screen hint no longer requests a TurboGX ROM.

## Fresh disk regressions

- SS-5: a private clone of `local/disks/solaris26-install.raw`, original
  OBP 2.15 and TurboGX ROM. Switch the saved graphical console using actual
  Sun keyboard events for `ttya io`, then `boot disk`. Solaris 2.6 reaches
  login, accepts root, prints `uname -a` and `HELIOS-BOOT-ACCEPTED`, and
  `sync; halt` returns to `ok`. Final evidence: `ss5-final.serial` and
  `ss5-final.log`, 1,323,000,000 steps through clean halt.
- Axil: a private clone of `/tmp/solaris26-clone-axil.raw`, original Axil
  2.19 firmware and TurboGX. Login/root/`uname -a` pass. The prior persistence
  test remains `3034423527 17 /helios-disk-test`. Clean halt at 1,288,500,000
  steps; `axil.serial` and `axil.log`.
- SS-20: the Run 21 disk clone boots to serial root and cleanly halts with
  the current core. Original 2.25R firmware, native SX, 128 MiB and target 3.

The bounded harness checks `Step::TrapTaken` for graphics queue faults,
checks error stops, and polls newly appended serial data in batches. It does
not scan the entire transcript on each guest instruction. Intermediate
aborted attempts are not the accepted SS-5 evidence.

## Heliacal input acceptance

The graphical-console handoff exposed missing VSIMM identity. The old model
returned MDI revision 0 and zero VBC configuration while exposing 8 MiB VRAM.
Xsun stalled after MDI_GET_CFGINFO, entering the revision-0 mode-change path
with video already enabled on the console. Sun's driver queues that change
and waits; the serial-console experiment had not exercised the same path.

The selected board is now a coherent VSIMM 2: MDI revision/implementation
0x11, VBC version 1 and HIRES at +0x20c (0x2400). The fields are read-only,
including mixed-width accesses, and reset restores them. Video power and
active-low blanking now match that revision; power-off stops new retrace
requests. Three tracked tests cover identity, byte lanes, power/blanking,
VRAM preservation and interrupt behavior. Sources are Sun cg14reg.h and
cgfourteen.c 1.112, corroborated by NetBSD cgfourteenreg.h 1.9 and driver 1.102.

The corrected frontend boot completes MDI_SET_PIXELMODE(32), as recorded in
`xtrace-vsimm2.txt`, and starts olwm/xterm. On an isolated Xvfb display with
SDL's X11/software renderer, actual XTEST events pass through the frontend's
SDL event loop. They are not injected directly into the emulator core.

- A normal graphical-console boot accepts the root login through SDL typing.
- Xterm prints `HELIACAL-INPUT-OK` from typed input.
- Ctrl+Enter releases capture. Typing `echo SHOULD-NOT-REACH-GUEST` while
  released leaves the existing text rows byte-identical and adds no command.
- Recapture and typing produce `/heliacal-input-test` containing
  `CAPTURE-RECAPTURE-OK`; read-only UFS inspection confirms the stored bytes.
- Paced mouse startup, positioning and left-button title-bar dragging move
  the client from `484x316+105+125` to `484x316+125+125`. Guest `xwininfo`
  independently confirms the geometry. Comparing the entire translated
  484x316 client gives **zero pixel differences**.
- Typing `sync; /usr/sbin/halt` through xterm completes filesystem sync and
  returns to a visible graphical OBP `ok`. `frontend-halted.png` records it.

The first shorter drag missed acceptance; the successful gesture uses a
calibrated title-bar point and one-second intervals, matching the core
harness's guest pacing. `gui-actions.log`, `gui-metrics.txt`,
`input-verification.txt`, `geometry2.txt` and native PPMs retain the evidence.
The test-only rc2 script starts X before the console login message; `xrefresh`
clears that console output during acceptance. It is not part of the emulator.

## Final desktop replay

The full Run 21 workload was repeated on the corrected VSIMM-2 model using a
fresh private clone. The copied harness changes only artifact paths and turns
off verbose SX tracing. All geometry and pixel assertions pass again:
20/1/2/3-pixel moves, terminal uncover, Full Size/Normal Size and
iconify/restore. The guest then cleanly halts to serial OBP. Evidence is
`desktop.log` and `desktop/accepted*` in the completion directory.

The measured shrink remainder is still exactly the surviving three-row
window border described in the earlier review. Complete SX instruction
coverage, arbitrary display timings, non-Linux frontend hosts and full POST
remain outside this accepted desktop workload.
