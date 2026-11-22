# Helios

**HELIOS — Hardware Emulator for Legacy Implementations Of SPARC.**

Named after the Greek sun god, Helios is a Rust emulator for Sun SPARC
workstations. The first full-system target is the **SPARCstation 5 (sun4m,
85 MHz microSPARC-II / SPARC V8)** with **128 MiB RAM**, a **CGsix framebuffer**,
SCSI disk and CD-ROM, Ethernet and two serial ports. The longer-term
goal is to run its firmware and operating systems, including SunOS/Solaris and
compatible BSD/Linux releases.

The project follows the principles of our sibling [IRIS](../iris/README.md)
emulator: model the behavior guest software observes, keep the core usable from
both headless tools and graphical frontends, expose failures clearly, and build
an interpreter correctness baseline before optimizing. MAME and QEMU are
implementation references; the SPARC architecture manual defines ISA behavior.

## Current status

Original Sun OBP reaches a **responsive `ok` prompt on serial TTYA** after
completing device enumeration and its 128 MiB RAM test. The processor timer
and Ethernet internal-loopback diagnostics now pass. Network boot transmits
RARP requests through a headless packet endpoint; serial BREAK returns to the
monitor, where `1 2 + .` returns `3`. The remaining POST fault is an unimplemented
parallel-port DMA register. Installed Solaris boot checkpoints are described
below. See the
[latest firmware checkpoint](docs/firmware.md#timer-and-ethernet-bring-up).

On the experimental **Ultra 5**, original OpenBoot 3.31.0 detects the
module's 2 MiB external cache, prints its firmware version on serial A and
clears the cache tags. Cold boot finishes the unplugged keyboard probe. Both
reset paths discover 512 MiB, enable the instruction and data MMUs, copy
256 KiB from flash and run the relocated decompressor. It expands 453,536
bytes and jumps into that image in RAM. With diagnostics enabled, the
expanded firmware prints `ttya initialized` on serial A. It handles six
data-TLB misses, configures the PCI host and both Simba secondary buses,
initializes the IOMMU and uses CPU timer interrupts to calibrate its delay loop.
Both reset paths then read the onboard EBus base address. The interactive
Ultra 5 prompt is still in development.
[Reproduce the latest checkpoint](docs/ultra5.md#onboard-ebus-configuration)

**SPARCstation 20** and **Axil 320** profiles boot their original firmware to
the serial `ok` prompt with either one CPU or two 75 MHz SuperSPARC-II modules.
Fast clocks (100 ns per outcome) remain the default for quicker boots.
`--accurate-clocks` selects the nominal 85/75 MHz timebase; full pipeline and
cache timing remain unmodeled. They reuse the SS-5 device models on the
high sun4m address map with 36-bit reference-MMU behavior, SMC ECC register
access and IDPROM machine type `0x72`; the Axil profile uses its own MAC and
host ID. Both profiles have booted installed Solaris 2.6 disks to a console
login after migrating the disk's device paths to the high sun4m map. SS-20's
native SX/CG14 subset also runs Xsun with olwm and xterm; window movement,
Full Size/Normal Size, uncover and iconify/restore have been exercised through
the headless core. Heliacal also boots the graphical Solaris console and runs
Xsun/olwm/xterm with verified typing, mouse dragging and input capture/release.
Fresh SS-5 and Axil disk clones reach login and shut down cleanly. The
deterministic two-CPU scheduler runs both original SS-20 and Axil ROMs to a
responsive serial `ok`; their banners identify two processors. Solaris 2.6
also boots on disposable SS-20 and Axil disk clones with CPUs 0 and 2 online,
processor-bound guest work, disk I/O, checksum persistence across reboot and
clean halts. Heliacal's experimental CPU selector preserves one or two CPUs in
named machines. Recorded two-CPU Heliacal checkpoints include native-SX
terminal input and dragging, plus Axil CDE window operations, focus/capture,
disk activity and clean reset/reboot persistence. SMP remains experimental;
full POST/cache accuracy, a SuperSPARC pipeline model and four-CPU dual-module
MBus configurations remain incomplete. See [machine profiles](docs/machines.md).

The optional **graphical console** runs the original TurboGX FCode ROM and
Sun Type 5 keyboard protocol. SS-20 can also use its native SX/CG14 console.
With 128 MiB RAM, OBP displays its banner and `ok` prompt; keyboard Stop-A
interrupts network boot and `1 2 + .` returns `3` on screen. The native SX
implementation covers the observed Solaris drawing paths, not the entire
instruction set. See [graphics and keyboard](docs/graphics.md) for the supported
device subset and [the graphical checkpoint](docs/firmware.md#graphical-obp-console).

**`probe-scsi-all` now lists attached disk and CD-ROM targets and returns to `ok`**
in original OBP. Both frontends accept `--disk disk.raw` (ID 3) and
`--cdrom cdrom.iso` (ID 6). These options keep images read-only;
`--disk-rw disk.raw` explicitly enables persistent writes to an existing disk.
Block reads/writes, capacity reporting and cache flushing are implemented.
Solaris 2.6 and Solaris 7 CDs now load
their kernels, mount the CD's UFS root filesystem, and reach the installer's
**Select a Language** prompt on TTYA. Serial interrupts and the DMA register
behavior needed during device setup are implemented. Solaris 7 installs to one
writable 2 GiB disk and Solaris 2.6 to a separate 2 GiB disk; both installed
disks boot to a console login, and root shells were verified with empty local
test passwords. Audio I/O remains unimplemented.
See [SCSI storage](docs/scsi.md).

A minimal **NetBSD 11.1 sparc** installation onto a new 2 GiB CHD also boots
its unmodified GENERIC kernel to serial root login on SS-20 (one CPU, 128 MiB,
accurate clocks). Installed-file checksums and guest-written data survive
clean halt and host reopening. The SS-20 SX console also reaches graphical
root login with 256 MiB and fast clocks after implementing its cursor ROPs
and queue-empty status. This does not establish NetBSD X11 or SMP support;
the earlier serial boot's timecounter-backwards warning remains unresolved.

The implemented foundation includes:

- A Rust library and headless CLI, dependency-free by default; optional CHD
  storage and Linux host ICMP echo.
- An optional native framebuffer window, CGsix indexed VRAM/palette/cursor,
  SS-20 SX/CG14 scanout and an SX instruction subset, console drawing operations
  and a Sun keyboard/mouse on the second SCC.
- Eight overlapping register windows, `%g0`, PC/nPC, and integer condition codes.
- An integer instruction subset: SETHI/NOP, ADD/SUB/ADDX/SUBX and their `cc`
  variants; AND/OR/XOR/ANDN/ORN/XNOR and their `cc` variants; SLL/SRL/SRA;
  Bicc with delay slots and annul; CALL/JMPL; SAVE/RESTORE; LD/LDUB/LDUH/LDSB/LDSH
  and ST/STB/STH; LDD/STD; UMUL/SMUL/UDIV/SDIV and their `cc` variants;
  MULScc; TADDcc/TSUBcc with optional overflow traps; SWAP/LDSTUB; FLUSH.
- PSR/WIM/TBR/Y reads and writes, supervisor checks, Ticc, RETT, UNIMP and
  synchronous STBAR. Precise traps save PC/nPC, rotate windows and enter a guest
  handler; faults with traps disabled latch CPU error state.
- Initial FPU state, single/double transfers, FSR, moves, comparisons, division,
  addition/subtraction, multiplication, integer/single/double conversions and floating-point
  branches, plus disabled/deferred traps
  and queue draining. Implemented arithmetic supports all rounding modes and IEEE
  exception flags; widening multiplication, square roots and quad conversions remain pending.
- Word-backed RAM and ROM, big-endian byte lanes, checked mappings and accesses,
  raw image loading, instruction tracing, and focused CPU/bus tests.
- Alternate-space byte, halfword, word and doubleword loads/stores and atomic
  exchanges; microSPARC-II SRMMU
  translation, access permissions, contexts, fault registers and TLB flush/probe.
- CPU/MMU reset, boot-mode instruction mapping, SS-20/Axil high-address profiles,
  36-bit reference-MMU aliases, SMC ECC registers and a raw PROM runner.
- Explicit two-module SS-20/Axil execution with independently owned
  CPUs/MMUs/FPUs/MXCCs, deterministic processor-tagged scheduling, directed
  IPIs, atomic guest locks and one shared device clock. Original OBP and
  Solaris 2.6 recognize CPUs 0 and 2; four-CPU dual-module configurations and
  complete desktop SMP acceptance remain future work.
- Z8530 asynchronous polling and receive/transmit/BREAK interrupts, bounded byte queues,
  loopback telnet serial consoles for TTYA/TTYB, and an M48T08 NVRAM/RTC with SS-5
  IDPROM and explicit clock advancement.
- CPU interrupt entry with ET/PIL masking, sun4m software interrupts and source
  masks, and system/processor counter timers on a deterministic timebase.
- The processor's 64-bit user-timer register: a 54-bit count, overflow flag,
  start/stop control and consistent doubleword reads/writes.
- NCR89C105 auxiliary LED/link/mux output latches, floppy density input and
  terminal-count pulse events, plus NCR82077 initialization, timed seek/recalibrate
  completion and interrupts. No floppy drive or media engine is attached.
- NCR89C100 DMA2 SCSI transfers through the IOMMU, count/chaining/error handling
  and IRQ gating; NCR53C9X selection (including select-with-ATN-and-stop), timed
  missing-target responses, PIO/DMA command/data/status/message phases, raw
  disk/CD-ROM targets and optional persistent CHD hard disks.
- The SS-5 Crystal CS4231 register aperture used by Solaris's `audiocs` probe;
  audio DMA, sample conversion and interrupts remain unmodeled.
- NCR89C100 Ethernet DMA and NCR92C990 initialization, single-buffer packets,
  CRC, filtering and internal loopback through the IOMMU, plus an optional
  user-mode NAT (ARP/RARP, DHCP/BOOTP, ICMP echo, UDP, DNS relay and
  read-only TFTP). External IPv4 ping uses the optional Linux `host-icmp`
  feature. See [Ethernet](docs/ethernet.md) and
  [host networking](docs/networking.md).
- SS-5 system reset/status, guest software reset and watchdog status latching;
  a second polled SCC with an optional Type 5 keyboard and Sun mouse protocol.
- microSPARC-II IOMMU page translation, I/O TLB flushes, SBus arbitration and
  slot bypass, with byte/halfword/word DMA access to RAM through the shared TLB.
  The chip revision and virtual-address compare configuration are readable.
- microSPARC-II instruction/data cache diagnostic storage and indexed
  invalidation; normal accesses still use coherent memory without cache fills.
- SS-5 DRAM banks, smaller-module mirrors and empty-bank access behavior.
- SBus timeout/error status, asynchronous CPU store capture and module-error
  interrupts, plus live memory-request metadata in MFSR/MFAR.
- TLB diagnostic ports backed by live translation entries and functional replacement
  controls; injected PTP walks and M-bit rewalks remain pending.

Unsupported emulator behavior remains a host stop, separate from guest traps.
Implementation-specific ASRs, broader illegal-opcode classification, remaining TLB
diagnostic modes, cache hit/fill behavior, remaining FPU arithmetic/conversions, quad-operation traps,
serial bit timing, broader SCSI command support, audio streaming, a host network
backend and broader graphics acceleration remain to be implemented. There is no general firmware or OS compatibility
claim.

## OpenBIOS fallback

Omitting the PROM from `helios boot` selects a locally installed OpenBIOS image.
An explicit OBP image still takes precedence, including any errors. Heliacal
uses the same rule for an empty PROM setting, including named machines.
Build the pinned firmware port with `python3 tools/build_openbios.py`; firmware
is not bundled or downloaded automatically at startup. `HELIOS_OPENBIOS` can
point to its ELF. OpenBIOS has separate NVRAM, a 32 MiB/contiguous-RAM requirement,
and no native SX console; original OBP remains the accuracy baseline. See
[setup, provenance and tested compatibility](docs/firmware.md#automatic-openbios-fallback).

## Build and run

Use Rust 1.74 or newer with Cargo for the default build. The core needs no host GUI libraries or
downloaded crates; nightly features are not required.
For CHD hard disks, use a current stable Rust toolchain and a C++17 compiler,
then build with `cargo build --release --features chd`. The optional codec
dependency requires Rust 1.87 or later; it is built from source, with no
runtime `chdman` requirement. Heliacal enables this feature automatically.
Create an empty disk with
`target/release/helios create-disk local/disks/new-scsi3.chd --format chd --size-mib 2048`.
See [SCSI storage](docs/scsi.md) for persistent children and format limits.
The executable embeds [sun_ascii.txt](sun_ascii.txt) and prints its Sun logo and
the HELIOS name on stderr at startup, keeping guest serial bytes on stdout.

To use the graphical firmware console on an X11/XWayland desktop:

```sh
python3 tools/fetch_firmware.py sun-ss5
python3 tools/fetch_firmware.py turbogx
cargo run --release --manifest-path helios-ui/Cargo.toml -- \
  local/firmware/sun-ss5.bin local/firmware/turbogx.bin
```

After the memory test, hold **Pause** and press **A** for Sun Stop-A to reach
`ok`. Close the window to exit; Escape goes to the guest. The desktop frontend
has separate dependencies and a lockfile. Its build requirements, key mapping
and optional NVRAM persistence are in [graphics.md](docs/graphics.md).

```sh
cargo test --locked --offline
cargo run --locked --offline -- info
cargo run --locked --offline -- demo
cargo run --locked --offline -- demo --trace
cargo run --locked --offline -- trap-demo --trace
cargo run --locked --offline -- serial-demo
cargo run --locked --offline -- timer-demo --trace
```

The demo executes the hand-encoded program in [examples/sum.s](examples/sum.s),
adds 1 through 10, and verifies that guest RAM at `0x100` contains **55** after
43 instructions. It requires no ROM image or SPARC assembler.

The [trap demo](examples/trap.s) installs a trap table, enters user mode, executes
`TA 7`, runs a supervisor handler and returns through `JMPL`/`RETT`. It verifies
the return state after 14 CPU steps: 13 completed instructions and one trap entry.

The [serial demo](examples/serial.s) runs from a synthetic reset PROM, reads the
SS-5 machine type and IDPROM checksum, configures TTYA, prints `HELIOS`, and
echoes the supplied `!` byte followed by a newline. The host checks completion
and identity after 256 steps.

The [timer demo](examples/timer.s) configures both counter timers from reset,
handles interrupts at levels 10 and 14, acknowledges each timer and returns to
the interrupted loop using `JMPL`/`RETT`. It verifies both handler counts and
continued work after 256 steps. All four demos run without external firmware.

Run your own raw big-endian program with an explicit CPU step budget:

```sh
cargo run --release -- run local/program.bin --load-address 0x1000 --steps 1000 --trace
```

Entry defaults to the load address. `--entry` selects another address;
`--ram-mib` changes the default 128 MiB allocation. The bare-metal harness is
limited to 256 MiB; named `boot` profiles validate their DSIMM geometry (SS-5 up
to 256 MiB, SS-20 with built-in SX up to 448 MiB, SS-20 without a VSIMM and Axil
320 up to 512 MiB).
The bare-metal harness maps only RAM at physical address zero, with translation
and boot mode disabled. microSPARC-II physical addresses are 31 bits wide.
`run` accepts raw bytes, not ELF executables or workstation firmware images.
Reaching the budget is a successful host stop, not a guest halt instruction.
Each completed instruction, synchronous trap or interrupt entry counts as one
step, with separate trace records and totals. Guest software resets are traced
and counted too; their triggering stores count as completed instructions.
With `PSR.ET=1`,
implemented exceptions enter the guest handler; with `PSR.ET=0`, they enter CPU
error state and exit with status 1. Unsupported instructions also exit with
status 1, without a guest trap. The harness starts in supervisor mode with traps
disabled; guest code must install a trap table and enable traps to handle them.

Inspect a raw PROM from CPU/MMU reset with a bounded run:

```sh
cargo run --release -- boot local/prom.bin --steps 1000 --trace
# Optional saved NVRAM and serial input; the output file must not already exist:
cargo run --release -- boot local/prom.bin --steps 10000 \
  --serial-in local/input.bin --nvram local/nvram.bin --nvram-out local/nvram-next.bin
```

Expose TTYA or TTYB as a loopback telnet console while it runs (`0` selects a
free port; the chosen port is printed to stderr):

```sh
cargo run --release -- boot local/firmware/sun-ss5.bin --steps 100000000 --serial-port-a 8880
telnet 127.0.0.1 8880
```

Attach the optional user-mode NAT with DHCP/RARP, DNS relay, UDP and read-only
TFTP for network boot (`--tftp-dir` serves the boot file):

```sh
cargo run --release -- boot local/firmware/sun-ss5.bin --steps 1100000000 \
  --net --tftp-dir local/tftp
```

Select an SS-20 or Axil 320 profile with its high-address firmware:

```sh
cargo run --release -- boot local/firmware/ss10-20-v2.25r-rom \
  --machine ss20 --cpus 2 --keyboard --nvram local/ss20-ttya.nvram --steps 300000000
cargo run --release -- boot local/firmware/axil320-2-19.bin \
  --machine axil320 --cpus 2 --keyboard --cgsix-rom local/firmware/turbogx.bin \
  --nvram local/axil-ttya.nvram --steps 300000000
cargo run --release -- boot local/firmware/ss10-20-v2.25r-rom \
  --machine ss20 --display cgsix --cgsix-rom local/firmware/turbogx.bin \
  --ram-mib 512 --steps 300000000
```

RAM choices follow the installed bank layout: SS-5 supports up to 256 MiB,
SS-20 with SX up to 448 MiB, and SS-20 without a VSIMM or Axil up to 512 MiB.
The Axil OBP 2.19 profile uses 16/64 MiB banks; totals that need more than eight
banks are rejected. SS-20/CGsix Solaris console boot also requires excluding
the unimplemented DBRI slot with `setenv sbus-probe-list f0123` in OBP. See
[machine configuration](docs/machines.md#ss-20-cgsix-console-configuration).

For external IPv4 ping, build on Linux with `--features host-icmp`; Heliacal
enables this feature. Gateway ping needs no optional feature. TCP NAT and host
port forwards are not implemented yet. See
[host networking](docs/networking.md).

`boot` installs the SS-5 bank layout and a read-only 1 MiB PROM aperture at `0x70000000`, starts
PC/nPC at 0/4, and selects PROM for instruction-space accesses through boot
mode. Ordinary data accesses use the physical map, including console serial at
`0x71100000` and NVRAM/RTC at `0x71200000`. `boot` and `serial-demo` send TTYA
bytes to stdout and diagnostics/traces to stderr. Input comes from a file or,
for `boot`, a loopback telnet console. NVRAM uses an 8192-byte raw image and is
saved at the step limit or an execution error when `--nvram-out` is supplied.
Use `boot --rtc host` to initialize its calendar from host UTC. The CLI default
`--rtc saved` preserves NVRAM time (year 2000 for fresh NVRAM); a decimal Unix
timestamp also accepts a reproducible date. Heliacal defaults to host UTC at
startup, with a per-machine Advanced option to preserve saved guest time.
This seeds the RTC once and is independent of CPU speed and display mode.
The sun4m harness advances device time by a 100 ns base per completed step,
including trap/interrupt entry, plus physical PROM read wait states. This is a
functional scheduling convention, not full CPU cycle timing. `--accurate-clocks`
selects nominal profile rates, at the cost of more interpreted work during
guest timer waits. The library also
exposes explicit time advancement and a decoded serial BREAK input. See [Memory and reset](docs/memory.md),
[Devices](docs/devices.md) and [Interrupts and timers](docs/interrupts-timers.md).
Guest reset behavior and current watchdog limits are in [System reset](docs/reset.md).
For `boot`, RAM must fit the selected profile's DSIMM sockets and module
sizes; larger modules occupy low slots first. The default is 128 MiB. SS-5 uses
8/32 MiB modules in eight sockets (maximum 256 MiB). SS-20 with built-in SX
reserves bank 7 for the VSIMM (maximum 448 MiB); the Axil OBP 2.19 profile uses
eight 16/64 MiB banks (maximum 512 MiB). The eight Axil RAM sockets and two
VSIMM-capable sockets are owner-confirmed. Physical support for 8/32 MiB Axil
modules remains unverified; the profile uses the bank sizes recognized by its
pinned firmware. `--display none`
or `--display cgsix` leaves the SS-20 VSIMM slot empty, allowing all eight DRAM
banks and 512 MiB. See [DRAM and TLB diagnostics](docs/dram-tlb.md) for
mirrors, empty-bank reads and the distinction from the contiguous RAM used by
synthetic harnesses; [references](docs/references.md) records the Axil
module-size evidence gap.

Original-firmware Solaris 2.6 checks cover SS-5/CGsix at 256 MiB, SS-20/SX at
448 MiB, and SS-20/CGsix and Axil at 512 MiB. These include bounded memory
verification, nonzero disk I/O, clean halt and reboot checksum checks, with
one and two CPUs on the MBus profiles. Heliacal also reaches usable graphical
consoles at those maxima. See [machine profiles](docs/machines.md) for setup
requirements and the limits of this acceptance.

## Development

- [HACKING.md](HACKING.md): architecture, invariants and validation commands.
- [SPARCstation 5](docs/sparcstation5.md): target and reference memory map.
- [Machine profiles](docs/machines.md): SS-20 and Axil 320 boot identities, high-address map and limits.
- [Memory and reset](docs/memory.md): SRMMU, ASIs, fault behavior and PROM mapping.
- [Floating point](docs/fpu.md): FPU state, transfers, comparisons, arithmetic, branches and deferred traps.
- [Fault capture](docs/fault-capture.md): SBus errors, asynchronous stores and memory-request capture.
- [Cache diagnostics](docs/cache.md): diagnostic storage, invalidation and functional limits.
- [DRAM and TLB diagnostics](docs/dram-tlb.md): DSIMM layout, diagnostic entries and replacement controls.
- [System reset](docs/reset.md): reset-status register, guest reset and watchdog limits.
- [Devices](docs/devices.md): console serial, IDPROM, NVRAM persistence and RTC.
- [Graphics and keyboard](docs/graphics.md): TurboGX, Type 5 keyboard and native console window.
- [SCSI storage](docs/scsi.md): image attachment, controller transfers and target commands.
- [Host networking](docs/networking.md): user-mode NAT, DHCP/RARP, DNS relay, UDP and TFTP limits.
- [Interrupts and timers](docs/interrupts-timers.md): register maps, masking and guest time.
- [I/O controls](docs/io-control.md): IOMMU translation, shared TLB, DMA access and current limits.
- [Firmware](docs/firmware.md): original Sun OBP development baseline, pinned downloads and probe results.
- [Roadmap](docs/roadmap.md): milestones from this harness to a usable system.
- [References](docs/references.md): specific MAME/QEMU sources and provenance.

Keep local firmware, disks and scratch programs under the ignored `local/`
directory. The repository contains no firmware or operating-system images.

Original Sun OBP is the primary firmware for development and accuracy checks.
Fetch the selected SS-5 image from the user-specified archive with Python 3:

```sh
python3 tools/fetch_firmware.py sun-ss5
cargo run --locked --offline -- boot local/firmware/sun-ss5.bin --steps 10000 --trace
```

The short probe reaches OBP's timed keyboard polling loop. To reproduce the
serial monitor checkpoint with the original unchanged image:

```sh
cargo run --release --locked --offline -- boot local/firmware/sun-ss5.bin --steps 100000000
```

The first `ok` arrives at step **93481624** with default NVRAM, 128 MiB RAM and
no serial input. It follows a device-probe error and does not establish a clean
boot. The CLI prints serial output; interactive input currently uses the library
API. [Firmware records](docs/firmware.md#serial-ok-prompt) include the transcript,
counts, input/response verification and remaining limitations.
The fetched image is hash-verified and stays ignored.
OpenBIOS is an explicit replacement-firmware compatibility path; original OBP
remains the hardware-accuracy baseline.

