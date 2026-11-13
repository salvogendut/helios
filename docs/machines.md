# Machine profiles

Helios builds every sun4m profile from the same device models and SPARC V8
interpreter. A profile selects its physical address map, PROM aperture, IDPROM
identity, MMU address width and DRAM bank layout. The default remains the
SPARCstation 5.

| CLI name | Machine | PROM base | IDPROM type | Default identity |
| --- | --- | --- | --- | --- |
| `ss5` | SPARCstation 5 | `0x70000000` | `0x80` | MAC `08:00:20:c0:ff:ee`, hostid `80c0ffee` |
| `ss20` | SPARCstation 20, one CPU | `0xff0000000` | `0x72` | MAC `08:00:20:c0:ff:ee`, hostid `72c0ffee` |
| `axil320` | Axil 320 (SS-20 clone) | `0xff0000000` | `0x72` | MAC `00:00:3c:13:de:ad`, hostid `72c0ffee` |

```sh
cargo run --release -- boot local/firmware/ss10-20-v2.25r-rom \
  --machine ss20 --keyboard --nvram local/ss20-ttya.nvram --steps 300000000
cargo run --release -- boot local/firmware/axil320-2-19.bin \
  --machine axil320 --keyboard --cgsix-rom local/firmware/turbogx.bin \
  --nvram local/axil-ttya.nvram --steps 300000000
```

`--nvram FILE` overrides the profile's default identity. The initial RAM
configuration is the SS-5-style 128 MiB unless `--ram-mib` is supplied; the
SS-20 profile lays RAM out in eight 64 MiB DSIMM banks with 16/32/64 MiB
modules, low slots first.

## SS-20/Axil implementation

- The high sun4m map places the IOMMU at `0xfe0000000`, SLAVIO at
  `0xff0000000` (PROM, keyboard, TTYA/B, NVRAM, floppy, timers, interrupt
  controllers and reset/status), and SCSI/Ethernet MACIO at `0xef...`.
  The same serial, NVRAM, timer, interrupt, SCSI, Ethernet and CGsix models are
  used; the IOMMU reports implementation 1/version 3 and exposes the additional
  arbiter-enable register.
- Instruction fetches use the 36-bit reference-MMU boot mapping while boot mode
  is set: `PA[35:28] = 0xff`, low address bits from the VA. ASI `0x20` is a
  32-bit bypass; ASI `0x21..=0x2f` assemble a physical address from the ASI's low
  nibble and the full 32-bit VA, which reaches the high control-space devices.
- The SS-20/Axil PROM aperture is a 16 MiB read-only alias: the image occupies
  the first 1 MiB and the remainder reads `0xff`. PROM write strobes are ignored
  for the reference profiles, matching OBP's low boot alias and its D-cache
  level-2 PTP POST writes.
- The reference MMU supports a 36-bit PTE physical page number, the
  SuperSPARC-II boot-bit position, 16-bit contexts, wider context-table/
  replacement masks and ASI `0x05`/`0x06` I/D-TLB diagnostic storage. MXCC, cache-tag, breakpoint,
  cache-flush and TLB-diagnostic address spaces are accepted with functional
  storage or no-op behavior where the POST only checks access/readback.
- The SMC ECC memory controller at `0xf00000000` reports implementation 2,
  retains its enable/delay/fault/event registers and reports memory addresses;
  parity generation, injected errors and the correctable-error interrupt are not
  modeled.
- Empty CPU timer/interrupt slots and unpopulated VSIMM probe windows return
  zero and discard writes, matching an unpopulated machine. The populated
  SS-20 SX/VSIMM windows route to the device before these empty-slot defaults.
  Low VSIMM probe windows `0x8000_0000..0x9c00_2000`, their legacy high aliases
  `0xf_f800_0000..0xf_f9c0_2000`, DBRI PROM/register placeholders at
  `0xe_e000_1000..0xe_e000_1030` and `0xe_e001_0000..0xe_e001_0100`, and the
  BPP placeholder at `0xe_f480_0000..0xe_f480_0020` are decoded. The SS-20
  SX/VSIMM framebuffer is modeled separately; DBRI audio and BPP are not.
- `--keyboard` attaches a Type 5 keyboard. With an SS-20 or Axil NVRAM image
  whose `diag-switch?` is false, OBP selects abbreviated startup and reaches
  serial `ok` without running the full POST cache-tag diagnostic. Without a
  keyboard, the headless SS-20 profile still runs full POST and reports that
  diagnostic.
- The built-in SX/cg14 subset is installed only for the SS-20. It models an
  8 MiB VSIMM 2 mirrored through the 64 MiB aperture, the SX register/control
  blocks and the ASI `0x28` memory alias at `0x800000000 + target PA`.
  `cg14` opens through OBP and draws the native 1152x900 console. MXCC state
  is shared by ASI 2 and the populated CPU-0 module-control alias at
  `0xf_f8c0_0000`; the architected port identity is read-only at
  `0xf_f8ff_fffc` and the `0xf_f8c0_0f04` page alias. MID 9 remains a zero
  window. Stream source/destination registers copy four doublewords through
  the regular bus and reject recursive register-window targets without
  panicking the host. Full MXCC error/control semantics remain incomplete.
  Monitor sense 4 selects the 1152x900 timing used by the renderer, and CLUT
  words are decoded as `00BBGGRR`. With these settings original OBP opens
  `cg14` and draws a native `ok` console. The Axil 320 shares the SS-20
  CPU/memory map but has no VSIMM or built-in SX device. The SX subset handles
  the observed Xsun fill, glyph, copy, outline and window-operation commands,
  including packed loads/stores, ROP_M and funnel shifts. Scanout decodes
  8/16/32-bit pixels and the raw/BGR/component-plane apertures; vertical
  retrace drives the video interrupt. Complete SX instructions and arbitrary
  programmed display timings remain unsupported. See [graphics](graphics.md).
- The integrated ESP accepts the SS-20/Axil 32-bit register lane accesses and
  test-mode probe commands, while the SS-5 profile retains byte-only access and
  strict reserved-register behavior.

## Evidence and limits

Original SMCC SPARCstation 10/20 POST 3.45 (`ss10-20-v2.25r-rom`, SHA-256
`b4f15bfafa69edfcea446778bd14b61b76273e4e77ddebe3a9002af757b48ca8`)
prints the CPU identity, reports CPU 1–3 absent, and passes the I/D-TLB
bit-pattern, flush, D-TLB cached level-2 PTP, `TLB_HIT`, `TLB_MISS` and
`TLB_PROBE` diagnostics. The PTP2/TLB diagnostics use a functional model:
PTP2 slots are filled from the level-1 table, a matching 256 KiB PTP2 can
override a 16 MiB leaf, fills are mirrored into ASI 5/6 storage, and ASI
diagnostic writes install real translations. The integer and FPU endianness tests pass after implementing the SuperSPARC-II
PSR.DE dual-byte-ordering conversions. The next full-POST diagnostic is the
CPU/MBus module cache-tag test at `0x4000_0000` (`exp[31:0]=0`,
observed `0x1e`), reported as a suspect MBus module. With a blank NVRAM the OBP
can reject the configuration checksum, rewrite defaults, set
`diag-switch?` true and reset. Use the documented initialized per-machine
NVRAM and keyboard setup for abbreviated startup. The Axil 320 ROM 2.19
(`axil320-2-19.bin`, SHA-256
`bc231c32d817d09e1d8192da0475a342f88b3ac2ed64e6fb67ab61e797e77425`) also
reaches serial `ok` with its initialized NVRAM. Full POST completion remains
unverified. CPU
identity is profile-specific: SS-5 reports the MB86904 microSPARC-II IU/FPU
identity, while SS-20 and Axil report SuperSPARC-II IU version 3 and FPU
version 0. The interpreter and reference MMU remain functional models; there is
no SuperSPARC pipeline, cache-fill, MBus multiprocessor, full PTP2 replacement
policy, complete SX acceleration, DBRI audio, BPP or Axil custom-controller
model. CG6/TurboGX cards can be attached through
`--cgsix-rom`; SS-20/Axil place them in SBus slot 2 at physical
`0xe_2000_0000`, matching the Sun-4M slot map. The Axil 320 uses a CG6 as its
usual framebuffer; Helios does not require the SS-5 AFX presence byte for it.
Axil/SS-20 SCSI discovery now works through the standalone IOMMU: OBP
`probe-scsi-all` lists an attached ID 3 disk. The key fix is honoring the
sun4m Arbiter Enable Register at `+0x1008` and masking IOPTE PPN bits 26:8
instead of the wider reference-MMU CPU PTE mask.

Both SS-20 and Axil have booted installed Solaris 2.6 disks to console login.
Disks copied from SS-5 need their device links and `path_to_inst` migrated to
the reference-profile device paths. A migrated Axil clone also passed a
write/sync/halt/reboot persistence check. These are specific firmware/disk
checkpoints, not a general OS compatibility guarantee.

Native SS-20 Xsun/olwm tests now preserve text through 20/1/2/3-pixel moves,
uncover previously hidden terminal contents, switch between Full Size and
Normal Size, and restore an iconified terminal with identical client pixels.
These tests drive the headless core. Heliacal also passes graphical-console
startup, xterm typing, mouse dragging with an unchanged translated client,
and capture/release checks on Linux/X11. Fresh SS-5 and Axil disk clones reach
login and return to OBP after a clean halt. See [graphics](graphics.md) for the
current device subset.
