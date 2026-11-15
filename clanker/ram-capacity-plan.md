# Configurable RAM: issue #3 implementation handoff

Prepared 2026-11-15. Slices A–C are implemented and reviewed; the completion
work found and corrected Axil firmware sizing and SS-20 split-bank mapping.
See the [completion review](ram-capacity-completion.md) for current acceptance,
validation and cleanup status. The user approved integration into both `main`
branches and closure of issue #3 on 2026-11-15.

## Objective and starting points

Implement [Helios issue #3](https://github.com/salvogendut/helios/issues/3),
“Increase ram capacity up to supported maximum per model,” in the core, headless
CLI and sibling Heliacal frontend. Users must be able to select, save and recall
each supported machine's valid RAM configurations, including its maximum.
Keep the existing 128 MiB default and existing named-machine configurations.

Both repositories already have a pushed `feat/configurable-ram` branch:

| Repository | Implementation base, before this handoff |
| --- | --- |
| Helios | `d821ed10705439aa79be0e43a3483ea63f4fdb04` |
| `../heliacal` | `ddafd879ce4301f030b5f78e7d3ca48963ac996f` |

Read each repository's `AGENTS.md`, then Helios `README.md`, `HACKING.md`,
`docs/references.md`, `docs/dram-tlb.md`, `docs/machines.md`, `docs/io-control.md`
and `docs/firmware.md`. The pre-feature baseline was 605 unique normal core
tests / 607 with `perf-counters`, and 34 Heliacal tests. Accepted slices A/B now
pass 620/622 unique core tests; slice C passes 42 Heliacal tests. Review notes
record the independent checks and the limits of those results.

Keep the current CPU configurations: SS-5 has one CPU; SS-20/Axil have one or two
single-CPU modules, processors 0/2. Four CPUs, new machine families, JIT, memory
hotplug, a second VSIMM and unrelated controller expansion are outside this task.

## Hardware constraints and sources

All capacities below use MiB. A maximum is not permission to accept every smaller
integer: a total must be constructible from the available modules and sockets.

| Machine / installed graphics | DRAM modules and sockets | Capacity target |
| --- | --- | --- |
| SS-5 / CGsix | Eight sockets, 8 or 32 MiB DSIMMs | 256 MiB |
| SS-20 / native SX with one VSIMM | Seven DRAM sockets, 16/32/64 MiB DSIMMs; bank 7 reserved for VSIMM | 448 MiB |
| SS-20 / CGsix, no VSIMM | Eight DRAM sockets, 16/32/64 MiB DSIMMs | 512 MiB |
| Axil 320 / CGsix | Eight sockets, two VSIMM-capable; pinned OBP 2.19 profile uses 16/64 MiB banks with no VSIMM | 512 MiB |

Source notes:

- [Sun SS-5 Service Manual, 801-6396-11](https://docs.oracle.com/cd/E19127-01/sparc5.ws/801-6396-11/801-6396-11.pdf),
  section 11.3, printed pages 11-12/11-13, specifies eight sockets, 8/32 MiB
  modules and a 256 MiB maximum. Table 4-4 gives the fixed 32 MiB bank apertures.
  Its shipped minimum of 16 MiB is distinct from the core's existing single
  8 MiB module configuration. Preserve that library configuration; determine
  original-firmware acceptance separately, without claiming Solaris boots in it.
- [Sun SS-20 Service Manual, 801-6189-12](https://docs.oracle.com/cd/E19127-01/sparc20.ws/801-6189-12/801-6189-12.pdf),
  tables 4-4/4-5, printed pages 35/36 (PDF pages 59/60), locates the first VSIMM
  at J0304, **physical address bank 7**, `0x1c000000..0x20000000` (exclusive end).
  The second VSIMM uses bank 4; it is outside this task. Appendix B, printed
  pages 198–200, gives 16/32/64 MiB modules and the 512/448/384 MiB capacities
  for zero/one/two occupied video slots. Slot labels, installation order and
  address-bank numbers differ; do not use the SS-10SX slot map for SS-20.
- The project owner confirmed on 2026-11-15 that the Axil 320 has eight RAM
  sockets and that two of them may hold VSIMMs. With no VSIMM, all eight are
  available for DRAM, so the 512 MiB target is geometrically consistent. This
  is user-supplied hardware evidence, not a manufacturer manual. The existing
  core's initial SS-20-style module assumption was superseded during acceptance:
  the pinned Axil 2.19 ROM recognizes 16/64 MiB banks. The profile now builds
  those populations. Physical 8/32 MiB Axil support remains unverified; do not
  confuse this firmware restriction with a vendor module inventory.
- Use the existing microSPARC-II and Sun-4M architecture manuals for address,
  mirroring and DMA rules. Emulator comparisons remain pinned to QEMU
  `f71afb409a87c4ecd0f9d63a5963eb645dfab780` and MAME machine sources
  `ed59c15fe78596ec6ff8fe04482ede78e6122e34`; see `docs/references.md` for paths.
  Original OBP is the acceptance target. OpenBIOS/QEMU's contiguous RAM or
  firmware interfaces are not a substitute for the board's bank geometry.

Derived examples for tests: SS-5 232 MiB fits seven 32 MiB plus one 8 MiB
module; 240 MiB does not fit eight sockets. Eight-bank SS-20 480 MiB fits,
496 MiB does not. Seven-bank SS-20/SX 416 MiB fits, 432 MiB does not, and
448 MiB fits. Verify these independently by enumerating module counts, rather
than copying the production layout algorithm into the tests.

## Findings at the start of implementation

| Area | Observed behavior / work required |
| --- | --- |
| `src/dram.rs` | `Ss5Dram` already implements sparse fixed apertures, including SS-20 64 MiB banks. `from_total` accepts 8/32 MiB modules; `from_ss20_total` accepts 16/32/64. Both choose a deterministic largest-module-first layout. Validation currently allocates backing RAM, and the SS-20 constructor does not reserve a VSIMM bank. |
| `src/machine.rs` | `with_profile_processors` constructs banked RAM directly through `MachineProfile::dram`. It does **not** use the 256 MiB `MAX_RAM_BYTES` guard; that guard belongs to `bare_metal`. SS-20/SX is installed after RAM construction. `InvalidDramSize` incorrectly describes only SS-5 modules. |
| `src/iommu.rs` | Standalone IOPTE reads reject addresses at/above `DRAM_LIMIT = 0x10000000`, even on SS-20/Axil. |
| `src/dma.rs` | `dma_address` also rejects every translated target at/above `0x10000000`. SCSI and Ethernet share this path. This is a functional blocker to using the upper half of 512 MiB safely. |
| `src/devices/io_control.rs` | Bypass validation has another `0x10000000` cutoff, with a hardware-rule comment. Research this rule per profile before changing it. Do not widen every occurrence mechanically. |
| `src/mmu.rs` | Integrated SS-5 IOPTE handling has its own 256 MiB guard. Preserve SS-5 semantics while correcting the standalone path. |
| `src/system_bus.rs` | Most decoding uses `dram.aperture_bytes()`, but `install_ss5_dram` checks existing mappings against the SS-5 `APERTURE_BYTES` constant. Audit installation order, overlap checks and complete access spans. |
| `src/main.rs` | `boot` and `run` already parse `--ram-mib` with checked multiplication. Boot constructs the profile before adding an optional CGsix card; adding CGsix currently does not remove the automatically installed SX. Make installed hardware explicit before selecting RAM layout. |
| Heliacal `src/settings.rs` | `ram_mib` already exists but validation permits only 128. `hardware_eq` omits RAM, so a RAM change would not request machine replacement. |
| Heliacal `src/profiles.rs` | `NamedMachine` already saves/restores `ram_mib` and delegates validation. Reuse this schema; validate inactive named machines as well as the active one. |
| Heliacal `src/emulator.rs` | `build` ignores `settings.ram_mib` and allocates constant `RAM_MIB`. It removes default SS-20 SX only after construction when CGsix is selected. |
| Heliacal `src/overlay.rs` | F9 General already has a Memory row, currently `Field::Fixed`. Model/display changes need compatible RAM choices. |
| Heliacal `src/main.rs`, `src/view.rs` | No frontend `--ram-mib` override yet; help/About contain fixed-128 wording. Check configured-versus-running hardware labels during pending replacement. |

Do not solve this by changing one constant or by adding only a frontend selector.
Allocation, bank topology, CPU access, device DMA and displayed settings must agree.

## Slice A — shared RAM topology and validation

1. Resolve and document the hardware questions above. Treat VSIMM presence as
   installed hardware, independent of whether a UI displays frames or the guest
   uses a serial console. Serial operation alone does not remove a VSIMM.
2. Add a small core API which computes/validates the RAM layout **without allocating
   RAM**, and exposes valid totals for a profile and its installed video hardware.
   Return useful profile-specific errors. Keep allocation as a separate step.
3. Preserve current layouts for valid existing configurations where possible.
   Keep address-bank numbering, sparse holes, supported mirrors and word-transparent
   accesses. With SS-20's single VSIMM, bank 7 cannot also contain DRAM. Do not
   subtract the 8 MiB framebuffer size from DRAM; the occupied socket is the
   constraint. Populate bank 0 as required by firmware/hardware.
4. Introduce or extend a core construction-options API so the final RAM/VSIMM
   combination is validated before RAM allocation and guest execution. Existing
   convenience constructors should retain their documented default hardware.
   An SS-20 without VSIMM must have a deliberate construction path; do not build
   an invalid 512 MiB/SX machine temporarily and then remove SX.
5. Add pure layout tests for all representable totals, zero, wrong granularity,
   over-capacity, integer overflow and invalid video combinations. Cover the
   near-maximum examples above, bank 7 reservation and unchanged 128 MiB defaults.
   Keep low-level bank fixtures usable for device tests; board population rules
   belong at the board construction layer.

Deliverable: reviewable topology API, source notes and focused tests. Report any
unresolved Axil assumption before relying on it in the UI or long guest runs.

## Slice B — core, DMA and headless CLI

1. Thread the validated layout through machine construction and bus installation.
   Audit memory overlap and address checks against the actual board aperture,
   including access width and overflow. Distinguish installed bytes, bank aperture,
   empty-bank behavior and real storage; they are not interchangeable bounds.
   Preserve existing synthetic `bare_metal`/`sun4m` harness behavior unless a
   separately justified change is needed. Keep unsupported targets explicit.
2. Fix standalone IOPTE fetches and translated DMA targets above 256 MiB through
   a shared, profile-aware classification of DRAM accesses. A raw upper bound must
   not admit MMIO, a VSIMM aperture, or arbitrary 36-bit addresses. Preserve existing
   empty-bank completion where applicable and specify how that interacts with DMA;
   do not silently replace it with contiguous allocation or clamp addresses.
3. Review bypass rules separately against primary manuals. Retain integrated SS-5
   PPN/IBAR widths, fault metadata and I/O TLB ownership. Preserve standalone
   36-bit IBAR/PPN handling, CPU-independent flush consumption and fault atomicity.
   A table base outside real board DRAM must never alias to low RAM.
4. Extend `boot --ram-mib` validation/help and make the installed graphics choice
   explicit. Recommended: `--display sx|cgsix|none`, with current profile defaults;
   `none` means no framebuffer/VSIMM, not merely serial output. Preserve existing
   `--cgsix-rom` command usage by inferring CGsix when no explicit choice is given.
   Reject conflicting options. CGsix without a VSIMM permits SS-20's full 512 MiB.
   Parse all options before validating the combination, regardless of argument order.
5. Add tests around `tests/dram.rs`, `tests/machines.rs`, `tests/standalone_iommu.rs`
   and `tests/cli.rs`. Required meaningful checks:
   - Independent data at low RAM, above 256 MiB, bank boundaries and the last
     populated word: `0x0ffffffc` for full SS-5, `0x1bfffffc` for full SS-20/SX,
     `0x1ffffffc` for full SS-20 without VSIMM. Check the exclusive endpoint too.
   - CPU translated/bypass access, byte/half/word/doubleword access and atomics in
     upper RAM, plus CPU-2 visibility with independent translations.
   - High-address IOPTE **tables and targets**, separately and together; stale and
     refreshed mappings after page/all flushes; low/high non-aliasing; invalid,
     protected, misaligned and non-DRAM targets with no unintended writes.
   - Both SCSI and Ethernet data paths through high RAM, rather than only calling
     a private translator. Retain source attributes and controller error handling.
   - Reset retains the selected capacity and documented memory contents; module
     reset, software reset and full cold construction retain their existing scopes.
   - SS-5 upper-bound and existing high-IBAR non-aliasing regressions still pass.

Use mostly small bank fixtures and allocation-free topology tests. Bound real
maximum-size tests so parallel testing does not allocate dozens of 512 MiB guests.

## Slice C — Heliacal configuration, named machines and F9

1. Replace the 128-only check with the core's allocation-free validation. Keep
   `ram_mib` and its default of 128 in both `Settings` and `NamedMachine`. Validate
   the final model/display/CPU/RAM combination, including inactive named records.
2. Construct the worker machine using `settings.ram_mib` and the selected installed
   display topology. Add RAM to `hardware_eq`; a change follows the existing
   confirmed cold-replacement flow. Never resize a running machine in place.
3. Make F9's existing Memory row selectable, using exactly the core's valid totals.
   Keep the current value when a model/display switch leaves it valid. Otherwise
   select the largest valid value no greater than the old value in the **draft**
   (or the new minimum if none exists), and visibly report the adjustment.
   Save/Discard/Cancel must remain transactional.
   Raw configuration and CLI input must reject invalid totals rather than clamp.
4. Add frontend `--ram-mib`, with checked conversion and override precedence matching
   `--cpus`: named-machine selection first, CLI overrides second, validation last.
   Invalid combinations should fail before starting the worker or touching media.
5. Ensure names and status accurately distinguish selected settings from running
   hardware while replacement is pending. A failed construction must preserve the
   old working machine. RAM changes must not create a new machine identity or
   change the named machine's NVRAM path.
6. Cover old configs omitting RAM, named-machine save/recall and round trips,
   inactive invalid records, CLI order/overflow, F9 model/display transitions,
   save/discard/cancel, replacement detection and actual worker allocation. Assert
   the constructed bus layout; a label saying “512 MiB” is not sufficient.
7. Update help, About, README and examples. Reconcile the old fixed-128 statement
   in Heliacal's development instructions with configurable RAM and a 128 default.
   Use temporary configs for validation; do not rewrite the user's live catalog.

## Slice D — firmware, Solaris and frontend acceptance

Use the original images pinned in `docs/firmware.md`. Available local paths include
`local/firmware/sun-ss5.bin`, `ss10-20-v2.25r-rom`, `axil320-2-19.bin` and
`turbogx.bin`. The local catalog is `local/heliacal-machines/heliacal.cfg`, with
source disks under its `disks/` directory: `ss5-solaris26.raw`,
`ss20-solaris26.raw`, `axil320-solaris26.raw`. Verify availability and hashes first.

Use disposable reflink/sparse disk clones and copied NVRAM. Never test writes on
the source disks. Preserve the known firmware startup settings; existing full
POST limitations are not permission to patch the ROM or fabricate its RAM report.

| Configuration | Required original-firmware and OS checks |
| --- | --- |
| SS-5 / CGsix / 1 CPU | 128 MiB regression, 256 MiB maximum; smaller and mixed-module OBP samples |
| SS-20 / SX / 1 and 2 CPUs | 128 MiB regression, 448 MiB maximum; smaller and mixed-module OBP samples |
| SS-20 / CGsix without VSIMM | 512 MiB maximum, at least one CPU; cover two CPUs with high-memory core tests and a guest smoke |
| Axil / CGsix / 1 and 2 CPUs | 128 MiB regression, verified maximum; smaller and mixed-module OBP samples supported by the confirmed geometry |

For small configurations, verify OBP memory discovery and a usable `ok` prompt;
do not require an installed desktop OS to run at the hardware's smallest size.
Include a supported non-power-of-two total per geometry, plus module-transition
boundaries. Capture the `/memory` device-tree `reg` ranges and compare their sum
and addresses with the selected layout. Do not rely only on the banner.

For maximum and default OS checks, reach Solaris root, record `prtconf` memory and
`psrinfo -v`, exercise a bounded memory allocation/touch workload and disk I/O,
then `sync`/clean halt. Reboot maximum-size clones and verify checksum markers.
Keep physical high-memory/DMA assertions in core tests: a large guest allocation
alone does not prove Solaris placed its pages above 256 MiB. If OBP reserves
memory, explain the difference between installed, available and OS-reported RAM.

In Heliacal verify F9 selection and named recall, confirmation/replacement,
display output at each graphics-dependent maximum, guest terminal typing,
capture/release, cold reset and clean quit. Boot-smoke proves display activity,
not desktop or OS acceptance. Rebuild Heliacal against the final core before
claiming these results. Keep SS-20/SX rendering and existing Axil/CGsix interaction
working at the larger sizes.

Use the fast serial harness pattern from the previous milestones: bounded batches,
incremental transcript polling with overlap, and absolute per-byte pacing. Record
per-run configuration, ROM/NVRAM/disk source hashes, steps, wall time, outcome and
the exact commands used. Do not restart a long guest for a question a unit test
can answer.

## Final validation, documentation and cleanup

Reuse the toolchain/cache described in each `clanker/build-environment.md`. In
Helios, after the final code change:

```sh
cargo fmt --all --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo clippy --locked --offline --all-targets --features perf-counters -- -D warnings
cargo test --locked --offline
cargo test --locked --offline --features perf-counters
for demo in demo trap-demo serial-demo timer-demo; do
    cargo run --locked --offline -- "$demo" || exit
done
```

In Heliacal, with its existing SDL linker setup:

```sh
cargo fmt -p heliacal --check
cargo clippy --locked --offline --all-targets --no-deps -- -D warnings
cargo test --locked --offline
cargo build --release --locked --offline
make smoke
```

Run the relevant original-firmware boot smokes and the acceptance matrix above.
Report unique test counts separately from the isolated MXCC child-test log line.
Update human-facing `README.md`, `HACKING.md`, `docs/machines.md`,
`docs/dram-tlb.md`, `docs/io-control.md`, `docs/references.md` and Heliacal README
where behavior changed. Keep findings and progress in `clanker/` and its index.

Keep compact evidence in ignored `local/ram-capacity-<date>/`: command manifest,
serial logs, assertions and a few useful captures. After each completed run,
remove that run's disk/NVRAM clones and temporary builds. Preserve source media,
the user's live configuration, existing acceptance evidence and unrelated work.

Return a review handoff with changes by repository, exact validation results,
RAM/display/CPU combinations actually accepted, unresolved assumptions and
remaining limits. Issue #3 is complete only when configurable RAM works through
both applications with maximum-size memory and storage acceptance. Keep the issue
open through partial slices; merging remains a separate review decision.
