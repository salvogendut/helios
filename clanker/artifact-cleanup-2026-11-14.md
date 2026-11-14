# Artifact cleanup before SMP development

2026-11-14. The user requested cleanup of superseded work artifacts before
issue #2 development. The SMP work branch is `feat/sun4m-smp`; its initial
handoff is commit `3799eef`.

Completed cleanup:

- Removed 15 obsolete Cargo build trees from earlier isolated review/harness
  directories in `/tmp` and the ignored performance investigation directories.
  Retained their source, fixture, manifest and result files.
- Removed 21 superseded scratch disk copies: SX runs 7–20 and two menu
  experiments, the old `/tmp` performance disk, two early platform-review
  copies and two performance investigation working disks. Retained the
  accepted Run 21 disk, original installed disks and current clean Solaris
  baselines under `local/completion-2026-11-13/`.
- Compressed 73 large historical logs and guest-memory dumps in the old
  platform/page-list/panic and Solaris boot/install investigations. Each
  compressed file was decompressed and SHA-256 checked before removing the
  uncompressed original. These files now have an additional `.gz` suffix;
  use `gzip -dc` when revisiting an old note that names the original path.

The exact paths, original metadata, archive checksums and completion status
are in `local/cleanup-2026-11-14/manifest.json`. Its companion cleanup script
is retained locally for audit; it is not a general-purpose cleanup command.
Removed/compressed files account for a 9.921 GiB reduction in allocated file
extents. Reflink sharing means this is not a measurement of unique physical
space reclaimed.

No candidate was open by an inspectable same-user host process. The manifest
records the three unrelated protected desktop/authentication processes whose
file descriptors were unavailable. Preservation checks on 289 retained files
passed using device/inode, size, modification time and allocated-block metadata.
All 15 nonempty ROM/disk/CD paths in the active Heliacal configuration and its
three named machines still resolve to files.

Firmware, user machine definitions/media/NVRAM, accepted captures, current
release binaries/build trees, offline dependency caches, source harnesses,
performance reports and the new SMP reference downloads remain available.
Concurrent SMP source edits were not part of this cleanup.
