# CHD hard disks — implementation and acceptance

Issue: [#6](https://github.com/salvogendut/helios/issues/6). Developed on
`feat/chd-images` from Helios `f62911f` and Heliacal `8651356`.
The user's live Heliacal guest/catalog and mounted media
were not used for writable tests.

The keyboard-presence exception is separately tracked in
[#7](https://github.com/salvogendut/helios/issues/7). Headless still attaches
the keyboard for original-OBP startup; no keyboard or guest CDE policy changes
are mixed into CHD work. Disk-free probes found SS-5 sync-trap and SS-20 POST
errors with no keyboard; Axil reached ok. No keyboard-free OS claim was made.

## Implementation

- `media` owns byte-addressed raw/CHD I/O and logical capacity. SCSI still
  stages complete DATA OUT and syncs before GOOD. Raw/ISO semantics remain.
- Optional exact `libchdman-rs=0.289.5`, source build, no prebuilt downloads.
  Default build compiles without dependencies on Rust 1.74. The CHD wrapper's
  `is_multiple_of` needs 1.87; that exact lower compiler floor has not been
  experimentally built. Current nightly and source C++ builds pass.
- v5/512-byte hard disks only. Header/map/metadata bounds, cycle checks,
  explicit parent identity and companion validation before native access.
  Malformed containers never fall back to raw writes.
- Uncompressed disks are written in place. Compressed parents create/reuse
  persistent `.diff.chd` children; read-only mode follows existing children
  without creation. No automatic merge/recompression/deletion.
- Native custom I/O syncs appended data before its map entry, then the guest
  command's sync persists the map. Native write/sync errors poison the backend
  until reopening. Handles stay worker-local; no unsafe Send implementation.
- First mutation of a hashed writable uncompressed image clears its stale
  raw/overall SHA-1 fields and syncs the header; parent identity is preserved.
  Merely opening an image leaves its hashes untouched.
- Reproduced native zero-hunk bug: zeroing an unallocated child hunk leaves
  nonzero parent data visible. Claim inherited data before zeroing. Full and
  partial zero writes and a child process exiting without destructors verify
  durability; a native read-only-fd failure verifies cache poisoning.
- `create_disk`: sparse raw or uncompressed CHD, no overwrite, private sibling
  staging, synced no-clobber publication and owned-temp cleanup. CLI:
  `helios create-disk OUTPUT --format chd|raw --size-mib N`.
- Heliacal enables CHD, validates logical capacity in active/inactive records,
  and offers F9 → Media → New blank CHD disk. Destination/capacity/unused target
  are draft controls. Creation runs off SDL; Save/F5 apply the attachment.
  Cancel before creation has no effect; Discard keeps the created user file.

## Completed checks

Evidence root: `local/chd-development/` (ignored).

- Final floppy/CPU corrections: 651 unique default core tests; 669 with
  CHD and perf-counters. Isolated MXCC
  adds one repeated success line. The CHD child exits without a per-test success
  line; its parent asserts status and reopened content. Final logs pass (`netbsd/final-core-checks.log` and the later complete
  `netbsd/feature-tests-final.log`, including the independent fragment-write test).
- Core formatting, default/CHD+perf Clippy `-D warnings`, all four demos with
  and without CHD, release CLI. Default all-target check also passes a separately
  downloaded, checksum-verified Rust 1.74.0 toolchain.
- Integration preflight also passes the separate `helios-ui` locked offline
  check. The final SCSI-ID filename suggestion passes Heliacal formatting,
  Clippy, all 75 tests, release rebuild and SDL smoke
  (`merge-frontend-checks.log`). New test disk names retain their target ID;
  renaming the accepted NetBSD disk to `netbsd11-ss20-scsi3.chd` preserved its
  SHA-256 and saved NVRAM.
- Heliacal: 75 tests, Clippy, formatting and release rebuild. Actual SDL dummy
  smoke exercises creation → draft → discard (file kept) → attach → save,
  then named-machine operations. Original SS-5 display smoke passes at
  98,172,928 steps. Use an isolated config and `--graphical` for boot-smoke
  to avoid inheriting a user's headless choice; this is not an OS claim.
- Disposable `chdman` 0.288 conversions boot Solaris 2.6 to root on SS-5
  (128 MiB/1 CPU), SS-20 (128 MiB/2 CPUs) and Axil (512 MiB/1 CPU). Each writes
  a marker and 256 KiB zero file, syncs/halts, then a new process verifies:
  `3219093202 22 /helios-chd-test` and
  `3975907619 262144 /var/tmp/helios-chd-zero`.
  All six boots halt cleanly; SS-20 reports CPUs 0/2 online.
- Six SHA-256 values match before/after: all three source raw baselines and
  compressed parents. Only disposable children changed. Counts/timings are
  in `acceptance/results.json`; they are not controlled performance results.
- Real Xvfb Heliacal creates a 2 GiB CHD, saves target 1, confirms F5 and boots
  the existing SS-20 disk with `boot disk -r`. OBP probes targets 1/3; Solaris
  formats, labels and verifies the new target. Clean halt and UI Quit succeed;
  the owned PTY symlink is removed. Independent chdman extraction verifies
  sector-zero Sun label magic `dabe` and XOR checksum zero. `gui3/acceptance.log`,
  screenshots, `label.bin` and `label-check.json` retain the evidence. Earlier
  harness attempts needed the reconfiguration boot and actual label prompt.

## NetBSD installation acceptance

The supplied read-only NetBSD 11.1 sparc ISO completed a minimal installation
on a new 2 GiB CHD using SS-20, one CPU, 128 MiB, serial console and hardware
clock mode. A fresh normal GENERIC boot reaches root login and reports a clean
filesystem. Kernel/shell source checksums and two guest-written files match
after host reopening and an independent CHD/FFS read. Both install and disk
sessions sync/unmount and halt cleanly. Original ISO hash is unchanged.

This exposed and corrected the floppy base-byte probe, reserved V8 ASR reads
and absent-coprocessor traps used by OpenSSL. There is no floppy media engine.
The initial installation stopped before sync/unmount; independent inspection
found missing directory data. It was replaced by a fresh installation,
not repaired or promoted.

The accepted disk is `local/disks/netbsd11-ss20-scsi3.chd`. A GENERIC timecounter
warning, deferred entropy setup and SS-5's missing APC device remain limits.
See [NetBSD installation](netbsd-11-install.md) for exact evidence, hardware
sources, test credentials and reproduction. This is SS-20 serial/storage
acceptance, not NetBSD graphics, networking, SMP or all-model support.

CHD CD images remain unsupported; raw cooked ISO is the installation source.
Issue #6's hard-disk attachment and blank-image creation scope is complete.

## Cleanup

Earlier stale-session cleanup removed nine unused SMP slice-1 clones and three
obsolete builds (manifest `local/cleanup-after-clock-2026-11-15/`). Current
proof/MSRV builds and downloaded toolchain were removed too: 2,511,319,040
allocated bytes (`scratch-cleanup.json`). Shared Cargo caches, normal builds,
sources, saved machines and the user's guest are retained. Fifteen completed
CHD acceptance/GUI disk copies were removed after guest and independent label
checks; their paths and hashes are in `acceptance-media-cleanup.json` (567 MB
summed allocated size, including reflinks). Compact evidence and the accepted NetBSD disk remain. The interrupted
NetBSD scratch image, extracted archive/kernel duplicates and private runner
executables were removed after final verification; `netbsd/cleanup.json`
records those paths and hashes.
