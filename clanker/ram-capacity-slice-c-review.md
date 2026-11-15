# Configurable RAM: slice C review

Reviewed 2026-11-15 against the uncommitted Heliacal changes on
`feat/configurable-ram`, following `bb1c0be`, with the already accepted Helios
slices A/B. No submitted production source or test was changed by this review.
Both repositories' implementation changes remain uncommitted.

## Verdict

**Slice C is accepted for progression to slice D.** No blocking defect was found
in settings validation, named-machine precedence, F9 choices, construction or
cold replacement. This accepts frontend integration, not maximum-capacity
Solaris/desktop support. Issue #3 remains open until slice D is complete.

The frontend delegates topology to the core, validates inactive named records,
preserves the 128 MiB default, and constructs the selected installed video and
RAM directly. RAM participates in hardware-change detection without changing
label/NVRAM identity. F9 adjustments remain in the draft and reach the visible
notification path. The running hardware description remains separate from saved
settings while replacement is pending or has failed.

## Independent checks

Six review fixtures were run against an isolated copy of the submitted frontend:

1. Independently enumerate socket/module sums, then cycle every F9 Memory value
   for SS-5, SS-20/SX, SS-20/CGsix and Axil. Check exact order and wraparound.
2. Exercise every valid starting total through model/display changes. Check
   unchanged valid totals, largest valid total below the prior size, minimum
   fallback, adjustment notices and the untouched saved settings.
3. Exercise RAM draft Cancel, Discard and failed Save through `App`; compare the
   saved settings and original catalog bytes after each operation.
4. Boot a synthetic SS-5 guest that writes and reads physical address
   `0x0c000000` (192 MiB). With 128 MiB installed, it enters the empty-bank loop.
   Save 256 MiB: the paused guest and running description remain unchanged.
   Cancel F5: still unchanged. Confirm F5: the new guest reads its write back and
   enters the installed-bank loop. Then save 128 MiB, remove the temporary ROM
   before confirmation, and check that the failed replacement preserves the
   paused 256 MiB guest, PC, step count and description. A custom NVRAM byte and
   the label's state path survive the replacements and clean worker shutdown.
5. Apply named-machine CLI overrides in both argument orders, including changing
   SS-20/SX 448 MiB to CGsix 512 MiB. Check final hardware, unchanged source
   records and no implicit save.
6. Call the startup path with invalid/overflow-scale RAM and nonexistent media.
   Validation rejects it before SDL, worker creation, configuration locking or
   media access. Also check an invalid inactive named record.

The lifecycle fixture is useful to promote into tracked coverage before final
integration: it verifies actual guest-visible RAM after F5, beyond the existing
constructor layout and running-description assertions. Its current result is a
pass, not an outstanding defect.

## Validation rerun

| Check | Result |
| --- | --- |
| Heliacal unit suite | 42 passed |
| Independent review fixtures | 6 passed |
| `cargo fmt -p heliacal --check` | Pass |
| Clippy, locked/offline, all targets, no deps, `-D warnings` | Pass |
| Locked/offline release build | Pass |
| `make smoke` with SDL dummy/software | Pass |
| Original SS-5 firmware display smoke at 256 MiB | Pass, 97,404,928 steps |
| Heliacal `git diff --check` | Pass |

The real-firmware capture visibly reports `256 MB memory installed` and is
still running firmware memory testing. It is **not** an `ok` prompt, completed
POST, OS boot or desktop acceptance. This run used an empty temporary config,
temporary NVRAM and no disk. The ROMs and live catalog had identical SHA-256
hashes before/after; no source disk was attached. The core suites were not rerun
for this frontend-only review; accepted slice B results remain 620/622 unique
tests without/with `perf-counters`.

Evidence is retained under `local/review-ram-slice-c-2026-11-15/`: validation
logs, independent fixture source, reviewed-source hashes, smoke command/source
hashes and the compressed PNG capture. Temporary fixture source/build copies,
the duplicate BMP and temporary configuration/NVRAM were removed. The normal
Heliacal release binary remains available.

## Handoff to the other AI

Proceed with **slice D** in [the shared plan](ram-capacity-plan.md#slice-d--firmware-solaris-and-frontend-acceptance).
Use the current uncommitted changes in both `feat/configurable-ram` branches.

- Use disposable disk clones and copied NVRAM; hash/preserve source media and
  the user's catalog. Reuse the efficient incremental serial harness.
- Complete the planned default/maximum matrix: SS-5/CGsix 128/256 MiB;
  SS-20/SX 128/448 MiB with one/two CPUs; SS-20/CGsix 512 MiB with the planned
  SMP coverage; Axil/CGsix 128/512 MiB with one/two CPUs. Retain the documented
  Axil module-size evidence limitation.
- Include smaller and mixed-module OBP samples. Capture `/memory` `reg` ranges
  and compare the addresses and total against the configured banks, including
  SS-20/SX's reserved bank 7. A firmware banner alone is insufficient.
- For Solaris acceptance, record `prtconf` and `psrinfo -v`, perform bounded
  allocation/touch and disk I/O, cleanly sync/halt, reboot maximum-size clones
  and verify checksum markers. Retain core high-memory/DMA assertions; a guest
  allocation alone does not prove which physical pages Solaris used.
- Validate actual Heliacal F9/named recall and confirmed RAM replacement,
  terminal typing, capture/release, cold reset and clean quit at the relevant
  graphics-dependent maxima. Rebuild against any final core corrections.
- Report exact passes/failures with evidence, rerun required checks after any
  fixes, reconcile documentation, and remove completed scratch clones/builds.
  Return the feature for final review before commit/merge or closing issue #3.
