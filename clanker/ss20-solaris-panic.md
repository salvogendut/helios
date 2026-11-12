# Solaris 2.6 first-panic follow-up

2026-11-12, Helios `4195f42fe5a8554cd8d9cfb37917cc0138cb038b`, following
[the initial SS-20/Axil review](ss20-axil-disk-cg14-review.md).
All implementation experiments are isolated in `/tmp/helios-panic-review`.
The shared implementation and original disk remain unchanged.

## The supplied PC is a console-output PC

In the original SS-20 2.25R ROM, `ffd42d40` is the Forth `c!` primitive:
`ffd42d48` stores a byte; `ffd42d4c` then pops the next stack value.
The supplied capture in `/tmp/panicprobe.err` stopped while printing
`kmem_getpage`, with character `0x65` in `%l0`. Thus `ffd42d4c` is not the
kernel's panic call or the cause of the allocation failure.

The actual failing check in this Solaris kernel is in `boot_getpages()`:

| Breakpoint | Meaning |
| --- | --- |
| `f005cf44` | Just returned from `va_to_pfn()`; `%o0` is the PFN, `%i3` the VA |
| `f005cf68` | Just returned from `page_numtopp()`; `%o0` is the page pointer |
| `f005cf88` | Calls the error-reporting path for NULL/free page; `%i3` is still the VA |

These addresses were checked against the kernel dumped from the running
guest. Symbols from the user's Solaris 2.6 ISO identified `boot_getpages`,
`va_to_pfn`, `page_numtopp`, `locked_bzero` and `page_add`; runtime instructions
were used for relocated calls. Addresses are specific to this kernel build.

## Confirmed cause: the reference-MMU segment-probe shortcut

Both machines reach the first check with VA **`f5940000`**, a 256 KiB boundary.
The guest's SuperSPARC probe routine returns **`02cd02ce`**, which
`va_to_pfn()` shifts to **PFN `0002cd02`**. That would be PA `2cd02000`, outside
the installed 128 MiB. `page_numtopp()` returns NULL, and Solaris panics.

Independent descriptor walks of the same paused machines show valid RAM:

| Machine/configuration | Actual page PTE | Actual PA | Correct PFN |
| --- | --- | --- | --- |
| SS-20, serial, display removed | `007b78be` | `07b78000` | `00007b78` |
| Axil 320, serial, CGsix | `007c2dbe` | `07c2d000` | `00007c2d` |

The bug is the `reference_mmu && kind == 1` branch at the beginning of
`Srmmu::probe()` in [mmu.rs](../src/mmu.rs). `next_table(context_word, va, 0)`
already returns the address of the indexed L1 entry. The shortcut adds the
L2 index to that address instead of following the L1 descriptor to its L2
table. For this VA it reads beyond the 1 KiB L1 table and interprets unrelated
memory as a PTE.

**Fix:** remove that shortcut and use the normal descriptor walk already
present below it. Do not compensate by changing RAM size, adding the bogus
PFN to Solaris's memory list, or suppressing the NULL-page check.

The new regression constructs a context table, L1/L2/L3 descriptors, and
PTE-shaped unrelated data beyond the L1 table. A segment probe must return
the L2 descriptor, not the unrelated word. It fails on `4195f42` with
actual `02cd02ce`, expected `00000401`, and passes with the deletion. It also
checks page/full probes and the segment probe after a TLB fill.

Removing only this shortcut gets both machines past the panic. Each ran
800 million steps without it recurring, but then waited for MXCC completion.

## Next confirmed blocker: MXCC RDY never becomes one

After the probe fix, both guests spin in `locked_bzero()`:

```text
f0046434  ldda [%l0] ASI 2, %l4
f0046438  cmp %l4, 0
f004643c  bge f0046434
```

`%l0` points to the stream destination register. The transferred data has been
written, but Helios returns the saved request value, whose high word is
`00000010`. The sign bit stays zero, so this completion wait cannot terminate.

The [Sun-4M System Architecture, revision 50](https://www.netbsd.org/~mrg/Sun4M_SystemArchitecture_edited2.pdf),
section B.III.5.4.1.1, printed page 117, specifies RDY at bit 63, cacheability
at bit 36 and the block address at bits 35:5. RDY is read-only and indicates
completed stream transfers. Reserved bits read zero. The pinned QEMU code
does not implement source/destination register reads, so it cannot supply this
missing completion behavior.

**Fix:** clear RDY when accepting a transfer and set it after the synchronous
32-byte transfer succeeds. Preserve the full 36-bit block address and C bit;
ignore written RDY/reserved/byte-offset bits. Source and destination both need
completion status. Existing error-register and partial-failure semantics still
need a separate hardware review; this patch does not complete all MXCC behavior.

Regression coverage checks completion after both reads and writes, transfers
above 4 GiB using sparse bus mappings, reserved-bit masking, and inability to
fake successful completion by writing RDY on a failed request. Original code
fails these tests; the patch passes.

## Result after both fixes, and the next breakpoint

Both machines pass the allocation failure and the MXCC wait, then reach a
different failure during `post_startup()` / `page_list_color()`:

```text
BAD TRAP: type=7
pc=f0076508
panic: Memory address alignment
```

This is `page_add()+0x24`, `ld [%g1+0x14], %g1`. `%g1` is `40400ce2`, causing
the misaligned effective address `40400cf6`. That value resembles a PSR, not a
page pointer. The first alignment trap occurs at step 325,410,677 on SS-20
with native CG14 and 306,882,586 on Axil with CGsix. Their serial tracebacks
show callers `f00632c0` and `f005e76c`; both subsequently reboot.

Next investigation: break at `f0076504` before loading the list head. Record
`%o0`, `%o1`, the physical mapping and value of `[%o0]`, and the subsequent
interrupt/trap transitions. If the list head already contains `40400ce2`,
watch writes to that physical word back to its first corruption. If memory is
valid but `%g1` changes, inspect register preservation across the intervening
trap. Do not weaken alignment checking. This failure is not yet diagnosed,
and neither machine has completed Solaris boot in these experiments.

## Handoff artifacts and checks

[local/review-2026-11-12-panic](../local/review-2026-11-12-panic/) contains:

- `01-reference-probe.patch`: shortcut deletion and regression.
- `02-mxcc-completion.patch`: completion/address fields and regressions;
  apply after patch 01.
- The headless harness, baseline/fixed serial and trap logs, page mappings,
  failure-before/fix-after test evidence and check logs.

The reproductions use original OBP, 128 MiB, a keyboard, serial NVRAM and
separate writable copies of the user's Solaris 2.6 disk. The first SS-20
panic comparison removed the display; the final combined run used native
CG14. Axil used CGsix. Frontend UI and host NAT were not exercised.

The isolated combined fix passes `cargo fmt --all --check`, Clippy with
`--locked --offline --all-targets -- -D warnings`, all **430 tests**, and
`demo`, `trap-demo`, `serial-demo`, `timer-demo`. The full test run allowed
local UDP sockets for the existing networking tests. Original firmware and
disk contents, extracted guest executables and memory dumps remain local and
are not included in tracked files or the patch files.

## Applied implementation

Both supplied patches were applied on top of `4195f42` (with the earlier
CPU-identity/CG14 work): the faulty kind-1 reference segment-probe shortcut is
removed, and MXCC source/destination completion now masks to the 36-bit block
address, clears RDY while accepting a stream, sets it after a successful
32-byte transfer, and cannot be forced by software. Regressions cover the
L1-table boundary probe and MXCC completion/address behavior.

The combined boot now passes `kmem_getpages` and the MXCC wait, then reproduces
the handoff's next failure: `BAD TRAP: type=7`, `panic: Memory address
alignment`, at `page_add()+0x24` (`pc=f0076508`). The next watchpoint remains
the one documented above.
