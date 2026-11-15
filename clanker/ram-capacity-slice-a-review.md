# Configurable RAM: slice A review

Reviewed 2026-11-15 on `feat/configurable-ram`, against the uncommitted
`DramLayout` / `InstalledVideo` implementation following `66acae4`.

## Verdict

No blocking functional defect found in slice A. The shared layout and
construction work is ready to support slice B. This is not acceptance of
large-RAM Solaris boots or completion of issue #3: the high-memory DMA and
frontend changes remain outstanding as planned.

The constructors validate processor count, profile/video RAM geometry and PROM
length before allocating DRAM. Default 128 MiB layouts remain unchanged.
The default SS-20 constructor rejects 512 MiB with SX; the explicit no-VSIMM
path constructs a real eight-bank 512 MiB machine. The seven-bank 448 MiB SX
configuration leaves bank 7 empty. Overflow-sized inputs fail without panicking.

## Independent checks

Review fixtures are retained in the ignored directory
`local/review-ram-slice-a-2026-11-15/independent.rs` with a small Cargo manifest
and `independent.log`. All eight tests passed, run serially to bound allocation:

- Boolean reachability per socket independently checks SS-5 enumeration and
  construction acceptance for every integer MiB total from 0 through 272.
- The same independent method checks SS-20-style populations for every socket
  count 0 through 8 and every integer MiB total from 0 through 528, plus invalid
  socket counts. It does not duplicate production's nested module-count loops.
- Arithmetic extremes, wrong byte granularity and socket overflow are rejected.
- A 512 MiB no-VSIMM SS-20 with two processor modules has distinct storage at
  physical 0, `0x10000000` and `0x1ffffffc`. Whole reset retains that storage
  and capacity; `0x20000000` is outside DRAM.
- A 448 MiB SS-20/SX retains the last populated word at `0x1bfffffc`; writes
  at `0x1c000000` retain the existing empty-bank behavior and do not create DRAM.
- A 512 MiB Axil model stores and retrieves the final word of bank 7. This is
  implementation evidence, not independent proof of real Axil module sizes.
- Mixed totals preserve sparse bank layouts (40 MiB SS-5; 80/112 MiB SS-20).
- Unsupported video/CPU combinations and oversized PROMs are rejected.

These are physical-memory/construction tests. They do not establish translated
DMA, firmware memory discovery, guest allocation behavior or desktop acceptance.

## Validation rerun

| Check | Result |
| --- | --- |
| Focused `ram_layout`, `dram`, `machines`, `smp_scheduler` suites | 43/43 |
| Independent review fixtures | 8/8 |
| Normal full core suite | 611 unique tests; 612 success lines |
| Full core suite with `perf-counters` | 613 unique tests; 614 success lines |
| `cargo fmt --all --check` | Pass |
| Clippy `--all-targets -- -D warnings`, normal and counters | Pass |
| `demo`, `trap-demo`, `serial-demo`, `timer-demo` | Pass |

The extra line in each full suite is the isolated MXCC child test, not another
unique regression. The first sandboxed full-suite attempt failed when the
existing UDP/DNS tests tried to bind loopback sockets; a host-permitted rerun
passed both full configurations. Logs are in the review directory.

Heliacal was inspected for direct callers, but its build/tests and guest boots
were not rerun in this review. No production source or submitted tests were
changed by the reviewer. The review created only this note/index entry and
ignored fixture/evidence files. The temporary fixture build directory was removed;
no source disk, firmware or live configuration was modified.

## Follow-ups to fold into the next revision

1. **Strengthen the tracked enumeration oracle.**
   `expected_ss5_totals` and `expected_ss20_totals` in `tests/ram_layout.rs`
   duplicate the production enumeration loops. The greedy layout checks are
   useful, but the exact-list comparison cannot catch a shared enumeration
   mistake. Promote the independent reachability method (or independently
   derived literal sets) and check rejected totals as well as accepted totals.
   Preserve the targeted maximum/bank-7 assertions. If promoting the maximum
   allocation checks too, group or serialize them so they do not all hold
   large guest memories concurrently.
2. **Keep the Axil assumption visible in human documentation.**
   The submitted agent plan correctly distinguishes the reported eight sockets
   from the SS-20-derived 16/32/64 MiB module assumption. Carry that distinction
   into `docs/dram-tlb.md` / `docs/machines.md`, with a pointer from README and
   `docs/references.md`. The new capacity descriptions otherwise read as an
   independently confirmed Axil hardware specification. Describe the implemented
   model and retain the pending evidence question explicitly.
3. **Report unique test counts.** Use 611/613 for this revision; 612/614 are raw
   success-line counts. Recompute after promoting any additional tests.

## Slice B work order

Continue the existing [shared plan](ram-capacity-plan.md#slice-b--core-dma-and-headless-cli).
Keep Heliacal changes for slice C. The next functional gate is memory usable by
devices as well as CPUs:

1. Audit and fix standalone IOPTE fetches (`src/iommu.rs`) and translated DMA
   targets (`src/dma.rs`) above `0x10000000`, using board-aware range/span checks.
   Preserve SS-5 integrated semantics and distinguish installed storage, mirrors,
   empty banks, the DRAM aperture and device windows.
2. Resolve bypass rules separately in `src/devices/io_control.rs`; do not globally
   replace every 256 MiB constant. Retain high-IBAR non-aliasing, flush ownership,
   device attributes and fault behavior.
3. Correct `SystemBus::install_ss5_dram` overlap validation for the incoming
   layout's full aperture; test both installation orders and unchanged state on
   rejection.
4. Expose the deliberate no-VSIMM construction path in headless CLI hardware
   selection before allocation. Adding a CGsix card must not leave an unintended
   SX/VSIMM installed. Validate the final option combination independently of
   argument ordering, retaining the default 128 MiB configurations.
5. Promote regressions for high IOPTE tables and high targets separately/together,
   SCSI and Ethernet transfers, flushes, bad targets and low/high non-aliasing.
   Return the slice with focused evidence and the normal required checks.

Keep issue #3 open. There is no merge or full-capacity OS acceptance claim yet.
