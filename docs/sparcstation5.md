# SPARCstation 5 target

The first machine target is a uniprocessor SPARCstation 5, sun4m, with a
microSPARC-II-compatible SPARC V8 core and eight register windows. This keeps
the first platform focused while leaving other Sun families for later profiles.
The current executable is only the bare-metal harness described in the README.

QEMU's SS-5 profile selects `Fujitsu-MB86904` and limits RAM to 256 MiB.
Helios uses 64 MiB as its default harness allocation. These settings and the
following addresses are drawn from QEMU's
[sun4m machine definition](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/sparc/sun4m.c).

| Component | Reference physical base | Helios status |
| --- | --- | --- |
| RAM | `0x00000000` | Implemented in harness |
| IOMMU | `0x10000000` | Planned |
| TCX framebuffer | `0x50000000` | Planned |
| Boot PROM / SLAVIO area | `0x70000000` | Planned |
| Keyboard/mouse serial | `0x71000000` | Planned |
| Console serial | `0x71100000` | Planned |
| NVRAM / RTC | `0x71200000` | Planned |
| Timer block | `0x71d00000` | Planned |
| Interrupt block | `0x71e00000` | Planned |
| DMA block | `0x78400000` | Planned |
| ESP SCSI | `0x78800000` | Planned |
| LANCE Ethernet | `0x78c00000` | Planned |

This table is a reference inventory, not installed bus mappings. Devices contain
subregions and mirrors that need independent verification before implementation.
In particular, reset instruction fetch and PROM access depend on CPU/MMU boot
behavior; simply setting PC to the physical PROM base is not a reset model.

Bring-up starts with a serial console. The first firmware milestone requires
CPU privileged state and traps, SRMMU/ASIs, reset/PROM mapping, NVRAM/IDPROM,
serial, interrupts and timers. Firmware may probe additional devices, which
must be implemented or explicitly diagnosed as evidence arrives.

Firmware compatibility has two separate paths to investigate: Sun firmware and
OpenBIOS. QEMU supplies OpenBIOS with interfaces such as `fw_cfg`; those are
emulator interfaces and must be identified as such if Helios implements them.
A successful OpenBIOS boot alone would not establish Sun PROM compatibility.

