# Solaris CD bootloader output

User authorized, in order: commit/push SCSI discovery, add CD commands, test
real Solaris media to first bootloader output. They added Solaris 7 alongside
2.6 and asked about 512-byte blocks. Both reached that stopping point. The later
request to try booting the CDs authorizes continuing toward the kernel and
installer prompt, stopping before installation and keeping source media read-only.

## Git and CI

SCSI discovery committed/pushed as `10fdbf481c32ed258dcaf8a768c9a9671887ebaa`.
Rust 1.74 Clippy flagged `&disk.0`/`&cdrom.0` at the last use of a Drop-bearing
test fixture (its suggested move would be invalid). Using `.as_path()` fixes it.
Correction `224507eea73b69ffde42dfb8e42c4505ae70f790` is pushed and all four CI
jobs passed: https://github.com/salvogendut/helios/actions/runs/42348194614.
The subsequent CD implementation belongs to a separate milestone; that earlier
CI result covers discovery only.

## Changes and limits

- ESP DATA OUT stages parameter bytes across PIO FIFO chunks or DMA counts;
  after the full list arrives, the target validates and commits the new mode.
- MODE SELECT/SENSE(6)/(10), 512/1024/2048-byte CD blocks, current/default
  descriptors, descriptor suppression and empty all-pages responses. No saved
  settings/pages. Zero parameter length is a no-op. Invalid/truncated lists
  return sense without partly changing the block size. SCSI bus reset restores
  the target's default block size. MODE SELECT does not modify image bytes.
- START STOP UNIT supports START/STOP plus IMMED, immediate functional spin-up,
  and stopped-state NOT READY. Eject/load remains explicit ILLEGAL REQUEST.
- Fix CD mode header: WP bit 7 belongs to disk, not CD. CD is intrinsically
  read-only. Response Debug reports buffer length, avoiding dumping image data.
- QEMU pinned scsi-disk.c and MAME pinned nscsi/cd.cpp were compared; SCSI-2
  clauses 8/9/14 define behavior. Primary validation-before-mutation semantics
  take precedence over emulator shortcuts.

## Evidence

Image names, sizes, hashes and exact transcript are in docs/firmware.md.
`local/media/` symlinks point to the supplied Downloads/Solaris Media files.
Both have Sun labels and ISO descriptors; source hashes were unchanged after
testing. The original extra 2.6 placeholder was incomplete and was not used.

`local/probes/cdrom-ss5/serial-probe.rs` boots 128 MiB with the existing graphical
configured NVRAM, keyboard and TurboGX. At step 60000000 it types `ttya output`
and `boot cdrom -v` on the emulated keyboard. Both enter PC 00004000 at step
116142551. Stops on first completed serial line after entry:

- Solaris 2.6: step 149954625, `Size: 243560+176918+41926 Bytes`.
- Solaris 7: step 148968160, `Size: 255664+158146+39070 Bytes`.

The probe logs each rising ESP IRQ with CDB/count/status and DMA state. Raw TTYA
transcripts retain spinner/backspace bytes. Initial graphical runs entered boot
code but were stopped too early for visible output (five million steps after
entry). Do not confuse firmware's `Boot device:` line with loader output.

These guest paths use 2048-byte logical blocks, READ(6) count 32 -> 64 KiB, and
do not issue MODE SELECT. Do not force 512-byte reads or claim a firmware test
of 512-byte mode: that behavior is independently tested with synthetic CDBs and
cross-sector image reads. No remaining CPU gap was encountered before the
requested stop. Broader CD commands and kernel startup remain future work.

## Validation

339 core tests, four UI tests, fmt/Clippy for both crates, release builds, and
all four demos passed locally. Four new SCSI tests verify 512-byte capacity and
cross-sector mapping, mode reset, data-out residuals/atomic invalid-list handling,
six-/ten-byte headers, no-op/unsupported save semantics, and drive readiness.
Check the CD milestone's own CI run separately from the earlier discovery run.
