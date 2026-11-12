# SS-20/Axil page-color panic: missing MXCC physical alias

2026-11-12. Reviewed `4195f42fe5a8554cd8d9cfb37917cc0138cb038b` plus the
uncommitted segment-probe and MXCC stream-completion fixes described in
[the preceding handoff](ss20-solaris-panic.md). Experiments are isolated in
`/tmp/helios-page-list-review`; the shared core, Heliacal binary and original
disk were not changed by this review.

## Confirmed cause

The `page_add()+0x24` alignment panic is caused by a missing physical alias
of the MXCC control register. It is not evidence of SCSI overwriting a page
pointer or of a register-window failure.

The current core keeps MXCC registers in `Srmmu`, accessed through ASI 2.
However, `Sun4mLayout::ss20().zero_windows` includes the physical CPU-0
module-register window at `0xff8c00000`. `SystemBus` returns zero there and
discards writes. The Axil profile inherits this layout.

| Access path | MXCC control address | Current behavior |
| --- | --- | --- |
| Processor ASI 2, doubleword | `0x01c00a00` | Retains the register |
| Processor ASI 2, low word | `0x01c00a04` | Retains the low word |
| CPU-0 physical control space, low word | `0xff8c00a04` | Reads zero; writes discarded |

Original SS-20 OBP executes this sequence while constructing the CPU node:

1. Earlier processor-side initialization has set MXCC control to `0x38`
   (PF, MC and PE).
2. OBP reads the physical control register and ORs in `CE=0x4`. The missing
   alias returns zero, so the attempted write is `0x4` and is discarded.
3. OBP reads it again and tests `PE=0x8`. It sees zero and omits the boolean
   `ecache-parity?` property.

The firmware does publish `ecache-nlines=0x8000` and
`ecache-line-size=0x20`. Those numbers are correct for 1 MiB, but Solaris
does not consume them on this path without `ecache-parity?`.

In the user's Solaris 2.6 kernel, `fill_cpu()` tests that property at
`f004827c`–`f004829c`. When it is absent, the branch skips cache geometry
initialization. The globals remain:

```text
f025e89c  mxcc_cachesize = 00000000
f025e8a0  mxcc_linesize  = 00000000
```

Later, `page_list_color()` calculates its color mask from
`(mxcc_cachesize / 4096) - 1`, yielding `ffffffff`. A PFN of `0x271` then
becomes an array index of 625 instead of being limited to 256 colors.

```text
page_freelists = f026c980, size = 1024 bytes (256 heads)
f026c980 + 4 * 0x271 = f026d344
f026d344 = cpu_pause_info, containing 40400ce2
```

The physical watchpoint at `07ead344` finds a legitimate store at
`f006dfd0`: Solaris saves the previous PSR in `cpu_pause_info`. Nothing
corrupted a list head. The out-of-bounds array access later treats this
saved PSR as a pointer. `page_add()` at `f0076508` executes
`ld [%g1+0x14], %g1`, faulting at address `40400cf6`.

These PCs, symbols and physical addresses are specific to the tested kernel
and SS-20 configuration. They were checked against live memory and the
user's ISO symbols, not inferred from the panic output alone.

## Counterfactual test

A small diagnostic patch routes CPU word accesses to physical
`0xff8c00a00`/`0xff8c00a04` to the existing ASI-2 control state after address
translation. With that one register shared, the read/write/read sequence is
`38 -> 3c -> 3c`, OBP publishes `ecache-parity?`, and `fill_cpu()` stores
`00100000` into `mxcc_cachesize` at PC `f00482dc`. The alignment panic goes
away on both SS-20 with native CG14 selected and Axil with CGsix selected.

An earlier experiment initialized the MXCC port identity to `08000104`
without fixing the control alias: the panic remained unchanged. Removing
that identity intervention while retaining only the control alias still
passes the panic. The missing control alias is sufficient to explain this
failure; changing CPU revision fields is not the remedy.

Both guests reach `Hostname: helios` and userland startup. The next failure is:

```text
mount: /dev/dsk/c0t3d0s6 no such device
/sbin/swapadd: /usr/sbin/swap: not found
WARNING - /usr/sbin/fsck not found.
```

The plain SS-20 run returns to `ok` after 470,558,720 total steps; the
earlier Axil alias run returns after 449,911,808 steps. Both report one
firmware startup reset, no host error stop, and zero SCSI writes. These are
userland-startup checkpoints, not complete Solaris or graphical-desktop boots.

## Implementation directions

Implement one authoritative CPU-module/MXCC register state shared by the
processor ASI-2 interface and the physical module-control decoder. Route the
physical window before the generic zero-window fallback. A bus-owned module
controller with a processor-side interface is preferable to duplicating
registers or adding guest-PC checks in the MMU.

The physical map for MID `n` has MXCC registers at `0xFFnC00000`, including
control at offset `0xa00`; the architected MBus identification address is
`0xFFnFFFFFC`. The observed OBP also reads the register-page port alias at
`0xff8c00f04`. Model populated and absent modules separately; do not alias
every MID to CPU 0. Keep the original OBP's property creation intact.

Preserve word-transparent lane ordering and review word/doubleword access
rules, reserved-bit masks and reset behavior. For the MBus control register,
the documented fields are `RC=0x200`, `PF=0x20`, `MC=0x10`, `PE=0x8`,
`CE=0x4`; reserved bits read zero and ignore writes. Parity-enable readback
does not by itself implement cache parity detection or a cache hit/fill model.

The included diagnostic patch is **proof of the cause, not a production
decoder to merge unchanged**: it covers only CPU word accesses to one MID's
control register, bypasses the normal bus request observation, and does not
provide direct-bus/doubleword aliases or complete register semantics.

Regression expectations for the production fix:

- ASI write `38`, physical read `38`, physical write `3c`, ASI read `3c`;
  clearing PE through either interface must be visible through the other.
- The same behavior through ASI `2f` bypass and through a valid 36-bit PTE;
  enforce translation protection and alignment before side effects.
- Direct physical bus reads and supported doubleword accesses see the same
  state; reserved bits, reset and absent MID behavior follow the hardware map.
- Original OBP publishes `ecache-parity?`; Solaris sets cache size to
  `00100000`, and the `page_list_color()` mask becomes `ff`.

The isolated proof includes two regressions for bypass and translated
coherence. Both fail against the user's current core (actual zero, expected
`38` / `8`) and pass with the diagnostic alias. The isolated tree passes
432 tests, formatting, Clippy with `-D warnings`, and all four demos.

## Next issue: migrate the SS-5 disk's device configuration

A read-only UFS inspection of the original installation confirms that
`/etc/vfstab` mounts `/usr` from `/dev/dsk/c0t3d0s6`, whose symlink is:

```text
../../devices/iommu@0,10000000/sbus@0,10001000/espdma@5,8400000/esp@5,8800000/sd@3,0:g
```

`/etc/path_to_inst` also contains those SS-5 addresses. The new machines'
boot path is instead:

```text
/iommu@f,e0000000/sbus@f,e0001000/espdma@f,400000/esp@f,800000/sd@3,0
```

Thus the installed device namespace must be migrated on a **cloned** disk.
Root can boot using the selected firmware path while `/usr` still refers to
the old controller. Do not treat the `/usr/sbin/fsck` message as proof of a
damaged filesystem. An Axil `boot disk -r` experiment still gets the same
missing-device failure and halts during shutdown within the 800-million-step
budget; `-r` alone has not repaired this installation.

Next, use a Solaris 2.6 rescue/miniroot or `boot disk -b` to inspect the new
instances and rebuild the clone's `/devices`, `/dev` links and instance
bindings together. Verify the resulting root, swap and `/usr` device names
against `vfstab` before normal startup. Use the tools belonging to Solaris
2.6 (`drvconfig`, `disks`, `devlinks`); do not assume a later Solaris
`devfsadm` workflow exists. Alternatively, test a fresh installation on the
new machine profile. Device migration and a full boot were not performed
in this review.

## Sources and local evidence

- [Sun-4M System Architecture, rev. 50](https://temlib.org/pub/SparcStation/Hardware/Sun4M_SystemArchitecture_edited2.pdf):
  B.III.5.4.3 (printed page 118), MXCC control fields; B.III.5.4.7 (page 120),
  MBus port fields; B.III.8 (page 124), physical module aliases.
- [Pinned QEMU CPU definitions](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/cpu.c):
  `cpu_sparc_set_id` and SuperSPARC MXCC identity are comparisons, not a
  replacement for coherent physical register decoding.
- [Oracle Solaris Transition Guide](https://docs.oracle.com/cd/E19455-01/805-6331/6j5vgg6a7/index.html):
  device paths, persistent instance numbers, `drvconfig`, and reconfiguration
  boot. The stale-path diagnosis above comes from the actual disk contents.
- [Local artifacts](../local/review-2026-11-12-page-color/): diagnostic patch,
  harness, before/after regressions, validation output, physical/Forth watch
  traces, serial logs and read-only disk metadata extraction.

Firmware, guest binaries, disk images and kernel memory dumps remain ignored
local data. None are included in the handoff patch.
