# microSPARC-II memory findings

Use docs/memory.md and docs/references.md for the human-facing contract.
Primary source: Sun microSPARC-II manual rev 1.1 (July 1994), mirrored by
Temlib. Public reference downloads were read from /tmp/helios-ms2.{pdf,txt},
/tmp/helios-v8.txt and /tmp/helios-qemu-{mmu,ldst,sun4m}.c; these are temporary
research files, not repository/build inputs.

- Table 40: BM redirects ASI 8/9 to 0x70000000 | VA[27:0], even with EN set.
  QEMU's generic boot mask/EN ordering is different.
- PCR.NF does not suppress ASI 8/9 faults. Do not copy QEMU's ET=0 suppression;
  real MMU permission/translation failures must still reach CPU error mode.
- CTPR is limited to physical bits 27:10; subsequent PTPs contain PA[30:8].
  Do not truncate all table pointers to the context table's 256 MiB range.
- SFSR and SFAR both have destructive read aliases (0x300/0x400). Diagnostics
  at 0x1300/0x1400 do not clear them. QEMU differs for SFAR.
- Page/segment/region probes must not return an earlier leaf; table 18 gives
  exact-level results. Entire probe sets R, with no permission check.
- Root PTE lookup is excluded on printed p75, although table 18 permits probe
  readout. Normal root PTE translation currently stops explicitly.
- PCR diagram and reserved prose conflict (notably bit 21 and bit 7). Current
  mask/unsupported controls are documented; do not claim full diagnostics.
- Public Srmmu calls assume the caller already checked CPU privilege. Cpu does
  so for alternate instructions, before destructive register reads.

Validation added: tests/mmu.rs (ACC matrix, walks, faults, contexts, probes),
tests/memory_cpu.rs (trap recovery, ASI decoding and reset), and CLI boot test.
Original sum/trap demos remain required. No firmware or QEMU differential run.
