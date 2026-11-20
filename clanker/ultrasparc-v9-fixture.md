# Headless UltraSPARC V9 interpreter fixture

Status: dedicated `feat/ultrasparc-v9-fixture` branch from Helios main.

`src/v9.rs` is a small, safe 64-bit V9 execution fixture kept separate from the
V8 `Cpu`/`Machine` path. It implements only the operations in the fixture:
SETHI, OR/ADD/ADDCC, SLLX/SRLX, RD `%ccr`, STX/LDX, and integer Bicc with
delay-slot/annul handling. Registers are 64-bit; ICC and XCC are exposed in
the V9 CCR layout.

`tests/v9_fixture.rs` runs:

- construct `0x00000000ffffffff`,
- `addcc` one, producing `0x0000000100000000` with ICC.Z set, ICC.C set,
  XCC.Z clear (CCR `0x05`),
- STX/LDX round trip,
- a taken Bicc with a visible delay-slot add.

The fixture was compared with QEMU 10.2.2 `qemu-sparc64`, user-mode:

| State | QEMU / Helios fixture |
| --- | --- |
| g1 | `0x00000000ffffffff` |
| g2 / g4 | `0x0000000100000000` |
| g3 | `0x0000000000500000` |
| g5 | `7` |
| g6 / CCR | `5` |
| PC / nPC at loop | `0x400040` / `0x400044` |

QEMU output bytes and the loop PC/nPC were captured from the same ELF; the
fixture assertions use those independently obtained values. Local evidence is
under `local/v9-ultrasparc-2026-11-20/`.

## Review corrections

The fixture now snapshots ADDCC operands before writeback, computes XCC signed
overflow with widened arithmetic, decodes the shift X bit and implements the
32-bit SLL/SRL/SRA forms, and validates fetch/LDX/STX alignment and full address
ranges with checked errors that leave architectural state unchanged. The
independent edge suite is promoted as `tests/v9_edges.rs`; the original branch
report remains strengthened by its skipped-instruction/annul case. The QEMU
oracle recipe and assembly are in `tools/v9-fixture/`.

Final validation: 826 test executions, Clippy `-D warnings`, formatting and
all four demos pass.


## SLL width correction

SPARC V9 A.49 specifies that non-X SLL shifts the full-width operand with a
five-bit count. The fixture now preserves `0x0000000100000000` for a zero or
effective-zero count and can move bit 31 to bit 32. QEMU 10.2.2 clears the high
word in these cases; the original 48-byte QEMU programs are retained as
comparison fixtures, while the SLL regressions follow the architecture manual
and the UltraViolet implementation. Right shifts and the count mask are
unchanged.
