# SCSI storage

Original SS-5 OBP 2.15 now completes `probe-scsi-all`, lists the attached disk
and CD-ROM, and returns to `ok`. The machine has one narrow SCSI bus with the
NCR89C100's ESP controller and DMA2 engine. Each attached target has LUN 0.

## Images

Both frontends accept these optional arguments:

| Option | Target | Image format | Access |
| --- | --- | --- | --- |
| `--disk PATH` | ID 3, LUN 0 | Raw disk, 512-byte blocks | Read-only |
| `--disk-rw PATH` | ID 3, LUN 0 | Existing raw disk, 512-byte blocks | Read/write in place |
| `--cdrom PATH` | ID 6, LUN 0 | Cooked CD data image/ISO, 2048-byte sectors | Read-only |

```sh
cargo run --release --manifest-path helios-ui/Cargo.toml -- \
  local/firmware/sun-ss5.bin local/firmware/turbogx.bin \
  --disk local/disk.raw --cdrom local/cdrom.iso
```

After entering the monitor with Stop-A (Pause+A), type `probe-scsi-all`.
The headless `helios boot` command accepts the same image options. Omit an
option to leave its target absent. Images must be nonempty regular files of
whole blocks, with at most 2^32 blocks. The backend does not interpret a
filesystem or partition table. QCOW2, compressed images and raw 2352-byte CD
sectors are not supported; supply converted raw images. Choose either `--disk`
or `--disk-rw`, not both. The writable option opens an existing image without
creating or truncating it. Guest writes modify that file in place; there is no
overlay. Other attachments remain read-only and report write protection.

Library callers attach images through
`machine.bus.devices_mut().unwrap().scsi.attach(id, MediaKind::Disk, path)` or
`MediaKind::Cdrom`. IDs 0–6 are available; ID 7 is reserved for the adapter.
Duplicate IDs and invalid images return errors before replacing a target.
Controller and machine resets retain mounted images.
Writable library attachment is explicit: `scsi.attach_disk_rw(id, path)`.

For a new installation, create a separate blank image first. This example
creates a sparse 2 GiB raw disk and refuses to overwrite an existing file:

```sh
mkdir -p local/disks
python3 - <<'PY'
with open('local/disks/solaris7-install.raw', 'xb') as disk:
    disk.truncate(2 * 1024**3)
PY
```

Attach it with `--disk-rw local/disks/solaris7-install.raw`, alongside the
read-only `--cdrom` image. The blank disk has no label or filesystem; the guest
installer creates the filesystems. Solaris 7 requires a Sun disk label before
its installer accepts a new blank disk. Boot the CD in single-user mode
(`boot cdrom -s`), run `format`, select the new disk, and answer its label
prompt. `verify` checks the label. The implemented geometry pages let Solaris
automatically configure a 2 GiB disk as 4094 data cylinders, two alternate
cylinders, 16 heads and 64 sectors per track. Then boot the CD normally for
installation. This labeling path has been verified with the original firmware
and Solaris 7 CD. Do not resize or modify an image while it is attached.

The ISO's physical sector size remains 2048 bytes. MODE SELECT can change the
CD drive's logical block size to 512, 1024 or 2048 bytes; capacity and subsequent
reads use that size. Four 512-byte logical blocks map to one ISO sector. The
default is 2048 bytes, restored by SCSI bus reset. The verified OBP/Solaris
bootloader paths use 2048-byte reads; 512-byte selection and reads are covered
by separate device tests. No image conversion is needed to change this mode.

## Controller and DMA behavior

The ESP supports select without/with ATN (`41`/`42`), IDENTIFY, six-, ten- and
twelve-byte CDB framing, TRANSFER INFORMATION, INITIATOR COMMAND COMPLETE and
MESSAGE ACCEPTED. Successful selection reports sequence step 4 and function
complete/bus service. An absent target reports disconnect at step 0 after
`timeout * 8192 * CCF / 40 MHz`, with CCF zero encoding eight. PIO input receives
one byte per transfer-information command; DMA supports partial transfers and
short target replies with residual counts. Status and COMMAND COMPLETE precede
the final disconnect. Selection/transfer interrupts require acknowledgement;
full stacked command and interrupt queues remain unsupported.
PIO/DMA DATA OUT now receives MODE SELECT parameter lists. Partial transfers
retain their position; parameters take effect only after the full list passes
validation. Invalid lists leave the previous block size intact.

DMA2 services one byte per CPU boundary through SBus master 5 and the shared
microSPARC-II IOMMU. This is functional scheduling, not SCSI wire timing or
burst timing. Address/count advance only after successful memory access;
arbitration denial retains the byte. Memory faults latch D_ERR_PEND, stop
DMA, and can interrupt at CPU level 4 without a CPU MMU fault. Host stops
preserve the pending transfer. D_WRITE must match the ESP transfer direction.

The write FIFO drains immediately. D_INVALIDATE clears memory error and
terminal count, without acknowledging the ESP interrupt. With counting enabled,
the 1-to-0 transition sets D_TC; D_TCI_DIS masks its interrupt contribution.
Chaining reloads the next address/count at expiration and stops when no next
buffer is valid. In chaining mode D_TC also supports write-one-to-clear.
Non-chaining DMA can continue through count expiration, as documented by NCR.

## Target command subset

The synthetic HELIOS target identity distinguishes these targets from physical
Sun/OEM drives. INQUIRY reports disk type 0 or removable CD-ROM type 5, with no
synchronous, wide or tagged-queue capability advertised. Unimplemented LUNs
report peripheral qualifier/type `7f`, rather than additional drives.

Implemented commands are TEST UNIT READY, START STOP UNIT without eject/load,
INQUIRY, REQUEST SENSE, READ CAPACITY(10), READ/WRITE(6)/(10), MODE SENSE(6)/(10)
and MODE SELECT(6)/(10) headers/block descriptors, plus disk format/geometry pages. CD-ROM
targets also implement PREVENT ALLOW MEDIUM REMOVAL.
MODE SENSE supports descriptor suppression, all-pages requests, and current,
default and changeable values. Disk pages 3 (format device) and 4 (rigid disk
geometry) describe a fixed virtual geometry: 16 heads, 64 sectors per track,
512-byte sectors, 5400 RPM, and enough cylinders to cover the image. READ
CAPACITY remains authoritative for a partial last cylinder; these values do
not describe the host's storage. A 2 GiB image has exactly 4096 cylinders.
There are no changeable geometry bits or saved modes. CD-ROMs expose neither
of these disk pages.

MODE SELECT accepts the descriptor form with either PF setting and rejects SP.
With PF=1 it also accepts unchanged disk pages; changed geometry is rejected.
Page 3 zero sector count/size request the existing defaults, and its interleave
field is ignored as SCSI-2 specifies. The complete parameter list is validated
before any change.
START STOP UNIT models readiness with immediate spin-up; stopped targets report
NOT READY until started. CD mode headers do not set the disk-only write-protect
bit; the device is inherently read-only. PREVENT retains a media-removal lock
until ALLOW or SCSI bus reset; attempting eject while locked returns MEDIUM
REMOVAL PREVENTED (`05/53/02`). Loading/ejecting media remains unsupported.
READ(6)'s zero transfer length means 256 blocks; READ(10)'s means zero blocks.
Reads use the image's block size and validate the requested range. A command
buffers at most 65535 CD sectors (about 128 MiB). Failed host reads return an
unrecovered-read-error CHECK CONDITION. Unsupported target opcodes/fields and
out-of-range accesses return ILLEGAL REQUEST sense; read-only attachments
reject writes with DATA PROTECT. Disk MODE SENSE write-protect status reflects
the chosen access mode. CDs remain inherently read-only.
SCSI bus reset establishes UNIT ATTENTION, which INQUIRY does not consume.

WRITE(6)/(10) uses the same PIO/DMA DATA OUT path as MODE SELECT and stages
the full payload before changing the image. Short transfers, DMA faults and
reset before completion do not commit incomplete data. Successful commands
write only the validated block range and call host `sync_data` before reporting
GOOD, including when FUA is set; no volatile write cache is advertised. DPO is
accepted without an emulated replacement policy. Host write/flush failures
return MEDIUM ERROR / WRITE ERROR (`03/0c/00`); a host I/O failure after writing
begins can leave partial on-disk data. Externally resized files are rejected
without growing them back.

Disk SYNCHRONIZE CACHE(10) validates the range and flushes the file; zero block
count covers the remainder of the image. IMMED and relative addressing remain
unsupported. Neither a zero-count WRITE(10) nor failed CDB validation changes
the image; WRITE(6)'s zero count means 256 blocks.

Writable overlays, media eject/load, other mode pages, CD TOC/audio, synchronous negotiation, select-with-ATN-stop,
disconnect/reselection and target-mode ESP commands remain pending. Kernel root
mounting and the serial language-selection prompt have been observed on both
Solaris CDs. Solaris 7 has also begun writing an installation to a new 2 GiB
disk; completion and installed-system boot remain unverified.

## Solaris CD checkpoint

Both supplied Solaris 2.6 5/98 and Solaris 7 11/99 SPARC images reach their
first bootloader output with unmodified OBP 2.15. At the graphical `ok` prompt:

```text
ttya output
boot cdrom -v
```

The first command redirects output to TTYA (the launching terminal), leaving
keyboard input active. The second enables verbose boot output. Both runs enter
boot code at `0x4000`; the first completed output lines, excluding spinner
characters, are:

| Image | Output |
| --- | --- |
| Solaris 2.6 5/98 | `Size: 243560+176918+41926 Bytes` |
| Solaris 7 11/99 | `Size: 255664+158146+39070 Bytes` |

The probe stops immediately after the newline, before further boot progress.
See [firmware.md](firmware.md#solaris-cd-bootloader-output) for hashes,
configuration, exact steps and evidence limitations.

Subsequent runs use `ttya io` to switch both console directions to serial,
then send `boot cdrom -v` through TTYA. Both load the Solaris kernel and mount
`sd@6,0:b` as UFS root. PREVENT ALLOW MEDIUM REMOVAL is required by the kernel's
CD driver; without it, root mounting fails after repeated CHECK CONDITION
responses. Both continue to the installer's `Select a Language` prompt on TTYA;
no installation choices are submitted. See the
[kernel checkpoint](firmware.md#solaris-kernel-startup).

## Evidence

The [firmware checkpoint](firmware.md#scsi-target-discovery) records the original
OBP result. Firmware-free tests cover both INQUIRY personalities and capacities,
missing IDs and timeout timing, unsupported LUNs, image validation, PIO and DMA
reads, split transfers and residuals, reset attention, preloaded IDENTIFY,
chaining/terminal-count interrupts, arbitration denial and mapping-fault recovery.
Image-content comparisons verify that testing does not modify source media.
Primary NCR and SCSI-2 sources and pinned emulator comparisons are listed in
[references.md](references.md).
