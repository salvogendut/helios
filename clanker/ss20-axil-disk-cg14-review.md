# SS-20 / Axil 320 disk and native CG14 review

2026-11-12. Reviewed Helios `ss20-axil320` at
`9b8716a85c28fc6c415c5ccb1e8725963524c019` and Heliacal at `0757652`.
Experiments used an exported tree under `/tmp/helios-platform-review`.
Shared implementation files and original disks were not changed.

Two blockers were reproduced and isolated:

1. Both profiles advertise a microSPARC-II integer unit to Solaris. Changing
   just PSR implementation/version to SuperSPARC-II gets both beyond the first
   boot failure and into the SunOS 5.6 banner. A later memory-management panic
   remains; this is not a successful Solaris boot.
2. Native CG14 fails `open-dev` because its false 16 MiB size exhausts OBP's
   virtual mapping space. An experimental 8 MiB backing store with mirroring
   makes `open-dev` succeed. With monitor sense matching the renderer, original
   OBP displays a readable native CG14 `ok` prompt.

## Evidence and reproduction

Local artifacts are in [local/review-2026-11-12](../local/review-2026-11-12/):
logs, the diagnostic harness, separate experimental patches, and
[native-cg14-ok.png](../local/review-2026-11-12/native-cg14-ok.png).
They are intentionally ignored. The patches are experiments, not production
implementations; limitations are described below.

The user's Heliacal configuration uses 128 MiB, original machine-specific OBP,
TurboGX, and writable target 3 `local/disks/solaris26-install.raw`.
Heliacal's dependency is `helios = { path = "../helios" }`; rebuilding Helios's
CLI alone does not update that linked library in the Heliacal executable.
The frontend binary inspected was newer than the reviewed core commit.

Headless reproductions used the same machine, RAM, ROM, CGsix and disk contents,
with a Type 5 keyboard attached. The disk was cloned separately for each
machine. Console output was redirected through existing valid serial NVRAM
images; host NAT and window presentation were omitted. Thus these are core
reproductions of the Heliacal configuration, not automated frontend UI tests.

The harness accepts:

```text
platform-review <ss20|axil> <output-dir> <commands-file> <cg6|sx|none> <disk-copy|none> [nvram-image]
```

Use `probe-scsi-all` followed by `boot disk`. The default stop marker is
`SunOS Release`; set `PLATFORM_STOP='kmem_getpages: pp is NULL or free'` to
capture the later panic. `PLATFORM_STEPS` bounds the run (default 800 million).
Step counts include executed instructions, synchronous traps, interrupt entries
and software resets; the logs report those categories separately.

| Experiment | SS-20 | Axil 320 |
| --- | --- | --- |
| Unmodified reviewed commit, CGsix | Target 3 discovered; 114 reads, 0 writes; host stop at step 250,325,618 | Target 3 discovered; 113 reads, 0 writes; host stop at step 252,569,014 |
| Only IU identity corrected | SunOS 5.6 banner, then `kmem_getpages: pp is NULL or free` panic | Same banner and panic |

The baseline stops are identical: PC `f00412f4`, nPC `f00412f8`, PSR
`04000fc0`, invalid level-2 supervisor-write translation for VA `f023ffc4`.
The instruction/trap counts and final page walks are in `probes/*-base/run.log`.
Do not start by allocating a missing stack page at that final fault: repeated
kernel traps lead up to it, and it is later than the initiating failure.

## 1. Make CPU identity coherent before debugging more SCSI

[Cpu::psr](../src/cpu.rs) always ORs in `PSR_VERSION = 0x04000000`, including
the SS-20 and Axil reference-MMU profiles. Their MMU/MXCC identities and OBP
banner describe SuperSPARC-II. The pinned
[QEMU CPU definitions](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/cpu.c)
give TI-SuperSparc-II IU `0x40000000`, FPU version 0, MMU `0x08000000`,
MXCC `0x104`, eight windows, and 16-bit contexts.

`cpu-identity-experiment.patch` changes only the value returned by `psr()` for
the reference-MMU profile. It is sufficient to get both test boots to the
Solaris banner. Make the real fix an explicit CPU-model configuration:

- Preserve SS-5's microSPARC-II identity.
- Configure IU, FPU and MMU together; do not equate every possible reference
  MMU with one SuperSPARC revision.
- Preserve the selected model through power-on and guest reset. WRPSR must
  not change implementation/version bits.
- Fix the separate hardcoded Meiko FSR version in [fpu.rs](../src/fpu.rs).
  That mismatch is confirmed by inspection, but was not isolated as the cause
  of the later panic.
- Test guest RDPSR/FSR reads, reset, and attempted writes to read-only identity.

The existing SCSI fixes in `9b8716a` are effective for discovery: standalone
IOMMU arbitration at `+0x1008`, shared onboard enable bit 20, and IOPTE physical
mask `0x07ffff00`. Do not reintroduce CPU PTE type bits into DMA addresses or
redo the old arbitration fix. Successful reads do not certify every SCSI/DMA
path, but the common boot failure above is not evidence of target selection
being broken.

## 2. Fix CG14 size/scanout, then expose it in Heliacal

Current [sx.rs](../src/devices/sx.rs) allocates 64 MiB of independent VRAM.
OBP's size probe therefore publishes a 16 MiB framebuffer:

```text
cd /obio/cgfourteen@2,0
.attributes
reg ... 00000004 00000000 01000000
" /obio/cgfourteen@2,0" open-dev .
0
```

Tracing the actual Forth execution shows `install` entering `frame-buf-map`,
then throwing the ROM string **Insufficient virtual memory** (`ffd5b6d6`).
The enclosing package open catches it and returns zero. There is no new guest
data-access trap, and CG14 generation stays 1933 with MCTL `00` throughout this
failed open. This is a mapping allocation failure before display initialization.

An 8 MiB backing store with address mirroring changes the reported size to
`00800000`. With everything else unchanged in that comparison, `open-dev`
returns `ffef3230`, generation reaches 200803 and MCTL becomes `41`.
See `probes/sx-mirror8/`. The subsequent console experiment sets monitor sense
4 and fixes palette channel order; `" screen" output` produces the saved
1152x900 image containing the native `ok` prompt. Input in this experiment
remained serial; native keyboard console and Solaris graphics were not tested.

Production work should cover:

- Model an actual 4 or 8 MiB VSIMM and its aperture decoding. Keep aperture
  size separate from installed storage. The experimental blanket modulo
  across all windows is only a size-probe/8-bit-console demonstration.
- Decode the real raw, CBGR and component-plane aliases against common storage.
  Their offsets are `0`, `01000000`, `03000000`, `03400000`, `03800000`,
  `03c00000`; they are not six independent RAM areas.
- Make monitor sense and scanout agree. The present zero sense selects
  1024x768, linebytes `0x400`, while the renderer scans 1152x900. Sense 4 selects
  the renderer's 1152x900 mode in this ROM. Ultimately expose active dimensions,
  stride, depth and blanking correctly instead of assuming one geometry.
- Interpret CLUT words as `00BBGGRR`. The renderer currently swaps red/blue,
  and `tests/sx.rs` repeats that incorrect expectation. Use independent red,
  green and blue test vectors from the hardware layout.
- Separate CG14 scanout from the SX processor. The ordinary OBP console uses
  `fb8` routines and can work before implementing SX acceleration. The present
  `0x800000000` plain VRAM alias is not SX instruction execution: SX memory
  operations use `0x800000000 + target physical address`, so a framebuffer
  target at `0xfc000000` is addressed through `0x8fc000000`.

The [NetBSD register definitions](https://github.com/NetBSD/src/blob/trunk/sys/arch/sparc/dev/cgfourteenreg.h),
[driver](https://github.com/NetBSD/src/blob/trunk/sys/arch/sparc/dev/cgfourteen.c),
and [SX definitions](https://github.com/NetBSD/src/blob/trunk/sys/arch/sparc/dev/sxreg.h)
provide independently maintained layouts. The
[CG14 FCode disassembly](https://github.com/mdehling/cg14-vesa-mod) was verified:
its 50,108-byte FCode is byte-identical to local original OBP offsets
`0x31848..0x3dc04`. ROM SHA-256:
`b4f15bfafa69edfcea446778bd14b61b76273e4e77ddebe3a9002af757b48ca8`.
No third-party code or firmware was added to tracked files.

Heliacal currently prevents a native test: `Model::requires_cgsix()` returns
true for every model, and `src/emulator.rs::build` calls `machine.remove_sx()`
when constructing the SS-20 with its required CGsix ROM. Its help text still
says built-in SX. Add an explicit native-CG14 versus SBus-CGsix selection,
retain CGsix for Axil, then rebuild Heliacal and verify the selected device.
Changing core CG14 alone cannot affect a frontend instance that removes it.

## 3. Continue from the first Solaris panic, not its reboot aftermath

The IU correction exposes `panic: kmem_getpages: pp is NULL or free` on both
machines. Capture the first panic and its calling addresses before reboot.
Trace the allocated VA, independently walked PA/PFN, and Solaris's physical
page lookup. Compare them with OBP memory/available properties. This will
distinguish a wrong translation/probe result from stale page contents or
incorrect memory accounting. The final `f00412f4` failure after resetting is
not a useful first breakpoint.

There are concrete CPU-memory gaps worth separate tests:

- Reference `probe()` still masks full PTE results with SS-5's 31-bit physical
  mask (`07ffffff`, and `07ffffdc` for hits). That loses high physical bits in
  SS-20 MMIO/PROM mappings. Test both a fresh walk and a cached probe.
- Lower-level probes do not return a valid large-page leaf encountered earlier
  in the walk. Compare SuperSPARC semantics with the pinned
  [QEMU mmu_probe](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/mmu_helper.c);
  keep documented microSPARC-specific behavior separate.
- Context-root lookup and TLB matching still mask context to eight bits. Test
  distinct reference contexts `0x0000` and `0x0100`; the SuperSPARC-II model
  should not alias them. Current failing boot uses context zero, so this is
  not established as its cause.
- Audit the POST-specific kind-1 probe and `reference_subregion()` shortcuts.
  The kind-1 path indexes from an already indexed L1 entry; normal fast and
  slow translation paths also differ in whether they consult subregions.
  Add independent descriptor-walk tests and fast/slow consistency checks
  before extending these shortcuts.

The private `mmu-probe-experiment.patch` preserves high bits and accepts early
large-page leaves. Axil still panics, so it is not a demonstrated fix for
`kmem_getpages`. It was not a comprehensive reference-MMU correction.

## 4. Implement MXCC transfer effects, with their own tests

`Mmu::mxcc_write()` currently stores source/destination register values only.
It never moves the stream data to/from memory. Guest code actively uses these
registers, including OBP's memory clear/allocation routines.

The pinned
[QEMU ASI implementation](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/ldst_helper.c)
implements four doublewords: source `01c00100` loads the 32-byte stream buffer,
destination `01c00200` stores it. Use the SuperSPARC/MXCC documentation to
define address/control bits, alignment and errors; QEMU's 32-bit address mask
and explicitly uncertain error handling are not a complete hardware spec.

The RAM-only experiment performs those transfers below `0x20000000` and
leaves other operations unchanged. It does **not** remove the first Solaris
panic. It does allow subsequent reboot attempts to continue instead of the
earlier terminal stack fault, but is not a finished implementation. Test
zero-fill, nonzero copy, buffer-register ordering, alignment, inaccessible
memory, and partial-failure behavior. Keep this separate from SCSI DMA.

## Checks and recommended sequence

Baseline `cargo fmt --all --check`, Clippy with `--locked --offline
--all-targets -- -D warnings`, and all four demos passed in an isolated export.
The first full test run was blocked by sandbox restrictions on two local UDP
socket tests; the rerun with loopback sockets enabled passed all 424 tests and
is recorded separately in the local artifacts. These checks describe the reviewed baseline, not
production validation of the diagnostic patches.

Suggested order: coherent CPU identity; CG14 VSIMM/scanout plus Heliacal
selection; first-panic VA/PFN tracing and reference-MMU tests; complete MXCC
stream effects; then repeat disk boot and write/read checks on disposable disk
copies. Maintain SS-5 regressions throughout. A readable native OBP prompt and
a Solaris startup banner are separate milestones, neither of which establishes
a successful SS-20/Axil Solaris boot.
