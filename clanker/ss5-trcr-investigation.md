# SS-5 TRCR replacement and page-walk investigation

2026-11-14. This is an investigation note; it does **not** change replacement
semantics. The diagnostic-cache optimization is a separate change.

## Observed state

The instrumented SS-5 second serial-idle window (204.8M steps) added:

```text
instruction requests    204,797,643
data requests            45,337,026
fast hits               137,931,212
architectural hits          227,304
page-table walks        111,976,153
replacements            112,008,165
flushes                       3,362
```

Unlike SS-20, SS-5 has no diagnostic-view updates. These combined hit/walk
counters do not establish a data-only miss rate. The earlier per-access-class
measurements in [the performance plan](performance-plan.md) found that nearly
all data requests walked. An earlier trace showed OBP programming TRCR from
`0x00001f80` to zero and then `0x00100000` (VP set, WP=0, TC=0), still present
at Solaris idle.

The current model implements the manual's inclusive wrap point as
`limit = WP`; therefore WP=0 gives `limit=0` and every CPU leaf fill replaces
slot zero. `insert` advances TLBRC modulo `limit+1`, and TC freezes it. This is
a concrete functional explanation for the observed walks, not proof of the
silicon behavior for WP=0.

## Sources inspected

- microSPARC-II User's Manual rev 1.1, sections 5.2.1 and 5.6.6, printed pages
  65 and 88–90, figures 5.15/5.35/5.36 and table 41: replacement counter, WP,
  TC and TLBRC fields.
- Local `docs/dram-tlb.md` and `docs/io-control.md`: documented mask
  `0x01f1ffff`, counter/wrap/freeze behavior, slot-lock redirection and the
  prior OBP trace.
- Pinned QEMU MB86904 source uses a `0x00ffffff` TRCR mask; this does not
  resolve the WP=0 interpretation.
- The inspected manual text does not explicitly say whether WP=0 means a
  one-slot replacement range, a disabled wrap, or all 64 slots. No independent
  physical-hardware differential run is available.

## Conclusion

Do not force WP=63, disable eviction, ignore flushes, or otherwise invent a
WP=0 correction. Keep the current modulo replacement as the documented
functional model until the manual/OBP sequence can be tied to independent
hardware evidence. A later change must be measured separately from the
diagnostic-bookkeeping optimization and must include boot and diagnostic
regressions.

The next useful experiment is a controlled OBP/Solaris trace that captures
TRCR immediately before and after the first CPU fill and compares it with the
manual's replacement-counter timing, without changing the model in the
measurement build.
