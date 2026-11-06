# ESP initialization and completed RAM test

- MACIO DMA commit ff0cc89 pushed; CI 42297724691 passed.
- NCR53C9X initialization now passes original OBP. Byte registers at 78800000,
  word spacing; count/counter separate, FIFO 16 entries/overwrite status,
  reset retention, config 1/2/3, NOP/flush/reset/selection-enable-disable.
  Chip ID 02 only after FE + DMA NOP and before high-count write; primary
  integrated-core manual, not unconditional QEMU ID read. IRQ through DMA2
  to NCR source 18, CPU level 4. Core held reset by DMA2 D_RESET.
- New PDF evidence corrected earlier DMA commit's ID width assumption:
  primary table 2-7 footnote 3 permits byte/halfword ID reads. PDF page 50
  checked visually, printed 2-25/2-26. /tmp/helios-classic-io.pdf.
- Data transfer/selection, diagnostic modes, stacked command/interrupt handling
  and wide ESP accesses remain explicit stops. The reset pulse is a functional
  timing choice, 130 CCF clocks at 40 MHz; hardware RESETO is absent in MACIO.
- The first next trap at step 103438914 is an EXPECTED empty SBus slot (30000000),
  followed by slots 40000000/50000000/20000000. Do not treat any next trap as
  evidence of an initialization failure: let OBP handle it. All print Nothing
  there and lead to the normal SPARCstation 5 / 128 MB banner.
- 100M steps insufficient even for enumeration; 200M reaches RAM test with
  119 MiB remaining. Let original default diagnostics run: 2B budget probe
  completed at 1009623777, no ROM patch or NVRAM change. Network auto-boot
  then reads unimplemented 78400010 -> recovery prompt. Earlier timer warning
  expected 1e remains. No reported RAM diagnostic error.
- First prompt 1009351805; send 1 2 + . CR, reply 3, second prompt 1009623777.
  Final counts 1009614328 instructions / 21 traps / 9428 IRQ / 1 reset;
  3922 bytes, PC ffd1689c / NPC ffd168a0, CPU error false.
  local/probes/esp-ram-test-ss5 holds all metadata/probe/logs/NVRAM export.
- 294 tests, fmt/clippy/four demos pass. User's serial-prompt stopping point is
  reached again after this progress. Next hardware target on resumption is
  Ethernet DMA2 at 78400010 and NCR92C990 at 78c00000, not more ESP probing.
  Actual SCSI targets/DMA and CGsix remain pending. Do not call this clean POST
  or working network boot. No subagents were used.
