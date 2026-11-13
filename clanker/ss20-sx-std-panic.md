# SS-20 olwm drag: paired SX instructions cause the first panic

Independent review of the 466-test outline milestone, 2026-11-13. Production
code is unchanged by this review. Evidence and isolated regressions are in
`local/review-2026-11-13-sx-std/`.

## The ESP stop follows an earlier SX bus fault

The complete serial capture from the supplied outline run contains:

```text
Xsun: Data fault
user write fault at addr=0xef7a0020, pme=0xf8000166
MMU sfsr=96: Bus Access Error on user data store at level 0
pid=283, pc=0xee66f104, ... context=24
o0-o7: bfe00509, bfe00d09, ...
panic: trap: unexpected MMU trap
...
dumping to vp f5bbb674, offset 264720
```

Only after this does the host stop at PC `0xf5ac6a94`, an ESP command write
at physical `0xef080000c`, with `Unsupported("ESP stacked interrupt")`.
That stop is in panic-dump activity. Fix the original SX access before using
the ESP stop to guide the next desktop experiment.

The supplied held/released PPMs are byte-identical. TERM-A remains at
`+100+100`; the changed pixels show its outline and pointer, not a completed
20-pixel window move. Passing every `Result` check in `pump()` was insufficient:
`Machine::step()` returns **`Ok(Step::TrapTaken { ... })`** for synchronous
guest traps, and the harness continued through the resulting panic.

## Root cause: treating an IQ doubleword as adjacent register writes

The original DDX maps PC `0xee66f104` to `sl_rect_fill_32_rop`, ELF `0x2f104`:

```text
d03e2020    std %o0,[%i0+0x20]
%i0 = 0xef7a0000
%o0 = 0xbfe00509
%o1 = 0xbfe00d09
```

This is one 64-bit write to the user instruction-queue port at physical
`0xf80001020`. Both words are SX ORS instructions: the first fills R10..R25
from R9 and the second fills R26..R41 from R9. In the captured guest R9 is
zero; the regressions seed it with a distinct value to detect missing words.

`SystemBus::write_doubleword` currently invokes `card.write(address, word0)`
and then `card.write(address + 4, word1)`. Consequently:

1. The first instruction executes at SX user offset `0x1020`.
2. The second is treated as a standalone register write to user offset
   `0x1024`, which `Sx::write_sx` rejects as `ReadOnly`.
3. The MMU records `SFSR=0x96`, `SFAR=0xef7a0020`. CPU trap `0x29` is delivered
   for `FaultKind::DataAccess`, and Solaris panics.

The isolated translated fixture reproduces those **exact SFSR/SFAR values**
with the original context and PTE. A two-instruction CPU fixture executes
the original `STD` at the original PC and produces:

```text
Ok(TrapTaken {
    fault: ... DataAccess(... address: 0xf80001024, kind: ReadOnly),
    trap_type: 0x29, vector: 0x290
})
```

There are two additional consequences: supervisor IQ doublewords silently
discard the second instruction, and failed pairs leave the first instruction's
register mutations committed. The latter breaks the project's retry/fault
atomicity contract for unsupported operations.

### Required implementation and evidence

Add a device operation that receives the complete doubleword. For an IQ
transaction, execute both words through the **same queue port**, in bus word
order, with the second seeing the first's result. Validate/stage the pair and
commit only on success. Stage the small execution state rather than copying
the entire 8 MiB framebuffer for every command pair. Keep ordinary VRAM and
queued-register doublewords distinct from IQ doublewords.

Do not make standalone offset `0x24` a generic writable IQ alias as a shortcut.
Sun's register definition calls that word reserved; the missing behavior is
the width-aware queue transaction. Preserve user/supervisor access rules.

Sun's [Pixel Processing in a Memory Controller](https://www.sunhelp.org/history/pdf/smc.pdf),
PDF page 8 (zero-based page 7), explicitly describes sending one or two
register-register instructions over the 64-bit MBus in one transaction to
the single IQ address. The local original PDF and extracted text are in
`local/review-2026-11-13-sx-plot/`. The Sun `sxreg.h` 1.5 structure there has
`s_iq` at `0x20` and reserved `s_pad1` at `0x24`. The original Xsun DDX's
`STD` independently confirms actual use of this transaction.

`tests/queue_pair.rs` in this review directory has seven fixtures: user and
supervisor paired submissions, ordering with a dependent second instruction,
rollback on a rejected second instruction, standalone reserved-word behavior,
the original translated user mapping, and the original CPU `STD`. Six fail
against production; the reserved-word control passes.

## Two further outline regressions

These are separate from the first panic, and can be addressed alongside the
queue fix before another expensive guest run.

1. **Partial packed loads disagree with packed stores about byte lanes.**
   The supplied trace includes `0x80d7d500`, `0x9122952a`, `0x808fd500` at
   PA `0xff07cbd0`: a two-byte X-plane load/invert/store for the vertical
   outline at pixels `(592,443)` and `(593,443)`. The new load right-justifies
   two bytes in a register, while the store consumes its high bytes. With
   nonuniform X bytes `01 89`, the sequence should write `fe 76`, preserving
   B/G/R and all other pixels; it currently writes `ff ff`. Keep packed-tail
   lane placement consistent with the high-byte-first store path. This
   fixture constrains the observed aligned two-byte case without asserting
   unspecified padding or all unaligned cases.
2. **ROP_M ignores the mask-selected upper truth table.** The implementation
   uses only `rop_control & 0xf`. Captured control `0x55` has equal nibbles,
   which hides this defect. An independent `0x3c` fixture with all R2 bits
   set must return `~A`; it currently returns `A`. With R2 clear it must
   return `A`. Sun's paper describes the mask-controlled ROP forms, and
   NetBSD's `cg14_invert()` documents `0x3c` as `~src a / src a` and enables
   inversion using an all-ones mask. This is a synthetic coverage extension,
   not a claim that `0x3c` caused the captured panic.

Both fixtures in `tests/outline_followups.rs` fail against production. The
source for the second oracle is the local NetBSD
`sys/arch/sparc/dev/cgfourteen.c`, `cg14_invert()`, alongside the independently
documented `SX_ROP_M` encoding in `sxreg.h`.

## Validation and next experiment

All 32 existing focused tests (`mouse`, `sx`, `sx_loads`, `packed_outline`)
pass independently. The new review suite has eight failures and one passing
control. It is isolated under ignored `local/`; no failing tracked tests or
production edits were introduced. No new Solaris boot was needed: the minimal
CPU/MMU reproduction matches the saved guest failure.

```sh
cargo test --locked --offline \
  --manifest-path local/review-2026-11-13-sx-std/Cargo.toml \
  --test queue_pair --test outline_followups -- --test-threads=1
```

The review also supplies a compiled harness that recognizes physical graphics
bus faults returned as `Step::TrapTaken`, records the first PC/instruction and
SFSR/SFAR, and stops before panic-dump activity. Normal window and demand-page
traps are not rejected indiscriminately. It saves its diagnostics under this
review directory and requires an explicit private disk clone and output prefix.
This updated harness was built, but has not been run against Solaris yet.

After promoting the regressions and fixing the three device gaps, rerun on a
fresh clone, require the original `STD` to retire, then verify frame geometry
and release/redraw before accepting the larger move or 1/2/3-pixel offsets.
Continue resize/uncover/iconify and Heliacal input tests from that checkpoint.
The ESP panic-dump limitation remains a separate issue if it still needs work.

## Implementation status

The IQ doubleword transaction is now implemented as one width-aware SX
operation: both words execute through the same user or supervisor queue port,
in bus order, with the second seeing the first result. Register/Rop/error
state is rolled back if either instruction is rejected; offset `0x1024` is
not made a writable alias. Packed-load tails now keep their bytes in the high
lanes used by packed stores, and ROP_M selects the upper/lower truth-table
nibble per bit from R2. The tracked queue-pair and outline-follow-up tests
pass, including the original STD, dependent ordering, rollback, partial
two-byte outline, and `0x3c` ROP cases.

The corrected diagnostic harness now recognizes `Step::TrapTaken` and stops on
graphics IQ bus faults instead of continuing into panic dumping.

## Applied fixes and next fault

Paired IQ doublewords, packed-load tail lane placement, and R2-selected ROP
truth-table nibbles are implemented. The tracked queue-pair and outline
follow-up fixtures pass. A calibrated guest run now completes the large
title-bar drag: held and released captures differ across the TERM-A region,
and the run reaches the first one-pixel move before stopping at
`0xc8e28518`, an SX funnel/shift form (operation 7) not yet implemented.
This is the next SX blocker; resize/uncover/iconify and Heliacal input tests
remain open. Captures and trace are saved under
`local/review-2026-11-13-sx-iq/`.

## Funnel and copy alignment applied

The next SX instruction, funnel-left immediate 24 (`0xc8e28518`), now executes
both opcodes 6/7 with an adjacent-word source snapshot; the driver-derived
unaligned operation-1 packed store skips its prefix lanes. Eight regressions
are promoted as `tests/funnel.rs` and `tests/packed_copy.rs`. The calibrated
guest run now reports TERM-A moving by exactly 20, 1, 2 and 3 pixels on
successive releases. See `ss20-sx-funnel-review.md` and
`local/review-2026-11-13-sx-funnel/accepted-run3/`.
