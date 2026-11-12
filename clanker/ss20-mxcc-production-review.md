# MXCC production follow-up at 652d5df

2026-11-12. Reviewed `652d5dfeab5fe83bba278b83c9516370d2828e81`
after the reported SS-20/Axil login and cloned-disk migration milestone.
The control-register alias now shares state as requested. The login result
and 432-test validation were reported by the implementing agent; this review
did not repeat a guest boot or the full suite.

Four new synthetic regressions were run in an isolated archive at
`/tmp/helios-mxcc-production-review-652d5df`. All four fail. They require
neither firmware nor a disk. Shared implementation files remain unchanged.

## First priority: a stream can panic the host

`Srmmu::mxcc_write()` holds `self.mxcc.borrow_mut()` while calling
`MxccState::write_u64()`. The latter performs bus accesses before releasing
the borrow. The physical bus aliases route back to the same `RefCell`.

Two concrete reproductions:

```text
ASI 2 stream-source write:
    register 01c00100, value 0000000f:f8c00a00
    -> read_doubleword(PA ff8c00a00)
    -> panic at SystemBus line 1553: RefCell already mutably borrowed

Physical stream-destination write:
    register PA ff8c00200, value 0000000f:f8c00a00
    -> write_doubleword(PA ff8c00a00)
    -> panic at SystemBus line 1631: RefCell already borrowed
```

These cases deliberately use the module's own register window as the transfer
target. Whether a particular transfer is supported hardware behavior needs
separate modeling, but a guest request must not unwind the emulator host.

Separate transfer preparation, bus I/O, and completion. Release the state
borrow before invoking the bus and reacquire it when updating completion.
Also define how recursive/unsupported register-space transfers are rejected,
so simply releasing the borrow does not replace a borrow panic with recursive
transfers. Preserve the existing RAM transfer path and RDY/error behavior.
An explicit unsupported/error result is acceptable until the hardware case
is implemented. The regressions permit such a result and reject host panics.

## Next: port identity is writable and its architected alias is missing

`MxccState::write_u64()` currently implements the port-register write as:

```text
registers[7] = value & !0x1f
```

A write of all ones changes the returned identity from `[0, 0]` to
`[ffffffff, ffffffe0]`. MID, device, revision and vendor fields are hardware
identity, not guest-programmable storage. The existing regression in
`tests/machines.rs` that writes `1234` and expects `1220` validates the current
implementation rather than the register definition.

The decoder only recognizes the `ff8c00000..ff8c00fff` register page.
`bus.read32(0xff8fffffc)` fails, so the architected MBus identification
address is still missing. The observed `ff8c00f04` page alias does not replace
it.

Give the configured module a documented fixed identity, preserve it across
reset, ignore writes to the read-only fields, mask reserved bits, and expose
the same value through ASI 2, the page alias and `0xff8fffffc`. Keep absent
module behavior separate. Assert the identity fields and write immunity in
tests instead of expecting writable identity storage.

The [Sun-4M System Architecture, rev. 50](https://temlib.org/pub/SparcStation/Hardware/Sun4M_SystemArchitecture_edited2.pdf),
B.III.5.4.7 (printed page 120), marks all port fields read-only: MID[27:24],
MDEV[15:8], MREV[7:4], MVEND[3:0]. It gives device 1, revision 0 and TI
vendor 4 for that documented MXCC; a different revision needs its own
source. B.III.8 (page 124) gives physical port address `0xFFnFFFFFC`.

## Artifacts and next verification

[local/review-2026-11-12-mxcc-production](../local/review-2026-11-12-mxcc-production/)
contains `mxcc_review.rs`, `review.log`, and reproduction instructions.
Copy the test file into `tests/` in an isolated checkout and run:

```sh
cargo test --locked --offline --test mxcc_review
```

After these corrections, rerun the required suite and retain the original-OBP
login result on each machine. Then test disk write/read checksums across a
clean shutdown and reboot on disposable clones, followed by native CG14/Xsun
display and input validation. Reaching serial login does not establish SX
acceleration or sustained disk correctness.

## Applied corrections

`MxccState` is now bus-owned and shared by ASI 2 and the physical module
window. Stream setup/transfer/completion no longer holds a `RefCell` borrow
across bus access; register-window targets are rejected explicitly instead of
panicking. Port identity is fixed and read-only at `0x08000104`, with page
alias `0xff8c00f04` and architected alias `0xff8fffffc`.

The migrated Axil clone's guest file survived `sync`/`halt` and reboot:
`cat /helios-disk-test` returned `HELIOS-DISK-TEST`, and `cksum` returned
`3034423527      17      /helios-disk-test`. Native CG14/Xsun display and
focused keyboard input remain the next validation milestone.
