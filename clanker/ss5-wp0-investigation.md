# SS-5 WP=0 replacement: evidence and reproducible probe

Status: investigation only. No replacement-semantics change and no new
page-walk cache. The current model still uses WP as an inclusive replacement
wrap point; WP=0 therefore leaves only TLB slot zero replaceable.

## Manual evidence

Primary source: *microSPARC-II User's Manual*, Rev. 1.1, July 1994,
<https://temlib.org/pub/SparcStation/Hardware/Sparc32/microSPARC-II-UsersManual2.pdf>.

- Section 5.2.1 (printed p. 65) defines a six-bit TLB replacement counter in
  TRCR, incremented each CPU clock and used on a miss to select the entry to
  replace. It says the programmable six-bit field defines the counter
  "roll-over" point and "effectively locks down entries beyond the roll-over
  point." Figure 5.1 (p. 66) labels the replaceable PTE range, the wrap point
  and locked TLB entries.
- Section 5.6.6 (printed pp. 88–90) defines the TRCR fields. WP is the wrap
  point; the I/O-lock note says previously locked entries are kept above entry
  48 and WP is set to `47-(63-old WP setting)`, confirming that entries above
  WP are the locked region.
- Section 5.10 (printed pp. 113–115) says a leaf PTE found by a table walk is
  stored in the TLB "indexed by the TLBRC" and used for the translation.

The manual gives no WP=0 special case that would disable wrapping or restore a
64-entry replacement range. Its ordinary reading is therefore: WP=0 means slot
zero is the only replaceable slot; slots 1–63 are locked.

This establishes the **replacement range** interpretation only. The manual's
counter advances every CPU clock, while Helios currently advances it on fills;
that is a separate functional timing simplification and is not used to justify
changing WP behavior.

## OBP/Solaris trace

A current Helios diskless probe using the original SS-5 OBP and NVRAM writes
TRCR twice before boot (`local/ss5-wp0-2026-11-16/probe.rs`; raw output in
`trace.raw.log`). The probe reads PC/instruction from `Step::Executed`; it does
not perform an extra instruction fetch through the live MMU before stepping:

```text
initial readback 0x1201f80   WP=63; SBSP/MEMSP read-only speed fields set
step 1668443  PC 0xbe9c    instruction 0xc0a20080 -> TRCR 0x1200000
step 12822574 PC 0xffd27558 instruction 0xc8a40080 -> TRCR 0x1300000 readback
effective writable fields: VP=1, WP=0, TC=0, TRC=0
```

The first write clears the writable TRCR fields. The second sets VP and
preserves the read-only speed fields; it does **not** restore WP.

Solaris evidence must be read from its matching measurement windows:

- `local/review-performance-2026-11-14/mmu-cache-check/phase-deltas.json`,
  SS-5 `ss5-counters` / `serial_idle_scanout`, **204.8M steps**:
  111,976,153 total walks, 112,008,165 replacements, 204,797,643 instruction
  requests, 45,337,026 data requests and 137,931,212 fast hits.
- `local/performance-2026-11-13/ss5-trcr.log`, SS-5 `serial_idle`,
  **40.96M steps**: 8,207,187 data walks and zero data fast hits, with idle
  TRCR `0x100000`.

These are separate idle windows and must not be combined. The retained
`ss5-trcr.serial` shows normal Solaris login and clean halt. This is consistent
with the one-slot range, but it is also compatible with an OBP/Solaris choice or
firmware bug; the traces alone do not prove the silicon interpretation.

## Reproducible regression

`tests/tlb_diagnostics.rs::wp_zero_limits_replacement_to_slot_zero` sets
WP=0, fills four different contexts, and asserts every fill lands in slot zero
while slots 1–63 remain invalid. This test captures the current functional
model and passed with the probe above. It is a regression fixture, not an
independent hardware proof. The diagnostics suite is **7/7** passing.

## Decision

Do not change WP semantics and do not add a page-walk cache from this evidence.
The manual supports the current range behavior; the only unresolved question is
whether real OBP intentionally accepts a one-slot TLB or relies on a WP=0
special case absent from the manual. Resolve that with independent hardware
trace or OBP source/errata before changing the model.
