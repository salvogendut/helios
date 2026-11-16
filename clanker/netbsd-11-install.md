# NetBSD 11.1: SS-20 installation and CHD acceptance

The supplied NetBSD 11.1 sparc ISO completed a minimal installation onto a new
2 GiB CHD. A fresh host process booted its unmodified GENERIC kernel to serial
root login, verified installed files and guest-written data, ran OpenSSL, and
halted cleanly back to original OBP. This checkpoint accompanies the
`feat/chd-images` implementation in Helios and Heliacal.

## Reuse the installed test disk

- Disk: `local/disks/netbsd11-ss20-scsi3.chd`, writable SCSI target 3.
- Profile: SS-20, one CPU, 128 MiB RAM, no framebuffer, hardware clock mode.
- Firmware: `local/firmware/ss10-20-v2.25r-rom` (original OBP 2.25R).
- Saved serial NVRAM: `local/chd-development/netbsd/disk-boot-final.nvram`.
- Hostname: `helios-netbsd`; console user `root`, empty password.
- Network setup, SSH and xdm are disabled; entropy setup was deferred.
- Layout: 2016 MiB FFS1 root, 32 MiB swap; raw partition c.

The user explicitly requested that this installed CHD be preserved for testing.
Keep it and the saved serial NVRAM during subsequent scratch cleanup.

In Heliacal choose SS-20/one CPU/128 MiB, enable General → Tinker, then
Advanced → Headless Guest and Accurate CPU clocks. Attach the CHD read/write
at ID 3 and use `boot disk` at `ok`. Preserve any existing named machine by
using a separate record. The ISO is no longer needed for the installed boot.
This is a local test installation, not a configured production system.

The accepted boot used the normal GENERIC kernel: no firmware/kernel patches,
USERCONF driver disabling, or boot-time single-user workaround. Keyboard
presence remains the original-OBP workaround tracked in
[#7](https://github.com/salvogendut/helios/issues/7). This acceptance covers the
headless core's SS-20 serial/storage path; it does not establish NetBSD
SMP, graphics, networking, SS-5 or Axil support.

## Evidence and checksums

Evidence root: `local/chd-development/netbsd/` (ignored).

| File | POSIX cksum | Bytes |
| --- | --- | --- |
| `/netbsd` | 3544516170 | 5707996 |
| `/bin/sh` | 1436774852 | 227568 |
| `/helios-chd-test` | 296033067 | 21 |
| `/var/tmp/helios-chd-io` | 1204609605 | 262144 |

Kernel/shell expectations were extracted independently from the source ISO.
The marker contains `HELIOS-NETBSD-CHD-OK` plus newline. The I/O file is the
first 256 KiB of the installed kernel; its expected checksum/hash came from
the source archive. All four matched inside the installer chroot, after a
clean halt and fresh installed-disk boot, and via an independent CHD-v5 map
plus FFS1 reader after the final halt.

- `install-complete-serial.log` / `-core.log`: five sets, MAKEDEV, certificate
  rehash, sysinst completion, explicit sync and clean unmount/halt.
- `disk-boot-serial.log` / `-core.log`: GENERIC, clean root filesystem, root
  login, `uname -a`, all four checksums, OpenSSL 3.5.7, acceptance marker and
  clean halt. The initial empty hostname caused Postfix startup failure;
  setting `/etc/rc.conf`, `/etc/hosts` and the live hostname let it start.
- `independent-installed-files.json`, `independent-installed-ffs.log` and
  `independent-installed-final.log`: independent persisted-file SHA-256 checks.
- `acceptance.json`: retained disk path/hash and tested configuration.
- `source-iso.sha256`: original ISO still matches after all runs.
- `fixed-binaries.sha256`: final core CLI and Heliacal release binaries.

The installation runner spent hours waiting at the final password/configuration
prompt between conversation turns. Its aggregate step/wall totals are **not**
installation-performance measurements. The disk-boot runner also includes
manual-command waiting time.

## Floppy probe correction

Helios already had NCR82077 reset/configuration, polling and seek/recalibrate
behavior, but the NetBSD probe stopped on byte offset zero: `probeget+0x1c`,
PC `f000a060`, word `d00a0000`; physical `71400000` on SS-5 and `ff1700000`
on SS-20.

NCR89C105 table 6-15 omits mode-dependent SRA/SRB. An early experiment treated
that omission as a faulting byte address, allowing NetBSD to skip fdc0.
Real SS-20/SS-5 NetBSD boot reports contradict that interpretation. The final
model completes a byte read without side effects and returns `ff` as an
explicit undriven-read convention pending hardware capture. It does not claim
PS/2 SRA pin semantics. Other unsupported registers/commands still stop; wider
accesses retain SBus errors. No floppy-image or sector-transfer engine was added.

The final INSTALL and GENERIC boots attach `fdc0 ... no drives attached` and
continue. Tracked tests cover pending IRQ, command/result state, all three
board bases, byte-only width, guest PC/nPC and absence of MMU fault capture.
See `docs/devices.md` and `docs/references.md` for the sources and limits.

## CPU capability probes

Certificate setup originally stopped at PC `00283fe8`, instruction `91408000`.
This is RD CCR on V9, reserved RD ASR2 on V8. V8 B.28 requires ASR1–14 reads
not to cause an illegal-instruction trap, leaving data undefined. Helios now
chooses Y readback, matching the pinned QEMU decoder's V8 choice. OpenSSL
clears Y before testing for the V9 CCR value; no V9 register is invented.
Reserved writes, ASR15 except STBAR, and implementation-specific ASRs retain
explicit host stops.

OpenSSL's following VIS probe encodings (`c19a5a40`, `81b00d80`, `81b022a0`,
`81b09206`) occupy V8 LDDC/CPop1 space. These machines have no optional non-FPU
coprocessor. V8 B.3/B.6/B.23/B.35 and C.9 require `cp_disabled`; NetBSD turns
it into the SIGILL that OpenSSL handles. CP operations, branches and transfers
now trap before alignment/data access; STDCQ retains privilege priority.
Tests cover captured words, classes/branch conditions, preserved memory/FPU,
delay-slot PC/nPC and ET=0 error latching. Certificate rehash and installed
OpenSSL now complete.

## Interrupted attempt and remaining limits

The first install unpacked its sets but hit the CPU stop before final
sync/unmount. Sysinst mounts targets with `-o async`. Recovery's read-only
fsck and independent CHD/FFS inspection found missing/old directory data,
including no kernel entry in the root. That interrupted image was never
promoted or repaired. A fresh format/install with the CPU fixes completed and
persisted correctly. A tracked 512-operation fragment/block/zero-write test
also matches a byte model across reopenings and independent stored-map reads.
Compact evidence from the failed attempt is retained; its scratch image and
extracted archive/kernel duplicates were removed after successful acceptance.

The installed GENERIC boot still emits a timecounter-backwards warning during
discovery, even with 75 MHz CPU / 2 MHz timer detection in hardware clock mode.
That timing issue remains unresolved. Entropy setup is incomplete. bpp0's
attachment with DMA revision zero is not parallel-port functional validation.

An earlier SS-5 CD experiment passed the floppy probe but panicked at the
unimplemented APC idle device: `apc_cpu_sleep+0x20`, read PC `f002790c`,
SFVA `fe028000`, SFSR `836`, physical `6a000000`. ISO symbols identify the
caller. `ss5-floppy-fixed-serial.log`, `-core.log` and `ss5-panic-symbols.txt`
retain this evidence. SS-5 NetBSD needs that separate device investigation;
no SS-5 installation claim is made.

## Validation and reproducibility

651 unique default core tests and 669 with CHD/perf-counters pass, plus
formatting, both Clippy configurations, all four demos and release build.
Heliacal was rebuilt; 75 tests, formatting, Clippy and SDL smoke pass.
Logs: `final-core-checks.log`, `feature-tests-final.log`,
`fragment-final-checks.log`, `final-heliacal-checks.log`.

The ignored `../boot/src/bin/netbsd_probe.rs` batches 10,000 steps between
serial socket polls. Options: `--boot-disk`, `--single-user`, `--userconf`,
`--accurate`. The accepted disk boot used only `--boot-disk --accurate`.
USERCONF exists in GENERIC, not the ISO's INSTALL kernel. `console.py --quiet`
controls the private socket while serial output is saved separately.
`inspect_ffs.py DISK.chd` performs the independent persisted-file checks.
Private runners are stopped and their build executables removed; source,
logs, hashes, useful references and the accepted disk are retained.
