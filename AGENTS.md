# Helios development

Helios is a Rust Sun SPARC emulator. The first target is SPARCstation 5 / sun4m,
SPARC V8 / microSPARC-II at 85 MHz, 128 MiB RAM, CGsix graphics, SCSI disk/CD-ROM,
Ethernet and serial ports. Read README.md, HACKING.md and docs/references.md before
changing CPU, bus or device behavior.

- Follow IRIS's functional-emulation principles and word-transparent data path.
  Keep endian conversion at external byte boundaries, with lane shifts inside.
- Keep the library usable headlessly and from future frontends. Guest behavior
  must not depend on UI state; core code must not terminate the host process.
- Consult the SPARC manual and the documented MAME/QEMU revisions. Distinguish
  sun4c from sun4m, physical from virtual addresses, and emulator firmware
  interfaces from hardware. Record sources for new register maps and quirks.
- Use original Sun OBP firmware as the primary development/accuracy target.
  OpenBIOS stays a separate compatibility test until proved working; its QEMU
  interfaces must not redefine SS-5 hardware. See docs/firmware.md for pinned images.
- Report unsupported behavior explicitly. Do not claim firmware or OS support
  from a partial interpreter or a successful isolated probe.
- Preserve PC/nPC, window aliases and fault atomicity. Add meaningful CPU/device
  regression tests with expected state derived independently of implementation.
- Run cargo fmt --all --check, cargo clippy --locked --offline --all-targets --
  -D warnings, cargo test --locked --offline, and all built-in demos (demo,
  trap-demo, serial-demo and timer-demo). Distinguish instructions, synchronous
  traps, interrupt entry and error stops.
- Human documentation belongs in the root and docs/. Update it with behavior.
  Agent findings and local environment notes belong in clanker/ and its index.
- Keep guest firmware and disks under ignored local/. Do not overwrite source
  images during testing. No third-party implementation or firmware is bundled.

