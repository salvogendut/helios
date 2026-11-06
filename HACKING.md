# Hacking on Helios

## Principles inherited from IRIS

Helios aims for functional workstation emulation: registers, memory, traps,
interrupts, DMA, observable ordering and handshakes must behave correctly. Cycle
accuracy is not the initial goal. Timing that guest software depends on still
needs a defined model. A successful firmware probe is evidence for that probe,
not evidence that an entire device or operating system works.

Keep the emulation core independent of host UI and process lifecycle. CPU and
devices report outcomes to their owner; they do not exit the process. Machine
construction wires hardware together. Future frontends should call this same
library rather than duplicate machine setup. JIT execution, device workers and
host acceleration come after a tested interpreter and observable baseline.

IRIS's mature devices use per-device threads and locks. This first Helios slice
uses one owner and synchronous stepping, which makes CPU tests deterministic.
As workers become useful, keep ownership per device, bound queues, and preserve
guest ordering and back-pressure. Do not put a global mutex around the machine
or assume host scheduling is guest timing.

## Current layout

| File | Responsibility |
| --- | --- |
| `src/cpu.rs` | Integer interpreter, register windows, ICC, PC/nPC and fault records |
| `src/bus.rs` | Physical bus contract, RAM/ROM mappings and external byte import |
| `src/machine.rs` | Bare-metal construction, target RAM limits and demo image |
| `src/main.rs` | CLI parsing, image files, trace printing and instruction budget |
| `tests/` | Architectural and boundary behavior through the public library |

`Machine::bare_metal` deliberately describes the current execution environment.
It creates a CPU at a requested entry address and zeroed RAM at address zero.
It does not perform SS-5 power-on, install a PROM alias or initialize devices.
The CPU's WIM starts at zero for these tests; that is a harness convention,
not a microSPARC-II reset claim.

## Data path

Follow IRIS's word-transparent convention: a host `u32` is a guest bit pattern.
RAM/ROM store these words. The CPU and bus pass the same values without native
endian conversions. External images contain big-endian bytes; image import is
the conversion boundary. Narrow accesses select byte/halfword lanes with shifts
and stores carry bit masks. Future disk and network adapters need similarly
explicit byte boundaries. Do not scatter byte swaps through instruction code.

Virtual/effective CPU addresses are 32-bit wrapping values. The physical bus
uses `u64` so later MMU implementations can expose wider physical addresses
without rewriting device interfaces. The current harness zero-extends effective
addresses directly onto the bus; it does not implement virtual translation.

Mappings are nonempty, word-aligned, nonoverlapping half-open ranges. RAM starts
at zero; ROM writes fail. Image loading validates the entire destination within
one RAM region before modifying it. Unmapped, read-only and misaligned accesses
are distinct errors. Failed bus accesses must not mutate memory or devices.
Masked stores do not call the device read path. RAM's default narrow accessors
are unsuitable for some MMIO registers; those devices must override them.

## CPU state and control flow

Globals are shared across all eight windows and `%g0` always reads zero. Each
window owns eight outgoing and eight local registers; its incoming registers
alias the next window's outgoing registers. SAVE decrements CWP and RESTORE
increments it modulo eight. Both read their operands before rotation and write
the result after rotation. The WIM bit of the destination window blocks rotation
and produces an overflow/underflow fault.

Successful ordinary instructions set PC to the old nPC and advance nPC by four.
Branches replace nPC with the target so the delay slot executes first. A
not-taken annulled branch skips its slot; BA,a skips it even when taken.
Annulled slots are skipped without a bus fetch in this initial interpreter;
instruction-fetch side effects will need review with the MMU/cache model.
CALL and JMPL save the address of the control-transfer instruction as the link.

Addition's C bit is carry-out; subtraction's C bit is borrow. Logical `cc`
instructions clear C and V. Non-`cc` operations leave ICC unchanged. Register
values and addresses use explicit wrapping arithmetic where the ISA requires it.

Faults preserve CPU state, including both PCs for faults in a delay slot.
Unsupported instructions are reported as unsupported, not silently executed as
NOPs or confused with implemented architectural trap entry. Fetch faults have
no instruction word. Adding trap handling must preserve precise fault context
and distinguish unsupported emulator behavior from architectural illegal
instructions. No guest-visible trap entry exists yet.

## Validation

```sh
cargo fmt --all --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
cargo run --locked --offline -- demo
```

Tests cover big-endian lanes, mapping bounds, ROM protection, register windows,
integer flags, all Bicc conditions across representative flag states, delay and
annul behavior, CALL/JMPL, loads/stores, address wrapping and precise stops.
The hand-encoded loop is an end-to-end test without an assembler dependency.
These are local regression checks, not a SPARC conformance suite or a completed
QEMU/MAME differential comparison.

For new instructions, add small programs with independently derived expected
state, including exceptional paths. Later differential tests should compare
architectural state at agreed boundaries and record the reference revision and
CPU model. Firmware tests must additionally record the firmware hash, machine
configuration, serial transcript and stopping condition. Keep performance
measurements separate from correctness checks.

