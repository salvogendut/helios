# Installed Solaris disk boot

Date: 2026-11-07. The user reported two local-disk boot stops. The SS-5 path
now boots the installed Solaris 7 disk to its first-boot root password prompt.

## Reported stops

1. ASI 47 word read, `PC=0x00026328`, instruction `0xec84c5e0`. Booting
   `local/firmware/ss10-20-v2.25r-rom` through the SS-5 harness reproduces this
   exactly:

   ```text
   helios boot local/firmware/ss10-20-v2.25r-rom --steps 50000000
   helios: emulator stop: UnsupportedMemory(UnsupportedAsi { asi: 47, width: Word, write: false })
   at PC=0x00026328, nPC=0x0002632c, instruction=0xec84c5e0
   ```

   The SS10/20 OpenBoot 2.25R ROM targets a different machine. The pattern also
   occurs inside ASCII data in that ROM and `axil320-2-19.bin`; it is absent
   from `sun-ss5.bin`, the Solaris 7 image and the Solaris media. The supported
   SS-5 reproduction does not hit ASI 47. ASI 47 is not implemented as a
   microSPARC-II extension; do not add it merely to run another machine's PROM.
   The current Heliacal config uses `sun-ss5.bin`.
2. ESP command fault at `PC=f5aa8d9c`, physical `0x7880000c`, during Solaris
   kernel device attach. Temporary logging showed Solaris writes `0xc3`
   (`SELATNS|DMA`). The report's captured ESP state kept the previous command
   because validation ran before assignment.

## Fixes

- Added ESP select-with-ATN-and-stop (`0x43`) handling. It transfers one
  IDENTIFY byte, reports sequence step 1 in message-out with interrupt
  `BS|FC`, and accepts a following TRANSFER INFORMATION message transfer
  (`0x10`/`0x90`) before command phase. Added PIO and DMA regressions.
- Added the SS-5 CS4231 register subset at `0x6c000000..0x6c00003f`.
  Solaris's `audiocs` probe performs byte accesses to the index/data ports;
  the model follows QEMU's CS4231 register array and the public CS4231 port
  layout. No audio DMA, conversion or interrupts are modeled.

## Verification

Ignored probe: `local/probes/solaris-install/disk-probe.rs` compiled against
the current release library. It attached a copy of `local/disks/solaris7-install.raw`
at ID 3, used the saved `sol7-power-nvram.bin`, sent `boot disk -v`, and reached:

```text
sd3 is /iommu@0,10000000/sbus@0,10001000/espdma@5,8400000/esp@5,8800000/sd@3,0
root on .../sd@3,0:a fstype ufs
...
audiocs0 at sbus0: SBus slot 4 0xc000000 SBus level 5 sparc ipl 9
audiocs0 is .../SUNW,CS4231@4,c000000
...
The system is ready.

helios console login: root
Nov  7 10:52:53 helios login: ROOT LOGIN /dev/console
Sun Microsystems Inc.   SunOS 5.7       Generic October 1998
#
```

The first-boot screens were answered with two bare returns (no root password).
The run reached a root shell at about step 2.89B with no host stop before the
host timeout. Raw serial logs:
`local/probes/solaris-install/sol7-csbyte-serial.log` and
`local/probes/solaris-install/sol7-login-serial.log`. This establishes installed
system boot, console login and a root shell. Audio playback/capture and external
networking remain unverified. The installed image's `/etc/vfstab` references
`c0t3d0`, so the disk must stay at SCSI ID 3 for this installation.
The local installation image was copied before
each run; source media and the installed image were not overwritten by these
probes.

## Solaris 2.6 installation

A new sparse 2 GiB disk was created at `local/disks/solaris26-install.raw`
(`open(..., 'xb')` plus truncate). Ignored probe
`local/probes/solaris-install/probe26` attached it writable at SCSI ID 3, the
read-only `solaris_2.6_598_sparc.iso` at ID 6, and the original SS-5 PROM/CGsix.
Inputs selected English, ASCII locale, DEC VT100, hostname `helios`, no network,
Middle Europe, current date/time, no client services, End User System Support,
`c0t3d0`, automatic layout (`/`, `/usr`, swap), and manual reboot.

The first `suninstall` run stopped at the expected blank-disk warning. From the
recovery shell, `format` auto-configured the disk, labeled it, and `suninstall`
restarted. Installation completed; `sync` and `halt` returned the live session
to the `ok` prompt. Logs:
`sol26-install-serial.log`, `sol26-install.log`, and
`sol26-install-nvram.bin`.

A separate ignored `disk-probe26` booted the disk with the saved NVRAM and no CD.
It mounted the UFS root on `sd@3,0`, completed first-boot setup with an empty
root password, and reached:

```text
helios console login: root
Nov  7 20:08:13 helios login: ROOT LOGIN /dev/console
Sun Microsystems Inc.   SunOS 5.6       Generic August 1997
#
```

`sync` and `halt` then returned to `ok`. Raw boot serial:
`local/probes/solaris-install/sol26-boot-serial.log`. Keep this installed disk at
SCSI ID 3; its device names are `c0t3d0`.
