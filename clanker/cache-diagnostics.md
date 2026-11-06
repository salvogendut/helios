# Cache diagnostic storage and OBP memory probing

Checked 2026-11-06. Contract: docs/cache.md. src/cache.rs models independent
diagnostic arrays behind Srmmu. Normal memory still bypasses them; do not claim
cache fills/hits, injected-code execution or cache POST from this slice.

- Primary /tmp/helios-ms2.pdf/.txt, revision 1.1 July 1994: chapters 6/7,
  table 55. Visually inspected figures 6.1/7.1 at printed 127/135, PDF125/133.
  Renderings /tmp/helios-ms2-dtag.png and -itag.png. I-tag VA31:14, R13:12;
  D-tag VA31:13, R12; both context11:4, ACC3:1, valid0. Reserved readback zero
  and all-zero reset storage are deterministic choices, not specified RAM reset.
- Both 512 tags. I index13:5, D index12:4; all other bits alias after CPU
  alignment checks. I-data16KiB word only; D-data8KiB byte/half/word/doubleword.
  Atomics remain unsupported. No physical bus access or fault-register effects.
- ASI10..14 invalidates one indexed tag in each array, preserving tag fields
  and data. ACC6/7 global. Page/segment/region match VA and (global OR context);
  context flush user/current only; user flush all user contexts at that index.
  Table55 segment VA13:18 is a typo: V8 appendixI tableI-2 confirms31:18.
- FLUSH updates only I tags, matching VA; includes other contexts as a
  conservative V8-permitted choice. No TLB flush, translation, alignment fault
  or bus access. Coherent ordinary accesses are unchanged.
- Hardware uses D-cache fill paths for some diagnostic loads; that interaction,
  ASI39, actual fill/use/protection, streaming, parity and alias behavior remain
  pending. CPU normal loads/stores never read or fill diagnostic arrays.
- Pinned /tmp/helios-qemu-ldst.c returns zero and ignores diagnostic stores.
  MAME sun4m unfinished; do not import sun4c cache behavior.
- Nine new tests, 166 total; fmt/clippy/all tests/four demos pass. Covers all
  entries, aliases, widths, BE lanes/doublewords, scope/context/ACC matching,
  guest instructions/privilege, rejection atomicity, FLUSH and both resets.
- Original OBP completes both 512-entry tag initialization loops. The library
  probe counts ASI0c/0e stores512 each; ASI0d/0f stores0. Thus this run gives
  no evidence about data diagnostics. PCR04004000 at first fault.
- First trap step20009111, PC700075f8, nPC700075fc, e6a40400 STA viaASI20,
  l0=08000000, l3=02000000: store just outside128MiB. Trap29 vector70000290.
  Inspect physical memory bank decode and unpopulated-bank behavior next;
  do not expand configured RAM, return fabricated values or ignore errors merely
  to get past this probe. The fixed RAM mapping currently reports Unmapped.
- Guest diagnostic154bytes then software reset step20044216. Final attempt
  20044380 atPC0000be7c,nPC0000be74,c0a200c0: ASI06 word store to1fc.
  Completed20044378instructions,1trap,0IRQ,1reset; CPUerrorclear; CLIexit1.
  ASI06 is TLB diagnostics, not a generic cache flush. It remains missing.
- local/probes/cache-ss5/ holds CLI + bounded probe + config. Temp source
  /tmp/helios-cache-probe.rs; takes pre-fault registers at the known step.
  Settings unchanged: original ss5.bin pinned hash,128MiB,defaultNVRAM,noinput,
  100ns per completed step,30Mbudget. No prompt or OS boot.
