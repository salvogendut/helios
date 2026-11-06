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

This is an executable foundation, **not yet a bootable SPARCstation**. It has:

- A dependency-free Rust library and headless CLI.
- Eight overlapping register windows, `%g0`, PC/nPC, and integer condition codes.
- An integer instruction subset: SETHI/NOP, ADD/SUB/ADDX/SUBX and their `cc`
  variants; AND/OR/XOR/ANDN/ORN/XNOR and their `cc` variants; SLL/SRL/SRA;
  Bicc with delay slots and annul; CALL/JMPL; SAVE/RESTORE; LD/LDUB/LDUH/LDSB/LDSH
  and ST/STB/STH; LDD/STD; UMUL/SMUL/UDIV/SDIV and their `cc` variants;
  MULScc; TADDcc/TSUBcc with optional overflow traps; SWAP/LDSTUB; FLUSH.
- PSR/WIM/TBR/Y reads and writes, supervisor checks, Ticc, RETT, UNIMP and
  synchronous STBAR. Precise traps save PC/nPC, rotate windows and enter a guest
  handler; faults with traps disabled latch CPU error state.
- Word-backed RAM and ROM, big-endian byte lanes, checked mappings and accesses,
  raw image loading, instruction tracing, and focused CPU/bus tests.
- Alternate-space byte, halfword, word and doubleword loads/stores and atomic
  exchanges; microSPARC-II SRMMU
  translation, access permissions, contexts, fault registers and TLB flush/probe.
- CPU/MMU reset, boot-mode instruction mapping and a raw PROM runner.
- Z8530 asynchronous polling on both console channels, bounded byte queues,
  and an M48T08 NVRAM/RTC with SS-5 IDPROM and explicit clock advancement.
- CPU interrupt entry with ET/PIL masking, sun4m software interrupts and source
  masks, and system/processor counter timers on a deterministic timebase.
- The processor's 64-bit user-timer register: a 54-bit count, overflow flag,
  start/stop control and consistent doubleword reads/writes.
- SS-5 system reset/status, guest software reset and watchdog status latching;
  a second polled SCC for the keyboard/mouse ports (input protocols pending).
- microSPARC-II IOMMU page translation, I/O TLB flushes, SBus arbitration and
  slot bypass, with byte/halfword/word DMA access to RAM through the shared TLB.
- microSPARC-II instruction/data cache diagnostic storage and indexed
  invalidation; normal accesses still use coherent memory without cache fills.
- SS-5 DRAM banks, smaller-module mirrors and empty-bank access behavior.
- SBus timeout/error status, asynchronous CPU store capture and module-error
  interrupts, plus live memory-request metadata in MFSR/MFAR.
- TLB diagnostic ports backed by live translation entries and functional replacement
  controls; injected PTP walks and M-bit rewalks remain pending.

Unsupported emulator behavior remains a host stop, separate from guest traps.
Implementation-specific ASRs, broader illegal-opcode classification, remaining TLB
diagnostic modes, cache hit/fill behavior, FPU, serial interrupt generation, SCSI, networking
and graphics remain to be implemented. There is no firmware or OS compatibility
claim.

## Build and run

Use Rust 1.74 or newer with Cargo. The core needs no host GUI libraries or
downloaded crates; nightly features are not required.
The executable embeds [sun_ascii.txt](sun_ascii.txt) and prints its Sun logo and
the HELIOS name on stderr at startup, keeping guest serial bytes on stdout.

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
`--ram-mib` changes the default 128 MiB allocation (maximum 256 MiB).
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

`boot` installs the SS-5 bank layout and a read-only 1 MiB PROM aperture at `0x70000000`, starts
PC/nPC at 0/4, and selects PROM for instruction-space accesses through boot
mode. Ordinary data accesses use the physical map, including console serial at
`0x71100000` and NVRAM/RTC at `0x71200000`. `boot` and `serial-demo` send TTYA
bytes to stdout and diagnostics/traces to stderr. Input comes from a file;
interactive terminal input is pending. NVRAM uses an 8192-byte raw image and is
saved at the step limit or an execution error when `--nvram-out` is supplied.
The sun4m harness advances device time by 100 ns per completed step, including
trap/interrupt entry. This is a functional scheduling convention, not CPU cycle
timing. The library also exposes explicit time advancement. A working firmware
prompt remains pending. See [Memory and reset](docs/memory.md),
[Devices](docs/devices.md) and [Interrupts and timers](docs/interrupts-timers.md).
Guest reset behavior and current watchdog limits are in [System reset](docs/reset.md).
For `boot`, RAM must fit eight 8 MiB or 32 MiB DSIMMs; larger modules occupy
low slots first. The default is four 32 MiB modules and four empty slots.
See [DRAM and TLB diagnostics](docs/dram-tlb.md) for mirrors, empty-bank reads
and the distinction from the contiguous RAM used by synthetic harnesses.

## Development

- [HACKING.md](HACKING.md): architecture, invariants and validation commands.
- [SPARCstation 5](docs/sparcstation5.md): target and reference memory map.
- [Memory and reset](docs/memory.md): SRMMU, ASIs, fault behavior and PROM mapping.
- [Fault capture](docs/fault-capture.md): SBus errors, asynchronous stores and memory-request capture.
- [Cache diagnostics](docs/cache.md): diagnostic storage, invalidation and functional limits.
- [DRAM and TLB diagnostics](docs/dram-tlb.md): DSIMM layout, diagnostic entries and replacement controls.
- [System reset](docs/reset.md): reset-status register, guest reset and watchdog limits.
- [Devices](docs/devices.md): console serial, IDPROM, NVRAM persistence and RTC.
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

This short probe now reaches its 10000-step budget in OBP's timed keyboard
polling loop. With a 30000000-step budget, OBP passes the former memory-bank,
TLB-diagnostic and IOMMU-enable stops. It completes 25373701 instructions with
two diagnostic bus traps, no interrupts and no resets. The earlier fault-status
warning is gone. It now stops at PC `0x70002454` on the unimplemented floating-point
load `LDDF`; FPU support is the next POST boundary. There is no serial output or
firmware prompt in this run.
The fetched image is hash-verified and stays ignored.
OpenBIOS remains a separate compatibility test until it is proved working.

