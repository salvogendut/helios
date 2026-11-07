# SPARCstation 5 target

The first machine target is a uniprocessor SPARCstation 5, sun4m, with a
microSPARC-II-compatible SPARC V8 core at a nominal **85 MHz** and eight register
windows. The initial configuration uses **128 MiB RAM**, **CGsix graphics**,
one **SCSI disk**, one **SCSI CD-ROM**, **Ethernet**, and **TTYA/TTYB**. This keeps
the first platform focused while leaving other Sun families for later profiles.
The executable provides bare-metal and reset/PROM memory harnesses described
in the README, with a polled console, NVRAM/RTC, interrupt control and counter
timers and system reset/status; other peripherals are pending.

The graphics target is a standard 1 MiB TurboGX card in the CGsix family, now
implemented as a console subset in SBus slot 1 at physical `0x30000000`.
The native frontend attaches it with a Type 5 US keyboard; see
[graphics.md](graphics.md). The SS-5 service manual lists the 85 MHz board and 1 MiB TGX option;
the SPARCengine 5 OEM manual describes this card's BT458 RAMDAC and 8-bit output.
See [references.md](references.md). Disk capacity and media are configuration
choices for the storage milestone. SCSI data transfers and installed-system
boot are now exercised; Ethernet has a headless packet endpoint. See
[ethernet.md](ethernet.md) for its implemented subset.

`CPU_CLOCK_HZ` records the nominal 85 MHz clock. The interpreter's 100 ns step
quantum is a separate functional scheduling convention. Physical PROM reads add
six 21.25 MHz SBus clocks per byte; the pipeline and ordinary caches are not timed.
QEMU's SS-5 profile selects `Fujitsu-MB86904` and limits RAM to 256 MiB; Helios
uses 128 MiB by default within that limit, with hardware bank decode in
`Machine::ss5`/CLI `boot`. [DRAM and TLB diagnostics](dram-tlb.md) describes
module mirrors, empty-bank behavior and the remaining memory-model limits. Common platform addresses below
were compared against QEMU's
[sun4m machine definition](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/sparc/sun4m.c).

| Component | Reference physical base | Helios status |
| --- | --- | --- |
| DRAM | `0x00000000` | Eight 32 MiB apertures; default four 32 MiB DSIMMs, four empty slots |
| SBus / IOMMU controls | `0x10000000` | Page translation, I/O flushes, slot bypass/arbitration and fault capture; RAM DMA API |
| CGsix / TurboGX framebuffer | `0x30000000` | Console subset with VRAM, palette, cursor and acceleration registers |
| CS4231 audio | `0x6c000000` | Indirect register/version subset; no audio DMA or interrupts |
| Boot PROM / SLAVIO area | `0x70000000` | 1 MiB raw PROM aperture; remaining SLAVIO devices pending |
| Keyboard/mouse serial | `0x71000000` | Second SCC with interrupts, Type 5 keyboard and Sun mouse protocol |
| Console serial | `0x71100000` | Z8530 asynchronous polling/interrupts, A/B byte ports |
| NVRAM / RTC | `0x71200000` | 8 KiB M48T08, IDPROM and deterministic clock |
| Floppy controller | `0x71400000` | NCR82077 configuration, reset, positioning/status and IRQ; no drive/media engine |
| Auxiliary registers | `0x71900000`, `0x71910000` | LED/link/mux/density/TC and power scratch/clear subset |
| Timer block | `0x71d00000` | CPU counter or 64-bit user timer here; system counter/configuration at `+0x10000` |
| Interrupt block | `0x71e00000` | CPU pending/soft IRQs here; system pending/masks at `+0x10000` |
| System reset/status | `0x71f00000` | Software reset, reset-cause flags; automatic CPU watchdog recovery pending |
| DMA2 SCSI/Ethernet | `0x78400000`, `0x78400010` | SCSI/Ethernet DMA through IOMMU |
| ESP SCSI | `0x78800000` | NCR53C9X initiator subset, selection/message/data phases and target images |
| NCR92C990 Ethernet | `0x78c00000` | Halfword CSR ports, initialization, packets and internal loopback |

QEMU's TCX mapping at `0x50000000` describes a different graphics device and is
not used as the CGsix register map. Use the card's own SBus-relative registers,
PROM and interrupt wiring when implementing it.

RAM, PROM, both SCCs, NVRAM/RTC, interrupt control, counter timers and reset/status are
installed, alongside the [I/O configuration subset](io-control.md).
See [devices.md](devices.md), [reset.md](reset.md) and [interrupts-timers.md](interrupts-timers.md)
for exact apertures and limits. Remaining devices contain
subregions and mirrors that need independent verification before implementation.
In particular, reset instruction fetch and PROM access depend on CPU/MMU boot
behavior. Helios starts at virtual PC 0 with PCR.BM set; instruction accesses
then use `0x70000000 | (VA & 0x0fffffff)`. See [memory.md](memory.md).

Bring-up starts with a serial console. The first firmware milestone requires
CPU privileged state and traps, SRMMU/ASIs, reset/PROM mapping, NVRAM/IDPROM,
serial, interrupts and timers. Firmware may probe additional devices, which
must be implemented or explicitly diagnosed as evidence arrives.

Original Sun OBP is the primary firmware for development and hardware accuracy.
OpenBIOS remains a separate compatibility test until proved working. QEMU
supplies it with interfaces such as `fw_cfg`; those must not redefine SS-5
hardware behavior. [firmware.md](firmware.md) records the pinned images and
bounded probe results. Original OBP reaches responsive serial and graphical prompts;
OpenBIOS remains unproven, and no OS boot is supported.

