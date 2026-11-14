# SS-20 and Axil 320 SMP implementation handoff

2026-11-14. Issue: [helios#2](https://github.com/salvogendut/helios/issues/2).
Branch: `feat/sun4m-smp`. This is a design and implementation work order;
end-to-end SMP has not been implemented or accepted. Slices 1 and 2A are
independently validated through core revision `a94f0aa` and approved by the
user for an incremental foundation merge. Production machines remain
single-CPU. Slice 2B's standalone-IOMMU ownership is now separately accepted
on this branch; continue with [slice 2C](smp-slice2c.md), with review between
slices. Issue #2 remains open.

## Scope and baseline

Start with **two identical single-CPU SuperSPARC/MXCC modules**, one in each
MBus slot, on SS-20 and Axil 320. Keep the existing CPU model and RAM size.
Default to one CPU, and keep SS-5 strictly uniprocessor. Four CPUs, dual-CPU
modules, heterogeneous modules, hyperSPARC, RAM expansion, JIT and host-parallel
execution are separate work. The first SMP implementation should run on one
host thread with deterministic guest scheduling. SMP guest support does not
by itself promise higher emulation throughput.

Reviewed bases:

- Helios `3875ba670d52b5231b35eadf9a39848b2fd95658`, on `main`.
- Heliacal `95b6e100e85322f2fff16503da2596653cde9aa2`, on `main`.
- Last accepted validation: 514 normal core tests, 516 with `perf-counters`,
  Heliacal 29 tests, formatting, Clippy, demos, and SS-5/SS-20/Axil Solaris 2.6
  boots. These are baseline results, not new SMP validation.

Read `AGENTS.md`, `README.md`, `HACKING.md`, `docs/references.md`,
`docs/machines.md`, `docs/memory.md`, `docs/interrupts-timers.md` and
`docs/reset.md` before changing behavior. Retain the recent diagnostic-cache,
visual scanout-generation and interrupt-cache improvements.

## Evidence and architectural decisions

### Processor indexes are not interchangeable

The Sun-4M architecture's MBus connector tables assign two master IDs to each
slot. For the proposed two single-CPU modules, use this explicit topology:

| Host storage index | MBus connector | Hardware processor number | MID | MXCC control PA | Architected port PA | Port identity |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 0 | 0 | `8` | `0xff8c00a00` | `0xff8fffffc` | `0x08000104` |
| 1 | 1 | 2 | `a` | `0xffac00a00` | `0xffafffffc` | `0x0a000104` |

Hardware banks are indexed by processor number: the second populated timer
and interrupt pages are `0xff1302000` and `0xff1402000`, respectively.
Hardware processor slots 1 and 3/MIDs 9 and B remain absent. Do not compact
guest-visible IDs to 0 and 1, or derive every hardware address from a vector
index. Confirm SS-20 and Axil firmware discovery against this topology before
advertising SMP support; the connector tables are architectural evidence,
not an observed two-CPU boot of either Helios profile.

Each CPU must have independent integer/window/FPU/trap state, MMU context,
TLBs, fast translation caches, diagnostic views and counters. Each populated
single-CPU module must have its own MXCC state. Local ASI 2 accesses identify
the executing module; physical MXCC accesses identify the addressed module,
even when another CPU performs them. Preserve word/doubleword semantics,
read-only identity, stream completion, reset and recursive-window error
handling across both modules.

### Startup is hardware plus original OBP

Sun-4M section 5.1.2 defines the arbiter enable register at `0xfe0001008`.
Its processor-2 enable starts clear, so the second module is initially denied
arbitration. Processor 0 remains enabled; the processor-1 reset enable serves
split-requester configurations and is not evidence of another installed CPU.
Use a profile-specific reset value: do not change microSPARC-II arbitration
semantics to match the standalone MBus controller.

Implement the documented arbitration/startup behavior, then trace original
OBP enabling the second module, discovering it and parking/releasing it.
Distinguish arbitration denial, firmware spin loops, CPU error mode, module
reset and whole-machine reset. A CPU with no eligible bus execution must not
consume fabricated instructions. Define which hardware event releases it.
Do not assume any interrupt may release an arbitration-disabled module.

Pinned QEMU powers off CPUs other than index zero and clears its halted state
on interrupt assertion. It also reports a CPU count through `fw_cfg`. Those
are useful comparison points, not a substitute for the original Sun firmware
interface. Do not inject CPU device-tree nodes, patch OBP/Solaris, add a
QEMU-specific startup port, or jump a CPU directly into a Solaris routine.

The archived Solaris 2.6 `sun4m/os/mp_startup.c` supplies a concrete debugging
path: `start_other_cpus()` enumerates firmware `mid` properties, converts
them with `MID2CPU`, optionally maps `mailbox`, and calls `prom_startcpu`
with the context table and a per-CPU entry address. It waits for the secondary
startup handshake. The emulator must let the firmware and guest execute this
protocol. A firmware CPU node alone does not demonstrate a working CPU.

### Interrupts and timers need local and shared state

- Keep shared device sources and global masks at the system controller.
  Implement the Current Interrupt Target register at `0xff1410010`; ordinary
  undirected device interrupts go to that hardware processor number.
- Give each hardware processor bank independent software interrupt bits,
  processor timer pending state and level-15 broadcast acknowledgement.
  Set/clear writes must affect the addressed bank, regardless of the sender.
  Acknowledge on CPU 0 must not erase CPU 2's pending interrupt.
- Hard level-15 sources are broadcast and latched per processor. Follow the
  manual's source-mask, mask-all and acknowledgement rules. Explicitly resolve
  the current implementation/QEMU difference in software interrupt masking;
  do not silently copy a single-CPU shortcut into the SMP path.
- CPU-local timer banks share elapsed machine time, but have independent
  count/limit/run/mode state and acknowledgements. The system timer remains
  one shared device. Timer configuration bits correspond to hardware slots,
  so the second populated CPU uses bit 2, not bit 1.
- Cache delivered PIL sets per processor and invalidate the affected sets
  when routing, masks, software bits, timers or broadcast latches change.
  Keep pending state distinct from whether PSR.ET/PIL permits CPU entry.

### One shared machine clock

Do not implement SMP by calling the old whole-machine `step()` once per CPU.
That would advance timers, SCC/mouse bytes, Ethernet, SCSI and retrace twice
as fast for the same elapsed per-CPU execution time.

Define the scheduler contract before implementing it. A suitable design is
per-CPU next-ready times with stable hardware-ID tie ordering and one shared
device clock. Preserve the one-CPU functional quantum (`STEP_NS = 100 ns`),
existing PROM wait handling, instruction-boundary interrupts and stop/error
atomicity. Account for individual CPU stalls without multiplying elapsed
device time; specify when DMA and other device service occurs. State what
happens if all CPUs are arbitration-blocked or otherwise non-runnable.

Expose the processor identity for every executed instruction, synchronous
trap, interrupt entry and error stop. A scheduler/device-only advance is not
an instruction. Specify aggregate versus per-CPU step budgets and counters,
and update CLI/harness/frontend consumers consistently. Scheduling must not
depend on frame presentation, UI focus, host wall time or thread races.

### Memory, atomics and DMA

Single-threaded instruction interleaving can preserve indivisible `SWAP` and
`LDSTUB`; keep the entire guest atomic operation, its translation and fault
handling within one scheduling boundary. Preserve PC/nPC and window aliases.
Shared coherent RAM is sufficient for the current functional cache model;
do not invent cache-line timing. Test visibility and ordering between CPUs,
DMA, MXCC stream transfers and framebuffer memory.

Sun-4M section 7.1.4 makes PDC/TLB flushes local to a module. Solaris performs
cross-calls for cooperative maintenance. Keep each CPU's MMU/diagnostic and
fast-cache invalidations local unless a separately documented mechanism says
otherwise. Do not flush all CPU TLBs whenever a shared PTE changes: that would
hide missing shootdowns. Verify stale translation, remote IPI, remote flush
and subsequent new translation as a complete sequence. Preserve atomic PTE
reference/modified-bit updates and per-CPU fault registers.

DMA ownership needs an explicit design. Today `src/dma.rs` translates through
`self.cpu.mmu`, and MMU paths consume queued IOMMU flushes. This combines
SS-5's integrated shared CPU/I/O TLB with SS-20's standalone-IOMMU path.
For SS-20/Axil, make the standalone I/O translation/flush owner independent of
whichever CPU is scheduled; preserve the documented SS-5 shared-TLB behavior.
Do not let an arbitrary CPU drain invalidations that another required
consumer then misses. Test DMA while CPU 0 is parked and CPU 2 changes an
IOMMU mapping, including stale mappings, flushes and error attribution.

## Implementation map

| Area | Current single-CPU assumption | Required work |
| --- | --- | --- |
| `src/machine.rs` | Public `cpu`, one reset path, one step clock, one IRQ set | Explicit topology, authoritative per-CPU storage, scheduling and CPU-tagged outcomes; preserve default UP behavior |
| `src/cpu.rs`, `src/mmu.rs` | CPU/MMU cloning can share the MXCC `Rc`; caches/counters belong to the current sole CPU | Construct each CPU afresh; prove local state and reset isolation; define counter snapshots by CPU |
| `src/devices/mxcc.rs` | MID 8 constants and one physical window | Identity and physical decoding by populated module; keep all aliases coherent |
| `src/system_bus.rs` | One MXCC handle, one initiator privilege flag, CPU-0 timer/IRQ decoding, extra pages read zero | Module registry, explicit CPU-versus-DMA/debug initiator context, populated-bank routing and correct absence behavior |
| `src/devices/io_control.rs` | MID read forced to 8; processor arbitration bits stored but do not schedule CPUs | Profile-specific MID/arbitration behavior, reset masks and processor grant events; preserve SBus grants |
| `src/devices/interrupt.rs` | One software-pending bank/timer flag/PIL cache; target register ignored | Shared controller plus hardware-slot banks, CIT routing, directed IPIs and broadcast acknowledgement |
| `src/devices/timer.rs` | One processor counter/user timer and mode bit | Per-hardware-slot timer state and shared elapsed clock |
| `src/dma.rs`, `src/mmu.rs` | CPU-0-owned translation and destructive flush queue consumption | Explicit standalone-IOMMU ownership for MBus profiles without changing SS-5 semantics |
| CLI, examples, tests, Heliacal | Assume one CPU/PC and one step stream | Compatible default plus explicit processor selection, scheduling budgets and saved topology |

Choose one authoritative representation for CPUs. Compatibility access to
the boot CPU is acceptable if it refers to that actual state. Do not maintain
a mirrored `cpu` copy or swap CPU objects in and out of a fake CPU-0 slot.
If public APIs change, migrate all callers, including Heliacal, in that slice.

## Ordered slices and review gates

### 1. Topology and independent state — first assignment

Document the proposed topology, reset/arbitration and API choices above,
resolving evidence gaps explicitly. Add internal topology/state ownership
and module identity/routing support without enabling a production SMP boot
configuration yet. Construct a second CPU/module in focused tests; do not
clone CPU 0. Keep the normal machine constructor and execution path UP.

Regressions must show independent CPU/MMU/diagnostic/MXCC state, correct
MID-8/MID-A identities, local ASI versus cross-module physical accesses,
read-only identity, widths and aliases, absent MID-9/MID-B behavior, reset
isolation, and recursive MXCC stream targets returning errors without a
`RefCell` panic or partial mutation. Expected addresses and values must come
from the sources, not the production decoder under test.

Run the normal and counter-enabled core suites, fmt, Clippy and demos.
Compile/test Heliacal against the new core. Prove existing one-CPU SS-5,
SS-20 and Axil boots to a shell on disposable images. **Return for review
here before implementing scheduling or attempting a two-CPU Solaris boot.**

### 2. Multiprocessor board fabric and DMA ownership

Implement per-slot IRQ/timer banks, CIT, IPI, broadcast latches, arbitration
events and the agreed standalone IOMMU owner. Exercise both populated and
unpopulated slots from either CPU. Test masked/pending/acknowledge transitions,
timer mode/run independence, CIT changes with an asserted source, level-15
delivery and independent clearing, and isolated IOMMU flush/error behavior.
Preserve existing one-CPU and disk/DMA behavior before continuing.

### 3. Deterministic execution and synthetic SMP

Add the scheduler and CPU-tagged outcomes with one-CPU compatibility checks.
Use small hand-encoded or independently assembled programs to prove shared
memory visibility, contended atomic exchange/locks, directed IPI entry and
return, independent contexts/faults, cooperative TLB shootdown, and reset or
arbitration changes. Show that equal elapsed virtual time produces equal
system timer, SCC and framebuffer timing with one or two CPUs. Test one CPU
waiting while the other progresses and repeatability of event ordering.

Return a trace showing both processor identities making progress, plus
per-CPU instructions/traps/interrupts/errors and elapsed virtual time. Do not
infer SMP correctness from the unit-test count alone.

### 4. Original OBP startup and Solaris 2.6

Use private firmware/NVRAM/disk test directories for each profile/topology.
Enable a validated headless CPU-count option at this point. SS-5 must reject
two CPUs; SS-20/Axil accept only the implemented one/two-module layouts.

First establish each original OBP discovers both modules and reaches a usable
serial `ok` prompt. Capture discovery and the actual secondary startup/park
sequence with CPU ID, PC/nPC, ASI/MMIO address and value. Keep a first-failure
trace; do not hide unsupported accesses or convert a CPU fault into a silent
halt. A documented existing POST limitation is not an SMP pass or a reason
to patch firmware. Keep OpenBIOS results separate.

Then boot Solaris 2.6 on SS-20 and Axil independently. Require:

- Two CPUs online in `psrinfo -v`, with matching OBP identities and kernel
  startup messages, and no secondary-start timeout.
- Actual guest work on each CPU, demonstrated by processor-bound workloads
  where the installed Solaris tools permit, per-CPU guest statistics and
  emulator execution/interrupt evidence. Merely counting online entries or
  retired idle instructions is insufficient.
- Repeated cross-CPU workload, allocation/mapping and I/O activity without
  shootdown deadlock, lost timer interrupts, alignment panic or reboot.
- File checksum surviving writes, `sync`, clean halt and reboot on the cloned
  disk; keep clean serial logs and exact command/workload descriptions.
- The existing SS-5 UP boot and SS-20/Axil UP boots still reach a shell.

Use bounded instruction/time budgets and incremental serial polling. The
earlier full-transcript polling bottleneck must not return. Report the first
unsupported behavior with a focused reproduction rather than running
successive long boots without new evidence.

### 5. Heliacal and final acceptance

Add a CPU-count setting to machine definitions and F9 editing/selection.
Existing files must deserialize to one CPU. Preserve named-machine ROM,
NVRAM, disks, graphics and network settings; validate model/count combinations
before launching a worker. Changing topology requires rebuilding the machine,
not live insertion/removal. Keep one worker owning the entire machine.

Rebuild Heliacal against the exact core under review. Verify actual typing,
mouse, capture/release, pause/resume, reset and shutdown, plus SS-20 native
CG14/SX and Axil CGsix desktop operation during disk activity with two CPUs.
Repeat meaningful single-CPU regressions. Report both core and frontend
revisions, not merely that a release binary was built.

Update root/docs hardware-support and timing/API descriptions, and record
measured evidence in `clanker/smp-progress.md` with an index entry. Compare
fixed UP workloads and matched UP guest phases with the reviewed baseline;
record any regression and avoid unrelated optimization work in this branch.
Run all required checks on the final tree. Commit reviewable slices. The
user-approved incremental merge of slices 1 and 2A brings validated foundations
to main without enabling two-CPU execution. Enabling and claiming supported
SMP, and closing issue #2, still require both machine profiles to pass the
acceptance above. Later slices return for review before merge. Four-CPU
support must not be implied by these results.

## Validation and test media

Core requirements: `cargo fmt --all --check`,
`cargo clippy --locked --offline --all-targets -- -D warnings`,
`cargo test --locked --offline`, plus Clippy/tests with
`--features perf-counters`, and all four demos (`demo`, `trap-demo`, `serial-demo`,
`timer-demo`). Count executed instructions, synchronous traps, interrupt
entries and error stops separately. Frontend requirements include its own
fmt/Clippy/tests, release build, SDL smoke and original-firmware smoke.

Existing local sources, subject to checking they still exist:

- Firmware: `local/firmware/ss10-20-v2.25r-rom`,
  `local/firmware/axil320-2-19.bin`; hashes in `docs/firmware.md`.
- Solaris disk baselines: `local/completion-2026-11-13/desktop.raw` (SS-20),
  `axil.raw` and `ss5-final.raw` in the same directory.
- Serial NVRAM baselines: `local/ss20-ttya.nvram`, `local/axil-ttya.nvram`.
- Incremental harness reference: `local/performance-2026-11-13/src/main.rs`.

Clone disks and NVRAM before every experiment that writes them. Do not launch
two guests on the same writable disk. Keep source images and the user's
`local/heliacal-machines/` catalog untouched. Local evidence is ignored;
promote small independent regression fixtures into tracked tests, not ROMs,
third-party implementation sources, guest disks or multi-gigabyte traces.

## Source ledger and unresolved checks

- [Sun-4M System Architecture, 950-1373-01, revision 50, 1991-07-19](https://temlib.org/pub/SparcStation/Hardware/Sun4M_SystemArchitecture_edited2.pdf)
  ([mirror](https://www.netbsd.org/~mrg/Sun4M_SystemArchitecture_edited2.pdf)):
  sections 3.2.2.2.2, 5.1.2, 5.3, 5.4.3, 5.5.3.3, 5.7, 6.1.3/6.1.4,
  7.1.4, A.I.2/A.II.2, B.III.5.4.7 and B.III.8 support the topology,
  arbitration, timer, interrupt, local flush and MXCC decisions. The manual
  is older than SS-20 and documents several boards/modules; retain distinctions
  and verify board-specific reset/identity quirks against firmware traces.
- [SuperSPARC II Addendum, STP1021UG revision 1.3](https://www.ele.uva.es/~jesman/BigSeti/ftp/Microprocesadores/SPARC/STP1021UG.pdf)
  and [SPARC V8 manual](https://sparc.org/wp-content/uploads/2014/01/v8.pdf.gz):
  use the project's existing references for IU/MMU identity, reset, atomics,
  trap priority and memory semantics. Any new module-reset/ASI interpretation
  needs an exact section and a regression; physical/ASI register maps must
  not be equated merely because some existing aliases coincide.
- QEMU pinned revision `f71afb409a87c4ecd0f9d63a5963eb645dfab780`:
  [sun4m.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/sparc/sun4m.c),
  [slavio_intctl.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/intc/slavio_intctl.c),
  [slavio_timer.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/hw/timer/slavio_timer.c),
  [cpu.c](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/cpu.c).
  Retrieved for this review. Compare routing, per-CPU timer state and lifecycle;
  QEMU/OpenBIOS conventions and maximum CPU counts do not define these boards.
- Solaris 2.6 source archive, pinned revision
  `3aad11543652bdbf6157623b246db26c069ca89f`:
  [sun4m/os/mp_startup.c](https://github.com/Arquivotheca/Solaris-2.6/blob/3aad11543652bdbf6157623b246db26c069ca89f/os_net/src_ws/usr/src/uts/sun4m/os/mp_startup.c).
  Inspected `start_other_cpus`, `mp_startup_init` and `mp_startup` for the
  guest's expected startup path. Match symbols/behavior against the installed
  guest image; archived sources alone do not prove an exact binary match.
- MAME revisions and files remain pinned in `docs/references.md`. Its unfinished
  sun4m support is not independent acceptance of a proposed SMP startup model.
  Compare implemented CPU/atomic semantics where useful.

Reference downloads for this investigation are under the ignored
`local/smp-reference-2026-11-14/`. Outstanding evidence includes original
SS-20/Axil two-module discovery/startup traces, any module-local reset mechanism
those ROMs actually use, and precise profile-specific MID-read behavior (the
Sun-4M manual explicitly describes implementations with a broken MID register).
Resolve those in the relevant slice; do not turn an unknown into a silent
constant or an advertised support claim.
