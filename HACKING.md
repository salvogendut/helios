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
| `src/cpu.rs` | Integer interpreter, register windows, privileged state, traps and error state |
| `src/bus.rs` | Physical bus contract, RAM/ROM mappings and external byte import |
| `src/mmu.rs` | ASI dispatch, SRMMU translation, TLB, permissions and fault registers |
| `src/machine.rs` | Bare-metal/reset construction, RAM/PROM limits and demo images |
| `src/main.rs` | CLI parsing, image files, trace printing and CPU step budget |
| `tests/` | Architectural and boundary behavior through the public library |

`Machine::bare_metal` deliberately describes the current execution environment.
It creates a CPU at a requested entry address and zeroed RAM at address zero.
It does not perform SS-5 power-on, install a PROM alias or initialize devices.
The CPU starts with S=PS=1, ET=0, CWP=0, WIM=0, TBR=0 and Y=0. These are
harness conventions. `Machine::sun4m` installs a raw PROM and selects CPU/MMU
reset; `Machine::reset` resets those components while retaining RAM and ROM.
See [memory.md](docs/memory.md) for the reset state and platform limits.

## Data path

Follow IRIS's word-transparent convention: a host `u32` is a guest bit pattern.
RAM/ROM store these words. The CPU and bus pass the same values without native
endian conversions. External images contain big-endian bytes; image import is
the conversion boundary. Narrow accesses select byte/halfword lanes with shifts
and stores carry bit masks. Future disk and network adapters need similarly
explicit byte boundaries. Do not scatter byte swaps through instruction code.

Virtual/effective CPU addresses are 32-bit wrapping values. The physical bus
uses `u64` so later MMU implementations can expose wider physical addresses
without rewriting device interfaces. The microSPARC-II MMU currently drives
31 physical address bits. CPU accesses pass through ASI dispatch; the physical
bus remains responsible only for physical mappings and device access.

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

## Privileged state and synchronous traps

PSR reads combine the canonical ICC and CWP with supervisor/trap control, PIL
and EF. MB86904 implementation/version bits are fixed at `0x04000000`.
Reserved bits and EC read zero; there is no auxiliary coprocessor. EF is retained
for the intended hardware FPU, whose instructions remain unimplemented. PIL is
stored but external interrupt delivery is not implemented yet.

RD/WR PSR, WIM and TBR require supervisor mode, even when a read targets `%g0`.
Y is accessible in user mode. State writes XOR their operands. WRPSR rejects an
unimplemented CWP before changing any state; WRWIM retains eight bits; WRTBR
updates only the base and preserves TT. Writes take effect immediately. V8
permits a zero-to-three-instruction write delay; guest fixtures use the portable
three-instruction spacing. STBAR completes immediately because the current
single-owner memory implementation has no queued stores.

An instruction that faults does not commit its normal effects. MMU fault
registers and page-table R/M updates are separate architectural side effects;
a completed translation can update R/M before a physical target access fails.
With ET=1, trap entry saves S in PS, sets S, clears ET, decrements CWP regardless of WIM,
saves faulting PC/nPC in the new window's `%l1`/`%l2`, sets TBR.TT and transfers
to TBR with nPC=TBR+4. The saved nPC retains a pending branch target when a
delay-slot instruction faults. ICC, PIL and EF are preserved. The architecture
rules are described in the [SPARC V8 manual](https://sparc.org/wp-content/uploads/2014/01/v8.pdf.gz),
chapters 4 and 7 and appendix C.8.

The implemented trap types are:

| Cause | TT |
| --- | --- |
| MMU instruction / data access exception | `0x01` / `0x09` |
| UNIMP or invalid WRPSR CWP; supervisor RETT with ET=1 | `0x02` |
| Privileged instruction in user mode | `0x03` |
| Window overflow / underflow | `0x05` / `0x06` |
| Misaligned memory access or jump | `0x07` |
| Physical instruction / data bus error | `0x21` / `0x29` |
| Taken Ticc | `0x80` OR low seven bits of the operand sum |

Physical access errors include unmapped memory and rejected ROM stores. MMU
access exceptions use distinct fault types and vectors `0x01`/`0x09`. A bus
failure while accessing a page table becomes an MMU translation fault; a bus
failure at the translated target remains a physical access error. Deferred store
errors, external interrupts, FPU traps and general illegal-opcode classification are
not implemented. Missing emulator instructions, including unimplemented ASRs,
still produce an `UnsupportedInstruction` host stop. Unimplemented control
spaces produce `UnsupportedMemory`; neither is converted into a guest trap.

With ET=0, a precise exception latches error state without ordinary trap entry:
no window rotation or saved locals, and no change to TBR.TT except for a failing
RETT. Further steps return the same fault without fetching. Reconstructing the
bare-metal CPU or performing CPU/MMU reset clears that latch. Device reset
will be added as devices are implemented.

RETT checks privilege, ET, the destination WIM bit, then target alignment before
mutating state. On success it increments CWP, restores S from PS, sets ET and
performs a delayed jump. A handler uses `jmpl %l1, %g0; rett %l2` to retry the
faulting instruction, or `jmpl %l2, %g0; rett %l2+4` to skip it. A failing RETT
with ET=0 updates TT and enters error state. See V8 appendix B.26.

`Cpu::step` and `Machine::step` return `Step::Executed` or `Step::TrapTaken`;
the latter includes the original fault and vector and does not execute handler
code yet. `Err(CpuFault)` denotes an emulator stop or latched CPU error state.
Fetch faults have no instruction word. CLI `--steps` bounds both instruction
completion and trap entry, and reports their counts separately.

## Validation

```sh
cargo fmt --all --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
cargo run --locked --offline -- demo
cargo run --locked --offline -- trap-demo --trace
```

Tests cover big-endian lanes, mapping bounds, ROM protection, register windows,
integer flags, all Bicc conditions across representative flag states, delay and
annul behavior, CALL/JMPL, loads/stores, address wrapping and precise stops.
Trap tests cover state-register masks, privilege checks, WIM bit seven, TBR.TT
preservation, Ticc conditions, RETT priority, fault recovery in a branch delay
slot and nested faults entering error mode. The hand-encoded loop and trap
programs are end-to-end tests without an assembler dependency. Memory tests
cover all ACC/access-type combinations, three page sizes, contexts/global
entries, stale TLB mappings, flush/probe, R/M, fault priority, no-fault mode and
PROM reset. A guest handler installs a missing PTE and retries a faulting load
in a branch delay slot; another guest switches from PROM to translated RAM.
These are local regression checks, not a SPARC conformance suite or a completed
QEMU/MAME differential comparison.

For new instructions, add small programs with independently derived expected
state, including exceptional paths. Later differential tests should compare
architectural state at agreed boundaries and record the reference revision and
CPU model. Firmware tests must additionally record the firmware hash, machine
configuration, serial transcript and stopping condition. Keep performance
measurements separate from correctness checks.

