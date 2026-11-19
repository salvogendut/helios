# Native JIT prototype result

Branch: `feat/issue-5-jit-prototype`, based on merged main `5fcffdc`.
Feature: `jit-native`, default off; Unix/x86-64 only. Core remains
`unsafe_code = "forbid"`.

## Final native candidate

The guard-patched native backend executes only device-free, single-CPU,
MMU-disabled writable-RAM blocks. It rechecks live MMU/boot state at every
block boundary, enforces the CPU physical-address mask over the whole candidate
span, and refuses a cached block that exceeds the remaining budget. Everything
else falls through the interpreter. See [the design](jit-prototype-design.md).

Guard-patched identical release builds, alternated order:

| Workload | Interpreter | Native | Runs |
| --- | ---: | ---: | --- |
| Fixed ALU, 25M outcomes | 41.57 ns/outcome | 35.21 ns/outcome | 3 pairs |
| Fixed memory, 7M outcomes | 54.59 ns/outcome | 55.195 ns/outcome | 2 pairs |

That is about **1.18x** on the bounded ALU workload and **1.1%** additional
memory fallback time. Native compilation was separated at 26.9-72.5
microseconds; all checksums and instruction counts matched. These are
synthetic fixed-work results and do **not** establish a faster Solaris boot or
workstation acceleration.

## Validation

The promoted native fixtures cover the four review edges (budget fitting,
MMU-enable revocation, physical-mask identity within a block and at entry), a
900-case independently calculated ALU/ICC matrix, the earlier review fixtures
through `run_jit_native`, self-modification, reset/error and device fallback.
The complete normal/native suites, formatting, Clippy and demos are run for the
final candidate: **814 normal / 850 with `jit-native`**, all passing. The
900-case matrix checks carry/borrow/overflow, `%g0`, aliases and all register
class destinations against the interpreter.

## Historical decoded experiment

`3ac11c0` was a decoded Rust operation loop, not a native backend. It reported
about 1.37x on ALU and a 2.56x memory regression; those measurements remain
historical and must not be presented as the JIT candidate. The retained `jit`
feature is comparison/fallback coverage only.

## Boundaries

No branch, memory, SMP, parking, MMU or device acceleration is added by this
correction. Register marshalling and persistent caching are follow-ups, not
part of this acceptance gate.

## Acceptance

Accepted for merge after the native review. Merged to `main` as `9ac1ed5`.
This closes issue #5 for the bounded native milestone. Broader JIT coverage,
persistent caching, register-marshalling improvements, memory JIT, SMP and
additional parking targets belong in follow-up issues.
