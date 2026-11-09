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
  --machine ss20 --steps 50000000
cargo run --release -- boot local/firmware/axil320-2-19.bin \
  --machine axil320 --steps 50000000
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
  SuperSPARC-II boot-bit position, wider context-table/replacement masks and
  ASI `0x05`/`0x06` I/D-TLB diagnostic storage. MXCC, cache-tag, breakpoint,
  cache-flush and TLB-diagnostic address spaces are accepted with functional
  storage or no-op behavior where the POST only checks access/readback.
- The SMC ECC memory controller at `0xf00000000` reports implementation 2,
  retains its enable/delay/fault/event registers and reports memory addresses;
  parity generation, injected errors and the correctable-error interrupt are not
  modeled.
- Empty CPU timer/interrupt slots and unpopulated VSIMM probe windows return
  zero and discard writes, matching an unpopulated machine. The low VSIMM
  `0x8000_0000..0x9c00_2000` windows, their legacy high aliases
  `0xf_f800_0000..0xf_f9c0_2000`, the SX placeholder at
  `0xf_8000_0000..0xf_8000_2000`, DBRI PROM/register placeholders at
  `0xe_e000_1000..0xe_e000_1030` and `0xe_e001_0000..0xe_e001_0100`, and the
  BPP placeholder at `0xe_f480_0000..0xe_f480_0020` are decoded. The SX/VSIMM
  framebuffer, DBRI audio and BPP are not modeled.
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
then rejects the configuration checksum, rewrites defaults, sets
`diag-switch?` true and resets. Stop-A skips POST and reaches OBP
initialization, but the serial output stops after the memory-bank probes while
OBP searches the device tree for `/obio/zs@0,0`; the loop persists past five
billion instructions. The Axil 320 ROM 2.19
(`axil320-2-19.bin`, SHA-256
`bc231c32d817d09e1d8192da0475a342f88b3ac2ed64e6fb67ab61e797e77425`) reaches
the same NVRAM reset path. These are not clean-POST or OS-boot claims. The CPU
remains the project's microSPARC-II-style V8 interpreter with a
SuperSPARC-compatible MMU profile; there is no SuperSPARC pipeline, cache-fill,
MBus multiprocessor, full PTP2 replacement policy, SX/VSIMM, DBRI audio, BPP or
Axil custom-controller model. The CGsix console remains the SS-5 SBus card
profile. No machine profile other than the SS-5 has an OS compatibility claim.
