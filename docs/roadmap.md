# Development milestones

## Verified workstation configurations

The table records the furthest reproduced guest result for each configuration with
its original OpenBoot PROM. Firmware choice, installed graphics and CPU count
matter when reproducing a result.

| Machine | Verified result | Setup and evidence |
| --- | --- | --- |
| SPARCstation 5 | Solaris 2.6 and Solaris 7 install from SCSI CD and boot from separate disks to a root shell. Solaris 2.6 runs with CGsix and 256 MiB. | [Disk boot](firmware.md#installed-solaris-disk-boot) · [RAM and display](machines.md) |
| SPARCstation 20 | Solaris 2.6 boots with one or two CPUs. Native SX runs Xsun, olwm and xterm. NetBSD 11.1 installs to CHD and boots on one CPU to both serial and SX framebuffer consoles. | [Solaris graphics](graphics.md#ss-20-built-in-sxvsimm) · [NetBSD installation](scsi.md#netbsd-111-installation-checkpoint) |
| Axil 320 | Solaris 2.6 boots with one or two CPUs. The two-CPU CGsix CDE session passed keyboard, window and disk-persistence checks. | [Machine checks](machines.md#evidence-and-limits) · [Migrating an SS-5 disk](scsi.md#moving-an-installed-solaris-26-disk-from-ss-5-to-ss-20axil) |
| Ultra 5 | Solaris 7 11/99 installs onto the primary IDE disk, boots its 32-bit sun4u kernel and reaches an interactive CDE desktop on Mach64. The onboard HME supports DHCP and outbound file transfers. | [Install and boot](ultra5.md#boot-the-installed-solaris-7-system) · [Network checks](networking.md#guest-checks) |

The separately built SPARC32 OpenBIOS port reaches Forth on serial and TurboGX
on all three sun4m machines. Guest checks include a NetBSD 11.1 single-user disk
boot on SS-20 and a Solaris 2.6 single-user CD boot on two-CPU Axil.
[Replacement firmware and configuration limits](firmware.md#nvram-and-current-compatibility-limits)

## Remaining work and limits

- **Keyboard-free sun4m startup.** Heliacal keeps a keyboard attached in headless
  mode so original OBP can use abbreviated startup on the MBus machines. Removing
  it exposes SS-5 and SS-20 POST diagnostics. The Ultra 5 already reaches its
  serial prompt with the keyboard absent. [Helios issue #7](https://github.com/salvogendut/helios/issues/7)
- **CPU and devices.** Four-CPU MBus configurations, full SX instruction coverage,
  ordinary microSPARC-II cache fills, audio streaming and a floppy media engine
  remain open. NetBSD X11 and SMP have no verified configuration here.
  [Machine limits](machines.md#evidence-and-limits) · [Cache diagnostics](cache.md) · [Floppy controller](devices.md#ncr82077-floppy-initialization-and-positioning)
- **Network boot.** Ultra 5 OpenBoot executes a Forth payload fetched by RARP/TFTP.
  Network installation of an OS is a separate milestone. [Reproduce the firmware boot](ultra5.md#firmware-network-boot)
- **Performance.** The interpreter runs the verified workstations. The optional
  x86-64 Unix native backend on main accelerates a limited set of bare-metal V8
  instructions. The workstation JIT and interpreter I/O experiments are parked.
  Heliacal offers opt-in idle parking for a proven single-CPU SS-20 Solaris loop.
  [Execution and profiling](../HACKING.md#execution-and-profiling)

The [sun4m firmware records](firmware.md#historical-bring-up-checkpoints) and
the Ultra 5 sequence below preserve earlier bring-up results.

## Ultra 5 serial console

**Original OBP 3.31.0 reaches a responsive serial monitor on 22 November 2026.**
Cold and soft-reset runs reach the same entry with diagnostics enabled or disabled.

| Milestone | Evidence |
| --- | --- |
| First serial register access | At virtual PC `0xffff_ffff_f000_033c`, the guest stores one byte, `0xc3`, at physical `0x01ff_f140_002c` (channel A CCR0). Both tested reset-marker paths reach this write. |
| First firmware text | Serial A emits `Hardware Power ON` on the cold path and `Software Power ON` on the matching-marker path. Each ends in CR/LF. With the diagnostic byte clear, these writes to the transmit FIFO are skipped. |
| DRAM controller setup | All four marker/diagnostic combinations turn on the front-panel LED, program both MCU registers, complete both polling delays and reach external-cache sizing. |
| External-cache initialization and build identification | All four runs detect 2 MiB, select ELIM 3 and clear 32,768 tag entries. Both diagnostic-enabled runs print the original firmware build string. |
| Cold-start keyboard probe | The unplugged keyboard probe completes three timed waits and one reset-byte transmission. With diagnostics enabled, serial A prints `Probing keyboard Done`. |
| Instruction and data TLB clear | On both paths the firmware clears all 64 slots in each bank, returns to its caller and reaches the ASI `0x14` read at physical address zero. |
| Physical DRAM discovery | OBP detects the two 256 MiB DIMMs in pair 2, selects 11-bit columns, reports the upper 256 MiB rank at `0x30000000` and clears the top 1 MiB. |
| MMU enable and relocation | The original ROM copies 256 KiB to PA `0x3ff00000`, branches to VA `0x2038` backed by RAM and starts its decompressor. Both diagnostic runs print the flash and RAM PCs. |
| Decompression and entry | OBP expands 453,536 bytes at VA `0xf0000000` and executes the entry at VA `0xf0051f8c`. A local-ROM test compares every byte against an independent decoder. |
| Serial initialization in the expanded image | The guest clears both on-chip cache tag banks and prints `ttya initialized` with diagnostics on. Both reset paths then enter `fprs!`. |
| MMU trap handling | The guest handles four data-TLB misses, retries the faulting memory operations and reaches the PCI host bridge. |
| PCI interrupt initialization | The guest handles a fifth data-TLB miss, programs PCI Control/Status and clears the PCI and onboard interrupt state and mappings. |
| PCI host configuration | OBP enables the 512 MiB target window at `0xc0000000`, programs the PCI command and latency registers, and starts configuring Simba function 0. |
| Simba bridge configuration | The guest programs both bridge functions and routes the onboard flash and EBus through bus B. All four reset/diagnostic combinations reach the first IOMMU control read. |
| IOMMU initialization | OBP clears the 16 I/O translations, sets the translation table base and enables the IOMMU on cold and soft boots. |
| CPU timer | OBP programs TICK and TICK_CMPR, takes a level-14 interrupt in its calibration loop, and returns through RETRY. |
| Clock calibration and onboard EBus | OBP executes its 64-bit unsigned divide, reads PCIO BAR1 and reaches the PCI command write. |
| Timekeeper and reset report | OBP programs the EBus command register, reads latched RTC seconds, restores NVRAM defaults on the test seeds and prints the reset cause on TTYA. |
| Module and memory banks | OBP acknowledges the reset and error flags, reads the flash signature and reports 512 MiB in bank 2 on serial A. |
| DRAM refresh and PCI identification | OBP restores Mem_Control0 through the primary caches, reads the onboard ATI device ID and identifies the Simba multifunction bridge. |
| PCI enumeration and banner | The guest probes UPA and both Simba buses, identifies the onboard devices and prints the 440 MHz / 512 MB Ultra 5/10 banner. |
| Interactive OpenBoot console | Verified by typing `1 2 + .` and receiving `3` and a second `ok`, both with autoboot disabled and after serial BREAK. |
| Graphical OpenBoot console | Heliacal displays the Sun logo and banner on Mach64 and accepts a keyboard-entered `1 2 + .`, returning `3` and another prompt. |
| Read-only ATAPI CD-ROM | OBP detects the secondary master and returns all 2,048 bytes of Solaris 7 ISO sector 16 through its device method. |
| Primary ATA disk | OpenBoot enumerates device 0. Sector writes persist through PIO and CMD646 bus-master DMA. |
| Solaris 7 installation CD | The 11/99 SPARC kernel mounts the installation CD, attaches the primary disk and enters Solaris Interactive Installation. |
| Installed Solaris 7 | The installer writes the disk label, filesystems, packages and boot blocks. After a clean halt, OpenBoot starts the installed kernel from IDE unit 0 with the CD removed. |
| Graphical login | Heliacal reaches the Solaris CDE greeter. Root login opens the CDE front panel, File Manager and Help Viewer on Mach64. |
| Onboard Ethernet | Installed Solaris 7 brings up `hme0` on the Ultra 5. In Heliacal it receives three replies from the NAT gateway at `192.168.0.1` with zero packet loss. |
| Guest network transfers | Solaris 7 obtains a DHCP lease, resolves a controlled DNS name through the gateway, and downloads the same 1 MiB file over TFTP and outbound HTTP. Both copies match the host checksum. |
| Diskless firmware network boot | Original OpenBoot 3.31 passes HME loopback, obtains an address by RARP and downloads a Forth payload by TFTP. The payload prints its marker on serial A and returns to the prompt. [Reproduce the check](ultra5.md#firmware-network-boot) |

[Boot the installed Ultra 5](ultra5.md#boot-the-installed-solaris-7-system)
