# Helios

**Hardware Emulator for Legacy Implementations Of SPARC**

Helios is a Rust emulator for Sun SPARC workstations. It provides the machine
models and a headless command-line runner. For a graphical desktop, use
[Heliacal](https://github.com/salvogendut/heliacal), the SDL frontend built on
the same core. It provides saved machine configurations, keyboard and mouse
input, and an interactive serial console.

## Machines and verified guests

| Machine | Emulated hardware | Verified with original OpenBoot firmware |
| --- | --- | --- |
| SPARCstation 5 (sun4m) | 85 MHz microSPARC-II, CGsix, SCSI, up to 256 MiB | Solaris 2.6 and 7 install from CD and boot from disk to a root shell. |
| SPARCstation 20 (sun4m) | One or two 75 MHz SuperSPARC-II CPUs, SCSI, up to 448 MiB with native SX or 512 MiB without the VSIMM | Solaris 2.6 boots with one or two CPUs and runs Xsun on SX. NetBSD 11.1 boots on one CPU to serial and SX framebuffer consoles. |
| Axil 320 (sun4m) | One or two 75 MHz SuperSPARC-II CPUs, CGsix, SCSI, up to 512 MiB | Solaris 2.6 boots with one or two CPUs. The two-CPU CDE desktop supports keyboard and mouse interaction. |
| Ultra 5 (sun4u) | 440 MHz UltraSPARC-IIi, 512 MiB, Mach64, IDE, HME Ethernet | Solaris 7 11/99 installs to IDE disk, boots its 32-bit kernel and reaches an interactive CDE desktop. |

## Build and run

From the repository root, build the headless CLI with Rust 1.74 or newer and
Cargo. The demo needs no firmware, disk image or GUI libraries.

```sh
cargo build --release --locked
./target/release/helios demo
```

The guest adds 1 through 10. The demo exits successfully when guest RAM at
`0x100` contains **55**.

To boot a workstation, supply an original PROM for the selected model, or
install the Helios OpenBIOS port for sun4m. Leaving the sun4m PROM path empty
selects the locally installed port. For the Ultra 5, use Heliacal or the
dedicated serial runner with its original flash. Firmware and guest media are
supplied separately. [Sun4m firmware setup](docs/firmware.md) · [Ultra 5 flash setup](docs/ultra5.md#reconstruct-and-test-the-original-firmware) · [Ultra 5 serial runner](docs/ultra5.md#interactive-serial-console)

With OpenBIOS installed, this disk-free SS-20 boot prints the firmware banner
and the `0 >` Forth prompt to your terminal. The CLI exits at the step budget.
For interactive input, use Heliacal or the [CLI serial options](docs/devices.md#cli-and-validation).

```sh
./target/release/helios boot --machine ss20 --display none --ram-mib 64 --steps 160000000
```

## Guides

- **Configure a machine** — [sun4m RAM, displays and CPUs](docs/machines.md) · [Ultra 5 setup and Solaris installation](docs/ultra5.md) · [Heliacal build and first boot](https://github.com/salvogendut/heliacal/blob/main/docs/building.md)
- **Attach media and peripherals** — [SCSI images and CHD creation](docs/scsi.md) · [Graphics and keyboard](docs/graphics.md) · [Serial and NVRAM](docs/devices.md) · [Host networking](docs/networking.md)
- **Develop and test** — [Architecture, tracing and validation](HACKING.md) · [Verified configurations and remaining work](docs/roadmap.md) · [Hardware manuals and emulator references](docs/references.md)
