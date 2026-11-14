# SMP slice 1 review

2026-11-14. Reviewed the implementation on `feat/sun4m-smp`, after
planning/cleanup commit `495523d`. The original review found two host-failure
paths; both are corrected and independently revalidated. **Slice 1 is accepted
for a feature-branch commit.** The user subsequently approved merging slices 1
and 2A as an incremental foundation checkpoint after the final 2A validation.
Scheduling and two-CPU boot remain unimplemented; issue #2 remains open.

## Accepted corrections

The six independent boundary fixtures now pass, including legal word-sized
RAM streams and the formerly aborting cyclic transfer. Address decoding uses
immutable installed identity, ASI word writes release live borrows before
bus access, and stream checks reject the full span across all four documented
module windows/ports before side effects. The port-width/absence policy is
explicitly documented as the functional subset, with tracked regressions.
Independent MMU contexts and reset isolation are tested on both MBus profiles.

Fresh independent checks on the corrected tree passed:

- 533 unique normal core tests and 535 with `perf-counters`; raw logs include
  one extra child-process execution of the cycle test in each configuration.
- Formatting, Clippy with `-D warnings` in both configurations, and all demos.
- All six independent boundary fixtures.
- Heliacal's 29 tests and a release rebuild against the corrected core.

The implementer's final `boot-width/` logs and serial transcripts were checked:
SS-5, SS-20 and Axil reach the root shell, print the acceptance marker, sync and
halt cleanly; Axil retains checksum `3034423527 17 /helios-disk-test`. These
boots were not repeated by this review. Independent results are recorded as
`accepted-*.log` under `local/review-smp-slice1-2026-11-14/`.

See [smp-slice1.md](smp-slice1.md) for the implementation and remaining limits.
The original findings and reproductions below describe the pre-correction
tree, not outstanding blockers.

## Original blocking findings (resolved)

### 1. Word-sized ASI 2 RAM streams now panic in a single-CPU machine

`src/mmu.rs`'s ASI 2 word-store path holds a mutable `RefCell` borrow while
calling `MxccState::write_word`. The new `SystemBus::mxcc_at` borrows each
installed MXCC merely to decode a physical address, including ordinary RAM
addresses. Consequently the first bus access of a legal RAM stream attempts
to borrow the already mutably borrowed MXCC and panics at
`src/system_bus.rs:533`.

Independent reproductions through the normal one-CPU SS-20 constructor:

- Set RAM at `0x100`; word-store `0x100` to ASI 2 address `0x01c00104`.
  Loading the 32-byte stream panics instead of filling the MXCC data bank.
- Initialize the MXCC data bank; word-store `0x200` to ASI 2 address
  `0x01c00204`. Writing the stream to RAM panics instead of completing.

Both fixtures pass against the committed pre-slice core exported from Git,
and both fail against the proposed core. This is a new UP regression, not
a missing future SMP feature.

Decode addresses from immutable module metadata without borrowing live MXCC
state on every bus access. Also route ASI word transfers through the same
borrow-free snapshot/transfer/publication discipline as doubleword transfers.
Test both CPU modules, RAM source/destination transfers and ordinary transfer
failures. Retain the existing address masks, completion/RDY and ordinary
failure semantics; do not turn every error into an unconditional rollback.

### 2. Stream recursion protection covers only the initiating module

`MxccState::is_register_window` checks the initiating MID. With two modules
installed, streaming into the other module's stream register executes nested
transfers. A finite reproduction writes RAM and changes both modules instead
of rejecting the unsupported target before mutation:

1. Set module A's first data doubleword to `[0, 0x200]`.
2. Set module B's first data doubleword to `[0x11223344, 0x55667788]`.
3. Write `[0xf, 0xfac00200]` to A's destination register at `0xff8c00200`.

The current code returns `Ok(())`, changes both module states, and writes B's
data to RAM `0x200`. This is a nested register operation, not a RAM-only stream.

A cyclic variant proves a host abort through the production physical path:

- A's first data doubleword is `[0xf, 0xf8c00200]`.
- B's first data doubleword is `[0xf, 0xfac00200]`.
- Start A with destination `[0xf, 0xfac00200]`.

A writes B's destination register, B writes A's destination register, and
the recursion overflows the host stack. The review ran this in a separate
test process with a five-second bound; it terminated with `SIGABRT` and
`fatal runtime error: stack overflow`.

Prevalidate stream targets against all relevant module register windows,
including the full transfer span and port aliases, before changing either
module or the bus. For unsupported control-space stream transactions, return
an explicit error without partial effects. A recursion-depth limit alone
would still permit earlier nested side effects. Cover physical and ASI
entry points, both directions, and both source and destination commands.

## Port coverage and documentation

Two additional policy fixtures fail. These are not being reported as newly
introduced UP crashes:

- The documentation says unknown/unpopulated module windows read zero, but
  absent architected ports `0xff9fffffc`, `0xffafffffc` and `0xffbfffffc` are
  outside the zero windows and currently return `Unmapped`. The existing
  tests check only selected control-window addresses. Decide and document
  the supported absence policy from hardware/firmware evidence, then cover
  both control and architected port accesses. Do not blanket-zero unrelated
  physical space just to satisfy a test's documentation-derived expectation.
- Populated port decoding recognizes only the exact word address. Although
  the bus exposes byte/halfword reads, byte offsets +1/+2/+3 and halfword +2
  fall through to `Unmapped`; the first byte and full word work. This behavior
  already exists for MID 8 in the committed core and was copied to MID A.
  Either support consistent lanes where supported, or explicitly reject
  unsupported widths without partial accesses; document and test the chosen
  behavior. These fixtures express the advertised alias/width consistency,
  not an independently established requirement for narrow hardware accesses.

Extend tracked slice tests beyond one global register and one diagnostic word:
exercise distinct MMU contexts/translations across a shared bus, reset one
module without clearing the other's translations, and verify local ASI and
remote physical access from both modules. Keep expected values independent
of the production address decoder. Check Axil as well as SS-20 defaults.

## Validation and reproduction

Independently reran on the proposed tree:

- Formatting and Clippy with `-D warnings`, normal and `perf-counters` builds.
- 517 normal tests and 519 counter-enabled tests.
- All four demos, and the focused existing MXCC/topology tests.
- The independent fixtures reproduce the failures above despite those passes.

The supplied SS-5, SS-20 and Axil transcripts contain the root-shell
`uname -a`, `HELIOS-BOOT-ACCEPTED` and clean filesystem-sync/halt evidence.
Axil also reports `3034423527 17 /helios-disk-test`. These logs were inspected;
the three guest boots and Heliacal tests were not independently rerun in this
review. Rebuild/revalidate after correcting production behavior.

Independent fixture source and logs are in the ignored directory
`local/review-smp-slice1-2026-11-14/`:

- `boundary_review.rs`: RAM-stream, cross-module recursion/effects and port
  policy cases. The cyclic case isolates the child process deliberately.
- `baseline-check/up_streams.rs`, `baseline-results.log`: the two RAM-stream
  fixtures pass against the committed core.
- `boundary-results.log`, `cycle-results.log`: proposed-core failures.
- `fmt.log`, `clippy*.log`, `tests*.log` and four demo logs: fresh checks.

Run the independent fixture with the configured Rust toolchain and
`CARGO_HOME=/tmp/helios-cargo`:

```sh
ulimit -c 0
cargo test --locked --offline \
  --manifest-path local/review-smp-slice1-2026-11-14/Cargo.toml -- --nocapture
```

The requested fixtures have been promoted and the corrections accepted above.
Scheduling, arbitration, IPI/timer banks and two-CPU guest startup remain the
following slices.

## Next assignment: slice 2A, interrupt and timer banks

Split the board-fabric slice into smaller reviews. First implement processor
interrupt/timer banks for hardware processor numbers 0 and 2, with absent
banks 1 and 3 handled by the documented policy. Default machines remain UP.

Implement Current Interrupt Target routing for shared device sources, directed
software interrupts, independent timer pending/acknowledgement, and hard
level-15 broadcast latches with per-processor acknowledgement. Resolve mask-all
and software-interrupt semantics from the primary controller documentation;
retain profile distinctions and source citations. A dense storage index must
never substitute for the hardware bank number.

Advance all installed timer banks from one elapsed device-time input; mode
and run state remain local. Add focused tests for directed set/clear, CIT
changes with an asserted source, absent targets, masking/pending independence,
broadcast acknowledgement, timer mode bits 0/2 and acknowledgement isolation.
Keep the existing CPU-0-facing APIs compatible or update all their callers.

Stop after synthetic bank/routing tests and the existing UP regression checks
for review. The scheduler, arbitration-driven CPU startup, standalone-IOMMU
ownership refactor, two-CPU firmware boot and frontend CPU-count settings are
subsequent work. Do not advertise SMP boot from this board-device milestone.
