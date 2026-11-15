# Configurable RAM completion review

2026-11-15; configurable RAM in Helios and Heliacal. This supersedes the
slice D interim review. The user approved integration into both `main` branches
and closure of issue #3 on 2026-11-15.

## Corrections made during acceptance

### Axil 2.19 bank sizing

The owner confirms eight RAM sockets, two capable of holding VSIMMs, and
suggests 8/16/32/64 MiB module sizes. No independent primary Axil inventory was
found for the smaller modules. Preserve the eight sockets and 512 MiB maximum;
do not present uncertain module support as established hardware fact.

The original Axil 2.19 trace at PC `0x338b4..0x338cc` writes `0x04000000` at a
bank base and `0x01000000` at base+16 MiB, then rereads the base. Its later
ASI-0x20 sizing pass repeats that test. It does not perform the SS-20's 32 MiB
size check. The old contiguous 32 MiB bank was therefore counted as 64 MiB:
416 MiB appeared as 448, and 480 as 512.

`DramLayout::axil_total` now uses 16/64 MiB banks. 32 MiB total uses two 16 MiB
banks; 416 uses six 64 MiB banks plus two 16 MiB banks. 432/480/496 do not fit
and are rejected by both applications; 512 remains eight 64 MiB banks. This
is a firmware/profile compatibility restriction, not a claim about every
physical Axil revision. Independent per-socket enumeration and a captured
alias-probe regression are tracked in `tests/ram_layout.rs`.

### SS-20 32 MiB DSIMM mapping

Checking actual `/memory/reg`, rather than just a banner, exposed a second
bug. A 32 MiB SS-20 DSIMM is advertised as two 16 MiB regions at bank offsets
0 and 32 MiB. The previous modulo-32 mapping aliased those advertised ranges.
The backing store now has two separate regions; PA24 is mirrored within each.
CPU lanes, doublewords, atomics, IOPTE tables and DMA use that same mapping.
Raw image import accepts real regions and rejects crossing into mirrors.

Tracked regressions in `tests/dram.rs` and `tests/high_ram.rs` verify distinct
patterns in both regions, aliases, byte lanes, doublewords, atomic exchange,
image import, and an IOPTE table/DMA target in the upper region. The existing
SMP TLB-isolation fixture now installs 64 MiB so its two target addresses name
real independent storage on both profiles.

Original OBP 2.25R with 48 MiB reports physical ranges
`[0,16 MiB)`, `[32,48 MiB)`, `[64,80 MiB)`. A fresh Solaris 2.6 clone also boots
with a mixed 416 MiB population, verifies 67,117,056 bytes of distinct strings,
performs nonzero disk I/O, and halts cleanly.

### SS-20 CGsix console discovery

The 512 MiB panic was reproduced at 128 MiB: `ddi_binding_name()+4`,
PC `0xf00f15d4`, loaded through a null devinfo pointer. Tracing
`i_path_to_drv`, `get_neighbors`, firmware node IDs and the saved pathname
showed a nameless DBRI placeholder node preceding CGsix. Solaris stops
copying those sibling nodes, then cannot resolve its PROM console phandle.
This is unrelated to high RAM or two-CPU scheduling.

Normal OBP configuration `setenv sbus-probe-list f0123` skips unsupported
DBRI slot `e`, retaining onboard SCSI/Ethernet slot `f` and external 0–3.
With `input-device keyboard` and `output-device screen`, the unmodified ROM
and Solaris kernel boot the CGsix console. Native SX does not require this
workaround. This setup requirement is documented in both READMEs and
`docs/machines.md`; no ROM or kernel patch and no claim of DBRI emulation.

### Frontend acceptance and fixtures

Six independent slice C fixtures are now tracked as
`heliacal/src/ram_acceptance_tests.rs`, including exact RAM-choice enumeration,
model/display transitions, transactional drafts, named-machine/CLI precedence,
invalid startup before media/SDL, and real guest memory access at 192 MiB
across confirmed 128-to-256 MiB replacement. Failed replacement preserves the
old machine and NVRAM identity. Axil choices follow the corrected core model.
A visible overlap between the model/RAM heading and “Unsaved changes” was also
fixed after the real F9 check.

## Validation and evidence

- Core: **624 unique normal tests / 626 with perf-counters** (625/627 raw
  successful lines include the isolated MXCC child). Formatting, both Clippy
  configurations with `-D warnings`, and all four demos pass.
- Heliacal: **48 tracked tests**, formatting, Clippy, release rebuild and SDL
  smoke pass. The SS-5 256 MiB original-firmware display smoke passed at
  96,614,400 steps; it is not an OS-boot claim.
- Earlier reviewed slice D evidence retains default-capacity UP/SMP boots,
  maximum-capacity serial boots and 16 MiB disk write/reboot tests for all
  four model/display combinations. Physical maximum RAM `reg` arrays were
  independently checked in `completed-memory-maps.json`.
- Nine completed guest runs add bounded allocation/touch/verification, nonzero
  disk data and reboot checksum checks. `/helios-ram-final` is 884,736 bytes,
  POSIX `cksum` **3135931465**. SS-5 and Axil workloads verify 67,633,152 bytes;
  SS-20 workloads verify 67,117,056 bytes. Both use unique numbered strings
  and compare every value. Commands and serial transcripts are retained.
- SS-20/SX 448 MiB completes the allocation/write run with CPUs 0/2 online,
  then reboots with one CPU and rereads the same checksum. SS-5 uses 256 MiB;
  SS-20/CGsix and Axil use 512 MiB. The latter two also cover both one- and
  two-CPU boots across the write/reboot pair. Read-only UFS inspection
  independently verifies all five written disks, including the mixed 416 MiB
  SS-20 case. `guest-assertions.json` and `ufs-assertions.json` record the checks.
- Heliacal SS-20/CGsix 512 MiB and Axil 512 MiB, both two CPUs: visible Solaris
  maintenance shells, `prtconf`, `psrinfo`, typing, release/recapture with an
  absent leak-file assertion, and clean `sync; halt` to OBP. Both host Quit
  operations return status 0. Axil additionally performs confirmed F5 reset,
  rereads `/heliacal-ram-gui` (`879949744 18`), and discards RAM/CPU draft
  changes in F9 while retaining the 512 MiB saved/running configuration.
- SS-5/CGsix 256 MiB also passes guest typing, memory/CPU reporting,
  capture/release with an absent leak-file assertion, clean halt, and host
  Quit status 0. F9's 8 MiB draft is discarded while the saved/running
  configuration stays at 256 MiB. Its final dirty-overlay capture verifies
  that the model/RAM heading and unsaved notice no longer overlap. The long
  footer is shortened to fit, followed by frontend tests, Clippy, formatting,
  release rebuild and SDL smoke.
- Earlier native-SX Heliacal capture independently shows 448 MiB, both CPUs
  online, root login and the typed `HELIACAL-MAX-CONSOLE-OK` marker. The earlier
  blank Axil captures used TTYA NVRAM and do not count as display acceptance.

All planned completion checkpoints are resolved. This report records the
reviewed payload from `feat/configurable-ram`; the repositories' Git histories
record its integration into `main`.

## Preservation and cleanup

All four source disks, four firmware images and the live Heliacal catalog
retain their starting SHA-256 hashes (`source-verification.json`). All task
guest/Xvfb processes exited before cleanup. `gui-assertions.json` records the
three final GUI runs, saved configurations, captures and read-only absence
of leaked-input files.

Removed 301 scratch files/links, including 43 raw disk entries (35 disposable
images and 8 aliases), temporary NVRAM, temporary harness builds, downloaded
reference copies and redundant captures. Their aggregate allocated-file size
was 8,879,284,224 bytes; reflink sharing means this is not a claim about actual
space reclaimed. `cleanup-manifest.json` records each path and reason. Kept
compact serial/build logs, commands, assertions, harness sources and 21 useful
PNG captures. The large Axil sizing trace is retained as a verified gzip file.
No source images, live preferences, prior unrelated evidence or main build
outputs were removed.

Evidence roots: `local/ram-completion-2026-11-15/`,
`local/ram-capacity-2026-11-15/`, and
`local/review-ram-slice-d-2026-11-15/`. Interrupted SX runs terminated with host
SIGTERM are explicitly excluded; only completed runs count. Input acceptance
waits for commands to finish before capture changes: switching immediately
after host typing can discard keyboard events that are still queued.

No firmware, OS binaries or disks are tracked. Full POST, DBRI audio, four
CPUs, Axil VSIMM, unverified small Axil modules and memory hotplug remain
outside this milestone.
