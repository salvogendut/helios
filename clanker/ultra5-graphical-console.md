# Ultra 5 graphical OpenBoot acceptance

Two cold power-ons with original OBP 3.31.0, a 512 MiB Ultra 5 and an attached
Type 5 US keyboard reached the graphical prompt. The Heliacal X11/software
test captured SDL input, typed `1 2 + .`, pressed Return and captured the
guest's `3` and second `ok` from the 1152×900 Mach64 framebuffer. A separate
F9 run selected the Ultra 5 from a new SS-5 settings draft, saved it and
displayed the Sun banner. With factory NVRAM defaults that run continued to
diagnostic network boot and stopped at the HME MAC reset.

The two command tests used a copy of the guest-initialized NVRAM with
`auto-boot?` disabled. The ROM hash is
`fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`.
The initial NVRAM hash is
`b32f818e9b392b152199ff7da18d75643656bde53f0de7699cf6b90d60094602`.

The local directory `local/ultra5-graphical-console-2026-11-22/` contains
the before/after guest captures and a separate ready-to-run config and seed.
From the Helios checkout run:

```sh
../heliacal/target/release/heliacal --config local/ultra5-graphical-console-2026-11-22/ultra5.cfg
```

Press Ctrl+P to start, click inside the display to capture and Ctrl+Enter to
release. The core console tests cover CRTC origin and pitch, indexed pixels,
DAC state and paced keyboard input. The graphical checkpoint passed 980 core
test executions and 121 frontend tests, formatting, strict Clippy, the four
core demos, SDL smoke, and the SS-5/TurboGX original-firmware display smoke.

## Solaris 7 11/99 installation

The original SPARC CD mounted its UFS root under the Solaris 5.7 kernel.
`dad0` identified the primary ATA master after choosing multiword DMA mode 2
and read its blank first sector through the CMD646 descriptor table. The
serial interactive installer selected the Solaris 2.7 End User System
Support group with the 32-bit kernel, hostname `ultra5`, English, no network
and MET. The only writable target was the new 4 GiB Ultra 5 IDE unit 0
image. The installer wrote its VTOC with magic `dabe` and zero XOR checksum,
created UFS filesystems, installed the selected packages and wrote boot blocks. The local
`ultra5-solaris7-install/evidence/` directory retains the guest's selection
screens. The original CD is read-only.

An independent graphical CD boot loaded Xsun and its SUNWm64 driver. The
real client drew the Solaris Install Console and the Select Language and
Locale dialog. An initial capture had malformed proportional text. The
driver wrote individual pixels and halfwords through the upper PCI aperture;
byte writes must preserve the pixel address. The corrected framebuffer
is retained as `evidence/cd-xsun-installer.png` in the install directory.


The installer completed its patch scripts, reported a successful installation
and returned to OpenBoot after `sync; halt`. The guest saw the root, /usr
and /export/home slices on disk 0. An independent read of the installed UFS
filesystems matched the package checksums for Xsun and dtlogin. A separate
production Heliacal process cold-booted the disk with no CD attached. The
guest completed first-boot system identification and displayed its Xsun/CDE
username prompt at 1152×900. The local profile is
`local/ultra5-solaris7-install/installed.cfg`.

On the GTB, Solaris set MEM_CNTL to `0x003210b7`. Both aperture endian
fields select bytes, including word and doubleword writes of indexed
pixels. An unconditional dword swap scrambled the login logo in groups
of four while the login text remained readable. A regression now reads
the indexed logo-pattern bytes through the other aperture and checks
independent word/dword settings.

The corrected release reproduced the installed-disk login on a fresh
power-on. All 21,888 pixels of the logo test region matched the expected
ordering. The installed guest also accepted the test account at the greeter
and reached the Solaris CDE 1.3 startup screen. A single-user reboot of the
configured filesystem identified SunOS 5.7 Generic_106541-08 and returned
to OpenBoot with `syncing file systems... done` after halt. A cold
Heliacal boot of a byte-identical copy of that final disk returned to the
CDE username prompt with just `boot disk`; first-boot setup was complete.

The local evidence directory retains separate CD installer and installed
login captures, the installer/halt transcript and the validation counts.
Network connectivity, audio playback and 64-bit Solaris are outside this
installation check.
