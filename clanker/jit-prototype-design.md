# Bounded safe JIT prototype design

Status: prototype on `feat/issue-5-jit-prototype`; default off.

## Scope

`helios` gains an opt-in `jit` feature and a safe-Rust block engine. The first
milestone compiles only straight-line SETHI and integer ALU operations
(including condition-code variants). Branches and delay slots, loads/stores,
register-window changes, traps, ASIs, FPU, privileged/MMU instructions and all
unknown encodings stop a block. The existing interpreter executes the stopping
instruction and any instruction outside a compiled block.

The engine emits no host machine code and uses no executable memory. It is a
runtime basic-block decoder over owned `Vec<Op>` storage, so the crate keeps
`unsafe_code = "forbid"`. The normal `Machine::step` path is unchanged.

## Fetch validation and invalidation

`Machine::run_jit` compiles a bounded block (at most 16 instructions) only for
a machine with no installed devices in this milestone. Each instruction word is
fetched through the CPU MMU with the current ASI and access context, so fetch
permission and translation faults fall back to the interpreter. The block is
never executed past an unsupported word.

The block cache is local to one `run_jit` call and conservative:

- a bus mutation-epoch change invalidates it;
- an MMU/TLB generation or privilege-state change invalidates it;
- a fallback interpreter step invalidates it;
- cache size is bounded (64 entries) and no block survives the call.

This removes stale-code and stale-translation hazards without claiming a
production code-page generation scheme. Device-equipped machines use the
interpreter fallback in this milestone because their fetch timing includes PROM
wait accounting; later work may admit RAM-only code after measuring that path.

## Architectural boundaries

Each JIT instruction advances PC/nPC exactly as the interpreter (`pc <- nPC`,
`nPC <- pc + 4`) and reuses the interpreter's ALU/condition-code implementation.
Clock and bus timing advance once per instruction, before the next boundary.

`run_jit` checks the software-reset request, device service, error latch and
interrupt lines before each instruction; any of those boundaries stops the
block and returns to the interpreter. Traps, window changes and fallback
instructions are never speculated. Device time, MMU state and guest timing are
not changed by compilation.

## Validation and measurement

The differential validator compares architectural snapshots after identical
fixed-work runs through `step` and `run_jit`. Fixtures cover supported ALU
results, self-modifying code, remapping, permission removal, unsupported
instruction fallback and device/interrupt fallback. Release measurements use
the same fixed ALU workload with the feature off and on, report compile time
and block counts separately from steady execution, and never claim a Solaris
boot speedup.
