# SCSI storage

Original SS-5 OBP 2.15 now completes `probe-scsi-all`, lists the attached disk
and CD-ROM, and returns to `ok`. The machine has one narrow SCSI bus with the
NCR89C100's ESP controller and DMA2 engine. Each attached target has LUN 0.

## Images

Both frontends accept these optional arguments:

| Option | Target | Image format | Access |
| --- | --- | --- | --- |
| `--disk PATH` | ID 3, LUN 0 | Raw disk, 512-byte blocks | Read-only |
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
sectors are not supported; supply converted raw images. Source files are
opened read-only, and WRITE commands report write protection.

Library callers attach images through
`machine.bus.devices_mut().unwrap().scsi.attach(id, MediaKind::Disk, path)` or
`MediaKind::Cdrom`. IDs 0–6 are available; ID 7 is reserved for the adapter.
Duplicate IDs and invalid images return errors before replacing a target.
Controller and machine resets retain mounted images.

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

Implemented commands are TEST UNIT READY, INQUIRY, REQUEST SENSE, READ CAPACITY(10),
READ(6), READ(10), and MODE SENSE(6) header/block descriptor without mode pages.
READ(6)'s zero transfer length means 256 blocks; READ(10)'s means zero blocks.
Reads use the image's block size and validate the requested range. A command
buffers at most 65535 CD sectors (about 128 MiB). Failed host reads return an
unrecovered-read-error CHECK CONDITION. Unsupported target opcodes/fields and
out-of-range reads return ILLEGAL REQUEST sense; writes return DATA PROTECT.
SCSI bus reset establishes UNIT ATTENTION, which INQUIRY does not consume.

Writable overlays, media eject/load, MODE SELECT, changing CD logical block
size to 512 bytes, CD TOC/audio, synchronous negotiation, select-with-ATN-stop,
disconnect/reselection and target-mode ESP commands remain pending. Discovery
and synthetic block-read tests do not establish Solaris/SunOS installation or
boot compatibility.

## Evidence

The [firmware checkpoint](firmware.md#scsi-target-discovery) records the original
OBP result. Firmware-free tests cover both INQUIRY personalities and capacities,
missing IDs and timeout timing, unsupported LUNs, image validation, PIO and DMA
reads, split transfers and residuals, reset attention, preloaded IDENTIFY,
chaining/terminal-count interrupts, arbitration denial and mapping-fault recovery.
Image-content comparisons verify that testing does not modify source media.
Primary NCR and SCSI-2 sources and pinned emulator comparisons are listed in
[references.md](references.md).
