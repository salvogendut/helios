# Use V8 trap-state rules when reference implementations differ

Confirmed 2026-11-06 against the V8 manual, chapters 4/7, B.26–B.29 and C.8.
Reference revisions are in docs/references.md.

- WRTBR changes TBA only, preserving TT. The inspected MAME code clears the
  low twelve bits; do not copy that behavior into Helios. Regression:
  tbr_write_changes_only_base_and_rd_reads_last_trap_type.
- Eight windows require eight WIM bits. MAME's inspected WRWIM path masks with
  0x7f; MB86904 requires 0xff. QEMU derives this mask from its window count.
- Trap entry decrements CWP without consulting WIM. SAVE/RESTORE consult WIM
  before their own rotation; their fault then performs trap rotation.
- For RETT, privilege outranks ET legality, which outranks underflow, which
  outranks alignment. With ET=0, RETT failure changes only TBR.TT and the
  error-mode latch, preserving the normal instruction effects. QEMU's helper
  sets ET before its WIM check; use the manual's check-before-commit semantics.
- Other synchronous faults with ET=0 preserve TT and latch error mode. Repeated
  steps must not fetch or execute. Unsupported emulator behavior must not latch
  error mode or masquerade as a hardware illegal-instruction trap.
- Immediate state writes are an allowed 0-of-0..3 delay choice. Test fixtures
  insert three NOPs before dependent operations when practical.

The bus currently reports physical failures as TT=0x21/0x29, alignment as 0x07.
SRMMU work must add separate instruction/data access-exception causes 0x01/0x09.
No delayed store-error or external-interrupt delivery is implemented yet.
