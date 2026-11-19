# Native JIT prototype design

Status: final candidate on `feat/issue-5-jit-prototype`; default off, not
merged. Parking integration is separate and unchanged.

## Backend and scope

The native backend is the optional `helios-jit-native` crate selected by the
`jit-native` feature on Unix/x86-64. It emits x86-64 code for straight-line
SETHI and basic integer ALU operations (ADD/SUB/AND/OR/XOR and their
condition-code forms). Branches and delay slots, loads/stores, register-window
changes, traps, ASIs, FPU, privileged/MMU controls and unknown encodings all
fall back to the interpreter.

The core crate keeps `unsafe_code = "forbid"`. The isolated backend owns its
small mmap/mprotect and calling-boundary audit; executable pages are RW then
RX and are released on failure or drop. The feature is absent from normal
builds. On other host architectures every block reports unsupported.

## Admission and invalidation

A native block is admitted only when all of the following hold:

- no installed devices and no secondary CPU;
- MMU and boot translation are disabled, rechecked at every block boundary;
- the complete candidate span lies below `MmuConfig::physical_mask` and inside
  writable ordinary RAM;
- PC/nPC form a sequential straight-line entry, and the block fits the
  remaining instruction budget;
- every instruction word decodes to the supported subset.

The call-local cache is bounded to 64 compile attempts and keyed by PC plus bus
mutation epoch. Unsupported branch words are negatively cached. Any guest store
or MMU-control write that changes the key/admission is observed before the next
block. There is no persistent code cache and no speculative MMU fetch: the
native path reads only admitted untranslated RAM words.

## Architectural boundaries

The core snapshots the 32 active registers and ICC, calls one generated block,
copies the architectural state back, advances PC/nPC only for the executed
straight-line count, and advances the existing clock once per instruction.
Pending resets, latched faults, interrupts, traps, nonsequential entries,
device machines and MMU-enabled execution use the established interpreter
boundary. Compilation does not execute an instruction or change MMU state.

## Historical decoded experiment

Commit `3ac11c0` was a decoded Rust operation loop, not a native JIT. Its
measurements remain historical evidence only. The `jit` feature and decoded
tests are retained for comparison/fallback coverage, but the native backend is
the milestone candidate.
