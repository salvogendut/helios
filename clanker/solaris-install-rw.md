# Writable disk and Solaris installation

The user explicitly authorized adding a read/write disk and attempting
installation. They selected **Solaris 7 first**, on a new 2 GiB image, then
Solaris 2.6 on a separate image. Installation choices on these newly created
disks are authorized; the former stop-at-language-menu restriction no longer
applies. Original CD images remain read-only. Never overwrite the user's ISOs.

Previous milestone `c813968f9ceb1f609566800b79e81d8dd61d91a1` is committed and
pushed; all four CI jobs passed at
https://github.com/salvogendut/helios/actions/runs/42351036093.
Writable-disk work is subsequent and currently uncommitted.

## Implementation

- `ScsiBus::attach_disk_rw` explicitly opens an existing regular raw disk with
  read/write access, never creates/truncates it. Existing `attach` stays read-only.
- Both CLI/UI accept mutually exclusive `--disk` and `--disk-rw` at ID 3.
  CD-ROM ID 6 remains read-only. No implicit image conversion or overlay.
- WRITE(6)/(10) stage complete DATA OUT before touching the image. Partial
  command/reset/DMA failure cannot commit partial payload. Valid writes use
  write_all plus sync_data before GOOD (including FUA); host errors return
  MEDIUM ERROR/WRITE ERROR. Host I/O failure during commit may leave partial data.
- SYNCHRONIZE CACHE(10) validates range then flushes; IMMED/RelAdr unsupported.
  MODE SENSE's disk WP bit follows the attachment. CD's inherent read-only
  behavior stays separate from the disk WP bit. Fixed 512-byte disk blocks.
- Bounds use u64; zero WRITE(6) count=256, zero WRITE(10) count=no-op.
  No geometry/mode pages yet; extend only with evidence from the installer.

355 core tests and four UI tests pass, with both Clippy checks and release
builds. The new tests cover PIO/DMA persistence/reopen, residuals/FUA, abort and
DMA error, host resize/write failure, count/bounds/flush semantics, read-only
protection, and CLI exclusivity/no-create/no-truncate behavior. Run required
demos and final checks after any later fixes.

## Live installation harness

- Disk: `local/disks/solaris7-install.raw`, **2147483648 bytes**, created with
  Python `open(..., 'xb')` then truncate, initially sparse/blank. This is the
  new authorized installation target; preserve its contents across probes.
- Firmware/NVRAM/media same as preceding checkpoint. Target 3 writable disk,
  target 6 read-only `local/media/SOL_7_1199_SPARC.iso`.
- Ignored runner `local/probes/solaris-install/probe.rs`, compiled to `probe`.
  Sends `ttya io` by keyboard at 60M, `boot cdrom -v` by TTYA at 70M. Cap 100B
  steps; stops on host error or guest panic, no automatic language-menu stop.
- Running exec session initially **60876**. Output is redirected to
  `local/probes/solaris-install/sol7.log`; raw serial is `sol7-serial.log`.
  Feed bytes by **appending** to `sol7.input`; the running probe retains a read
  offset and polls it every 10k steps, then respects the SCC receive FIFO.
  For example Python `with Path(...).open('ab') as f: f.write(b'0\r')` selects
  English only when the actual menu is visible. Never truncate the input file.
  Do not restart the same harness name without archiving/clearing old commands
  deliberately: it starts reading command-file offset zero.
- Read progress via tail or Python; strip spinner pairs `.\x08` from raw
  serial for basic text. Curses screens may need VT100 rendering later.
- Solaris sees sd3 and reports its blank/corrupt label and 4194304 512-byte
  blocks. This is expected before partitioning. It mounted CD UFS root and
  reached the actual text installer. English (0), default ASCII locale (0),
  and DEC VT100 (3) were selected, then the introduction/identify screens.
- `screen.py` renders the raw serial stream as a 24x80 VT100 text screen.
  Key sequences: Continue `ESC 2`, Change `ESC 4`, Down **`ESC O B`**, Up
  `ESC O A`, Space selects. The terminal is in application cursor mode;
  `ESC [ B` is not recognized. This caused an initial Yes networking selection;
  a temporary documentation IP was entered only to reach the review page, then
  Change restarted identification. It has now explicitly confirmed hostname
  **helios** and **Networked: No**. No network settings were accepted earlier.
  It is proceeding to timezone/date setup. No partitioning or install yet.

Installation defaults can be English, standalone/no networking (no host
network backend), hostname helios, and Europe/Rome timezone. Do not configure
real host networking or expose credentials. Record actual installer progress
and limitations; a disk write test alone is not installation evidence.

## Identification checkpoint and integer conversions

The first run completed system identification: helios, **Networked: No**,
MET timezone (Europe/Rome filename absent on this media), 2026-11-07 date.
It stopped at step 5894059630, PC 0xef6eacb0, instruction 0x81a01904
(FITOD f4,f0). Disk remains blank, sparse with no allocated blocks.
Added FITOS/FITOD with signed integer bit arithmetic, all RD modes and NX
deferred handling. Five conversion tests cover limits, ties, carry,
overlapping registers, traps and unsupported quad/EF priority.
The retry is `sol7-r2`, exec session 44974, empty new command input file.
The probe now logs FPU state on host stops. Earlier sol7 logs are retained.

After FITOS/FITOD, **360 core tests**, 4 UI tests, both Clippy checks, all
four demos and both release builds pass. The retry's identification is fed by
ignored `identify.py` (session 19518), which waits for stable real terminal
screens and stops after confirming hostname/no networking. It never preloads
inputs during boot. Continue timezone and installation manually afterward.
QEMU pinned fop_helper.c was fetched to /tmp/helios-qemu-fop_helper.c with curl
(network escalation was approved) and signed conversion helpers checked.

## Second conversion stop

Run sol7-r2 reached system identification completion, then stopped at step
2441294476, PC 0x00019ca0, instruction 0x85a01a40 (FDTOI f0,f2). FPU f0/f1
contained exactly 1.0. No disk layout/write had happened. Added FSTOI/FDTOI,
FSTOD/FDTOS with integer bit arithmetic. Primary V8 B.34/N.4-N.7 and microSPARC-II
figure 4.9 (PDF page 55, printed 56, visually checked) define semantics.
QEMU round-to-zero helpers agree with V8; MAME's generic rounding argument
is not used as the reference. Ten directed conversion tests now pass.

Current retry: **sol7-r3**, exec **51175**, identification driver **19381**.
Driver now handles language/locale/terminal, hostname/no networking, MET and
current Europe/Rome date through confirmation; stops before disk setup.
Probe saves `sol7-r3-nvram.bin` every 10M steps for later disk-boot runs.

After all six conversions: 365 core tests, 4 UI tests, Clippy/release builds
pass. `local/probes/solaris-install/conversion-check.py` generates **14400**
finite cases (seed 0x535335, 512 sampled raw words per conversion plus edges,
all RD modes and narrowing UFM variants). Fraction oracle binary-searches
numeric encodings for adjacent representable values, independently of core
shift/rounding. Rust checker includes numeric helper modules, checks bits/flags;
CPU integration/deferred state/NaNs are covered by directed integration tests.
All oracle cases pass. `conversion-oracle.txt` retains the inputs/results.

Run r3 passed both prior conversion stops. At 4.19B steps, serial still says
system identification completed; successful new CD READ(10) requests continue,
no host error and no writes to disk. Inspection of original ISO files with
isoinfo shows /sbin/sysconfig runs parse_dynamic_clustertoc before suninstall.
The dynamic catalog has 1693 lines/301 conditional entries, each potentially
spawning utilities; this is a plausible explanation for quiet CPU-heavy startup,
not yet proven live process identification. Source scripts extracted read-only
to /tmp/helios-sol7-{sysconfig,suninstall,parse-clustertoc,clustertoc.dynamic}.
Framebuffer snapshot is black (serial console); no hidden graphical dialog.
Probe source now buffers PPM writes; running r3 binary still uses the older
unbuffered dump. Compile buffered source on next retry if needed.

## Real disk label written

Catalog processing completed near 5.9B steps. Solaris ran preinstall scripts
and ttinstall, which warned disk found but hardware failure/unformatted. After
Esc-2 it exited with caught signal 11 to a working recovery shell. Root cause
of that guest error is not yet established. `format` -> disk 0 -> Auto configure
requested MODE SENSE(6) page 3 and got CHECK CONDITION (missing page), then
reported Auto configure failed. Added fixed disk pages 3/4, read-only changeable
masks, complete MODE SELECT validation and two regressions (367 core total).
**Running r3 predates those geometry changes**, so live auto configuration with
new code is not yet verified. Geometry fields follow SCSI-2 tables 163/171,
checked against pinned QEMU scsi-disk.c.

In the live guest, format -> type -> other defined HELIOS 2GiB:
4094 data cylinders, 2 alternate, 4096 physical, 16 heads/physical heads,
64 sectors/physical sectors per track, 5400 RPM, other fields default.
`partition -> print` showed root64MiB, swap128MiB, whole slice2, usr1.81GiB
(3799040 blocks). `label` -> yes wrote primary/backups via real SCSI writes.
`quit -> verify` read back the same geometry and map. Host read of sector0
confirms label magic **dabe**, XOR of 256 BE words **zero**, allocated disk
storage **12288 bytes**. Initial label preserved under ignored
`local/probes/solaris-install/sol7-first-label.bin`.
Exited format and ran **suninstall** again in the SAME session, which avoids
repeating catalog preprocessing. Installation has not yet copied files.

## Package installation started

The rerun accepts c0t3d0 and enters Solaris Interactive Installation.
Selected initial install, no client services, English, no 64-bit packages,
default **End User System Support**, c0t3d0 (2047 MiB usable), no preservation,
auto-layout default filesystems and no remote mounts. Final layout:
root s0 90 MiB, swap s1 147 MiB, usr s6 380 MiB, export/home s7 1429 MiB.
Selected **Manual Reboot** and Begin Installation.
At ~11.55B steps, newfs/initialization completed and the actual package progress
screen appeared: 0.00 MiB installed, 260.27 MiB remaining, installing X Window
System & Graphics Runtime Library Links in /usr/lib. Disk host allocation
~38 MiB; actual target 3 WRITE(6) DATA OUT/GOOD traces present. Continue the
installation; do not claim completion or installed boot yet.
Final code checks so far: 367 core tests, 4 UI tests, fmt/Clippy, both release
builds and all four demos pass. Original source firmware/ISO hashes unchanged.

Parallel independent geometry validation is running as exec **34478**:
`geometry-probe.rs`/binary from latest release, separate sparse
`local/disks/geometry-test.raw`, name `geometry`, input `geometry.input`, serial
`geometry-serial.log`, log `geometry.log`. It boots **cdrom -s** into maintenance
mode using buffered PPM output. NEVER attach the live installation image to
this second machine. Goal: guest format Auto configure -> label -> verify
using the new MODE SENSE pages, then terminate this helper.
Main installer remains original exec51175 with sol7-r3 input/logs.

Main r3's unbuffered full-frame dump every 10M steps was doing >1M tiny
host writes per snapshot despite the all-serial install. Its own /tmp progress
PPM was preserved as helios-sol7-r3-serial-only-frame.ppm and the progress path
redirected to /dev/null during the live run. This changes only host diagnostics;
error/entry dumps still use distinct real files, CPU/devices untouched. Geometry
probe already uses BufWriter. Clean the progress symlink after r3 terminates.

## Automatic geometry verified in real Solaris

Geometry helper **34478 ended** after success. Original OBP boot cdrom -s
reached `INIT: SINGLE USER MODE`/root shell. `format` automatically printed
`c0t3d0: configured with capacity of 2.00GB` and selected
`HELIOS-SCSIDISK-0001 cyl 4094 alt 2 hd 16 sec 64` without manual type entry.
Selected disk0, answered yes to Label it now, and `verify` read back physical
4096/data4094/alternate2/head16/sector64. Host label magic dabe, checksum XOR0.
New MODE SENSE pages and durable writes are now validated end to end with the
latest compiled code. Logs retained under geometry*. Main installer unaffected.

At ~19.53B steps main installation reports 62.02 MiB installed / 198.25 MiB
remaining (X Window System platform software). Core /usr, root and devices
packages completed. The devices package's postinstall script takes several
billion steps creating device nodes and updating the package database; new disk
writes continued. No failed target3 commands after Begin Installation were
found when checking at ~14B steps; both WRITE6 and WRITE10 are exercised.

Next: wait for completion, inspect real installer result/logs, then test the
installed disk with the latest compiled core. Manual reboot was selected.
Safely stop main probe only after guest sync/halt or a clear error, preserve
NVram output, and remove/restore its /tmp progress-PPM symlink to /dev/null.
For a fresh disk boot: use original OBP/CGsix, saved NVRAM, ID3 rw image and
**no attached CD**; switch console with keyboard `ttya io` at60M as usual,
then send serial `boot disk -v` at70M. This proves boot/root from the disk.
A dedicated ignored disk-probe.rs can adapt probe.rs for these choices, with
new empty input/log names. Do not attach one rw image to two live machines.
Check a real serial login/shell, uname/release and root filesystem. If root
password setup is requested, a fresh eight-character legacy-compatible secret
can be stored in ignored local/ with mode0600; chmod the probe input file0600
before feeding it, never print the secret. No password has been set so far.
Keep final claims limited to observed installation/boot results. New RW/FPU/
geometry work remains uncommitted; previous CD milestone c813968 is pushed.

Latest-core disk boot harness is compiled, not started:
`local/probes/solaris-install/disk-probe NAME DISK_PATH NVRAM_PATH`.
Create a new empty NAME.input first. It attaches only ID3 rw, no CD, uses the
provided saved NVRAM, switches ttya io at60M then boot disk -v at70M. Buffered
frame diagnostics, 100B step cap, live input polling/logging as before. Wait
for installation completion and stop main51175 before opening its image here.
Main installation passed 92.23 MiB / 168.04 MiB remaining (~35%).
