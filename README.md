# Helios

**HELIOS — Hardware Emulator for Legacy Implementations Of SPARC.**

Named after the Greek sun god, Helios is a Rust emulator for Sun SPARC
workstations. The first full-system target is the **SPARCstation 5 (sun4m,
microSPARC-II / SPARC V8)**. The longer-term
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

Unsupported emulator behavior remains a host stop, separate from guest traps.
Implementation-specific ASRs, broader illegal-opcode classification, MMU/cache
diagnostics, FPU, external interrupts, timers, serial, NVRAM, SCSI, networking
and graphics remain to be implemented. There is no firmware or OS compatibility
claim.

## Build and run

Use Rust 1.74 or newer with Cargo. The core needs no host GUI libraries or
downloaded crates; nightly features are not required.

```sh
cargo test --locked --offline
cargo run --locked --offline -- info
cargo run --locked --offline -- demo
cargo run --locked --offline -- demo --trace
cargo run --locked --offline -- trap-demo --trace
```

The demo executes the hand-encoded program in [examples/sum.s](examples/sum.s),
adds 1 through 10, and verifies that guest RAM at `0x100` contains **55** after
43 instructions. It requires no ROM image or SPARC assembler.

The [trap demo](examples/trap.s) installs a trap table, enters user mode, executes
`TA 7`, runs a supervisor handler and returns through `JMPL`/`RETT`. It verifies
the return state after 14 CPU steps: 13 completed instructions and one trap entry.

Run your own raw big-endian program with an explicit CPU step budget:

```sh
cargo run --release -- run local/program.bin --load-address 0x1000 --steps 1000 --trace
```

Entry defaults to the load address. `--entry` selects another address;
`--ram-mib` changes the default 64 MiB allocation (maximum 256 MiB).
The bare-metal harness maps only RAM at physical address zero, with translation
and boot mode disabled. microSPARC-II physical addresses are 31 bits wide.
`run` accepts raw bytes, not ELF executables or workstation firmware images.
Reaching the budget is a successful host stop, not a guest halt instruction.
Each completed instruction or trap entry counts as one step. With `PSR.ET=1`,
implemented exceptions enter the guest handler; with `PSR.ET=0`, they enter CPU
error state and exit with status 1. Unsupported instructions also exit with
status 1, without a guest trap. The harness starts in supervisor mode with traps
disabled; guest code must install a trap table and enable traps to handle them.

Inspect a raw PROM from CPU/MMU reset with a bounded run:

```sh
cargo run --release -- boot local/prom.bin --steps 1000 --trace
```

`boot` installs RAM and a read-only 1 MiB PROM aperture at `0x70000000`, starts
PC/nPC at 0/4, and selects PROM for instruction-space accesses through boot
mode. Ordinary data accesses still reach RAM. This is a memory-system harness;
SS-5 peripherals and a working firmware prompt are still pending. See
[Memory and reset](docs/memory.md) for implemented ASIs and known limits.

## Development

- [HACKING.md](HACKING.md): architecture, invariants and validation commands.
- [SPARCstation 5](docs/sparcstation5.md): target and reference memory map.
- [Memory and reset](docs/memory.md): SRMMU, ASIs, fault behavior and PROM mapping.
- [Roadmap](docs/roadmap.md): milestones from this harness to a usable system.
- [References](docs/references.md): specific MAME/QEMU sources and provenance.

Keep local firmware, disks and scratch programs under the ignored `local/`
directory. The repository contains no firmware or operating-system images.

