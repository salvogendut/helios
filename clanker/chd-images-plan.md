# CHD images: issue #6 implementation handoff

Prepared 2026-11-15 for [Helios #6](https://github.com/salvogendut/helios/issues/6),
selected by the user after the clock/serial fixes were merged. This records
the original staged plan. The user subsequently asked this agent to implement
the feature: see [current progress](chd-images-progress.md) for the actual
code and validation. The old slice-A handoff below is historical, not an
instruction to discard or repeat completed work.

Both repositories have a local `feat/chd-images` branch:

| Repository | Starting commit |
| --- | --- |
| Helios | `f62911f6261e1f6798f0835331c5df1bea307e9e` |
| `../heliacal` | `865135614fe51da5cbed8cd48ea149df05abf874` |

Read both AGENTS.md files, Helios README.md, HACKING.md, docs/references.md and
docs/scsi.md. The accepted baseline is 636 unique core tests, 638 with
perf-counters, and 69 Heliacal tests. Original-firmware SS-5, SS-20 and Axil
Solaris boots pass with the limitations recorded in the clock/serial handoff.

## Outcome and scope

Mount CHD hard disks through the existing SCSI attachment APIs and CLI, keep
guest writes across reset and application restart, and create a new blank CHD
from the CLI and Heliacal. Preserve raw disk and ISO behavior and existing
named-machine records. The guest sees disk sectors, never CHD headers or maps.

Start with hard-disk CHDs with 512-byte sectors. Complete the writable hard-disk
and blank-creation slices before adding single-track MODE1 data-CD CHDs. Data
CDs stay read-only and expose a cooked 2048-byte stream; preserve Solaris's
existing 512/1024/2048-byte logical CD block selections. Mixed/audio/GD/DVD/AV
images, arbitrary parent chains and unsupported versions/codecs must fail
explicitly. Do not claim those formats from the library's broader capability.

No CPU, MMU, ESP timing, JIT, new machine model, automatic firmware change,
partitioning, filesystem formatting or rewriting of existing source media is
part of this issue. Do not silently convert configured raw disks to CHD.

## Inspected starting points

| File | Relevant behavior |
| --- | --- |
| `src/devices/scsi.rs` | ScsiTarget owns a File. Attach validates host length as capacity. Reads seek/read; a complete DATA OUT buffer is staged before writes; writes call sync_data before GOOD. SYNCHRONIZE CACHE also syncs. |
| `src/main.rs` | `--disk` is read-only, `--disk-rw` writable, `--cdrom` read-only. Existing attachments never create/truncate files. |
| `tests/scsi.rs` | Existing sense, block-size, raw write, reset/DMA-failure and host-I/O-failure regressions must remain. |
| Heliacal `src/settings.rs` | Save currently validates physical file size modulo sector size, then opens writable disks with OpenOptions. Both are wrong checks for a compressed parent with a writable child. |
| Heliacal `src/emulator.rs` | Worker constructs the machine and media before replacing the old guest. Keep that transaction and its error handling. |
| Heliacal `src/main.rs` | Media picker already lists chd, although the backend cannot decode it. New image creation needs a separate explicit action. |
| Heliacal `src/overlay.rs`, `src/profiles.rs` | SCSI slots, independent drafts, named-machine persistence and confirmed F5 replacement already exist. |

The raw backend's external-file-length check must remain for raw images.
CHD container length is neither guest capacity nor a stable write invariant.
Do not spread format-specific branches throughout ESP or DMA handling.

## References and dependency investigation

Local sources inspected (read them; do not copy an entire sibling emulator):

- IRIS `db49ed4beb9b533a3669d6fcb5f12480e52dc16f`: `src/chd_disk.rs`,
  `src/block_node.rs`, `src/block_node/chd_cow.rs`, `src/exit_persistence_tests.rs`
  and STORAGE.md. Useful evidence includes parent/child drop ordering, durable
  map writes, writing zero over an inherited nonzero hunk, hashless parents and
  reopen/persistence tests. IRIS auto-fold behavior is not required here.
- Prismatic `203147116e8259e857e0854315eb29d81251fe36`:
  `src/disk_image.rs` and `src/media.rs`. Inspect safe temporary publication,
  read-only inspection, collision checks, progress/cancellation and the reason
  CHD creation uses a child process when native cancellation does not finish
  promptly. Its feature macros and IRIS-only environment variables do not
  belong in Helios.
- MAME machine-reference checkout: `ed59c15fe78596ec6ff8fe04482ede78e6122e34`.
  The CHD library embeds its own MAME revision; record that separately.
- [MAME chdman documentation](https://docs.mamedev.org/tools/chdman.html):
  use info, verify, createhd and extracthd as independent format oracles.
- [libchdman-rs upstream](https://github.com/danifunker/libchdman-rs): preferred
  candidate because the sibling applications already use its read/write APIs.

Do not infer a resolved dependency from Cargo.toml alone: both sibling
manifests request 0.289.2, but the inspected IRIS lock resolves **0.289.5** and
Prismatic resolves **0.289.3**. The cached 0.289.5 source is available under
`/tmp/heliacal-cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.
Its README says Rust 1.75+, whereas Helios declares and tests 1.74. Its package
does not declare rust-version. Prove the effective compiler floor, including
transitive dependencies, instead of assuming the README is sufficient.

Preferred build arrangement to investigate in slice A: optional core `chd`
feature, preserving the dependency-free default build and Rust 1.74 lane;
Heliacal enables that feature. Document a CHD-enabled core CLI build. Check
that optional dependencies/lockfile syntax still permit the default 1.74
build. If this cannot be preserved, report the exact conflict and a proposed
toolchain migration before changing that contract.

Version 0.289.5 packages native sources, while its `prebuilt` feature can
download static archives during build. Prefer a pinned source build for the
offline validation lane. Explicit dependency fetch/setup may use the network;
the locked offline checks must then work without hidden downloads. Check the
upstream notices for the exact chosen package and include required notices;
do not vendor third-party implementation into this repository. Keep Helios's
own `unsafe_code = forbid`. The machine is constructed on its owning worker;
there is no need to copy IRIS's unsafe Send implementations.

## Storage contract

Introduce a small byte-addressed media abstraction, implemented by raw and
CHD media, with logical length, read_exact_at, write_all_at, durable sync and
read-only inspection. An enum or narrow trait is sufficient. Preserve target
sense state, transfer staging and guest-visible block sizes in ScsiTarget.
Only complete, validated DATA OUT commands reach the media backend. A host
I/O failure can still be partial, as today, but must never report GOOD.

Detection uses the eight-byte MComprHD signature. A corrupt/unsupported CHD
must never fall through to raw sector writes. A CHD suffix with a malformed
header should produce a useful error. Verify renamed CHDs and truncated
headers. Feature-off builds must reject recognized CHDs explicitly.
Capacity is the validated logical length, with checked arithmetic and the
existing SCSI bounds. Validate media type, sector size, metadata and geometry;
record how CHD CHS metadata relates to the current SCSI geometry pages, without
silently changing raw geometry or truncating the guest's final sectors.

Writable policy for the initial implementation:

- Raw writable disks continue writing their selected file in place.
- Uncompressed CHD disks may write in place if no existing child is present.
- Compressed CHD parents stay read-only; writes use a persistent uncompressed
  child named by appending `.diff.chd` to the full base path. Reuse a valid
  existing child rather than starting a fresh disk view.
- Opening a disk read-only still exposes an existing valid child, but neither
  creates a child nor modifies any file. Validate parent identity, geometry,
  hunk size and sector size; a corrupt/mismatched child is an error, not a
  reason to ignore its data or replace it.
- A child must outlive no parent it references. Keep their lifetimes and drop
  order explicit. Never auto-merge/delete a child on halt, F5 or application
  exit. Document that parent and child must be kept together.
- A hashless uncompressed base can be written directly. Do not create a native
  child for an unsupported hashless parent or use raw byte offsets into the
  container. A separate persistent sparse-overlay design is a follow-up, not
  an implicit fallback. Existing incompatible sidecars must be preserved.

Durability is a blocking acceptance criterion: native map/data buffers must
reach the backing file in the required order before host sync succeeds.
Drop alone cannot return errors to the guest. Inspect the exact library I/O
callbacks and IRIS's custom I/O wrapper. Test both data and new map allocation,
error propagation and reopening in a separate process. Do not assume calling
sync_data on an unrelated handle flushes a native library's private buffer.

## Ordered slices and review stops

### A — dependency proof and raw storage boundary

1. Build a small local proof with the exact candidate dependency. Create/read
   an uncompressed image, a compressed parent and a writable child; establish
   capacity, read-only behavior, parent lifetime and explicit durable writes.
   Verify independently with chdman. Probe an inherited hunk overwritten with
   zeros, and a hashless parent. Record actual compiler/build requirements.
2. Refactor ScsiTarget behind the narrow media abstraction with raw behavior
   unchanged. Add read-only format inspection and safe unsupported-CHD handling.
   No CHD guest boot, UI or implicit sidecar creation in this slice.
3. Run required core checks, normal/perf variants, demos and frontend baseline
   checks; report the proof and unresolved library behavior. Stop for review.

### B — CHD hard disks and persistence

Implement validated CHD reads, writable uncompressed images and compressed
parent/child attachments. Expose these through existing library/CLI paths.
Retain the full SCSI error and reset contract. Add tests for first/last sector,
hunk boundaries, nonuniform data, zero overwrites, bounds, read-only rejection,
parent mismatch, malformed map/metadata, failed sync and separate-process
reopen. Run a disposable Solaris disk boot/write/sync/halt/reboot, including
an independent parent hash check. Stop for review before image-creation UI.

### C — explicit blank-image creation

Provide a core helper and CLI command, proposed spelling
`helios create-disk OUTPUT --format chd --size-mib N`. A 2048 MiB disk is a
useful Solaris default. Use uncompressed CHD v5, 512-byte sectors, exact
logical length and documented geometry; do not silently round requested size.
Raw sparse creation may share the API if it does not complicate this slice.
No Sun label, partition or filesystem is manufactured on the host.

Creation uses a private sibling temporary file and atomic no-replacement
publication. Refuse existing files, symlinks (including dangling ones) and
colliding sidecars. Validate the result before publication; flush completed
content and directory state as supported. Failure/cancellation removes only
the temporary files owned by this operation. Do not allocate the whole disk
in RAM. Ensure zero reads, exact capacity and successful guest writes after
reopening; do not mistake a hashless new uncompressed image for corrupt media.

### D — Heliacal integration

Follow `../heliacal/clanker/chd-images-plan.md`. Save must inspect media
without creating children or acquiring writable guest media. Replace physical
file-length validation with shared logical-capacity validation. F9 SCSI offers
an explicit New disk action with destination, format and capacity; show
progress/error/cancel if work is not immediate. Keep SDL responsive and do
not allow a slow native compressor to block shutdown indefinitely.

Creation updates only the selected SCSI slot's draft on success. The completed
file is a user artifact: discarding later settings must not delete it. F9
Save/Discard/Cancel and confirmed F5 keep their existing meanings. Label,
NVRAM identity and unrelated target paths are retained. Explain where writable
CHD changes reside and preserve the global Advanced read-only policy.

### E — data CDs and final acceptance

Add the bounded single-track MODE1 CD path if the earlier slices are accepted.
Expose cooked user data and reject unsupported track layouts. Test 512/1024/
2048-byte logical selections, cross-sector reads and reset to 2048. Compare
payloads against an independently extracted ISO and boot the Solaris CD.

For closure: boot existing Solaris 2.6 CHD disks on SS-5, SS-20 and Axil using
private converted copies of the correct per-platform disks. Verify writable
markers after clean halt and a new process. Exercise one dual-CPU MBus case,
CHD read-only mode, the new blank disk through Solaris format/label and a
fresh installation/reboot, and the real Heliacal create/select/save/F5 flow.
Include failed replacement, cancellation, close/reopen and raw/ISO controls.
Keep hardware settings and clock mode matched across comparisons. Never claim
OS acceptance from a successful info command or only the OBP banner.

## Validation, evidence and cleanup

Run core fmt, locked offline Clippy/tests and four demos in the supported
default, chd and chd+perf-counters configurations. Keep the supported MSRV
lane honest and update dependency setup in CI. Heliacal requires its package
fmt, locked Clippy/tests, release rebuild, SDL smoke and real-firmware/OS tests.
Do not add firmware or OS bytes to tracked fixtures; generate small synthetic
fixtures with independent expected sector contents and cross-tool comparisons.

Use ignored local directories for disposable disks and evidence. Do not use
the active user's disk as a scratch conversion or guest-write target. Preserve
source hashes/metadata, small logs, fixture sources, exact commands and useful
captures; remove completed clones, temporary conversions, builds and helper
processes. Keep the user's running frontend untouched.

The pre-issue cleanup is complete: nine obsolete SMP boot copies and three
harness build trees removed; 59 protected source/baseline/catalog/media files
retained. Audit: `local/cleanup-after-clock-2026-11-15/manifest.json`. Live
user writes to the SS-5 disk were recorded separately from cleanup activity.

The user also queried headless keyboard/CDE behavior. Current headless mode
has no framebuffer but retains an emulated keyboard for abbreviated original
PROM startup. Disk-free no-keyboard probes reach ok on all three profiles;
SS-5 emits a synchronous-trap diagnostic and SS-20 a TLB/POST error. Evidence:
`local/headless-keyboard-2026-11-15/`. These are not OS-boot or full-POST passes.
The CDE banner was confirmed to be text on serial; Solaris's enabled dtlogin
service is a guest setting. Do not silently detach the compatibility keyboard,
patch PROM output or edit guest CDE startup as part of CHD work.

## Pasteable first assignment

Work on Helios `feat/chd-images` for issue #6. Read AGENTS.md and
`clanker/chd-images-plan.md`, then implement **slice A only**: verify the
candidate CHD library's build/MSRV and durable-I/O contract in an isolated
proof, introduce the minimal raw media boundary without changing SCSI behavior,
and make recognized unsupported CHDs fail explicitly. Preserve the default
raw/offline/MSRV contract, source disks and the user's active Heliacal session.
Run the specified checks, record findings and exact limitations, clean your
scratch builds/media, and return the changes uncommitted for review. Do not
start writable CHD guest integration, blank creation, frontend changes or POST
work until the first slice is reviewed.
