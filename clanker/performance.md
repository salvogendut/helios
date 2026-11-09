# Performance branch

Branch `performance` starts from `main` commit `619c80a` (the SS20 work remains
on `ss20-axil320`). `STEP_NS` is unchanged.

## MMU fast translation cache

`Srmmu` now keeps separate 64-entry direct-mapped instruction and data
translation caches, plus cached TLB diagnostic validation. Each cache entry
stores a full-page tag (virtual page, context, supervisor), the TLB slot, and
the TLB generation. Insertions, flushes and diagnostic TLB writes bump the
generation; TLB fills and tried translations are published only after
permission/R-M success, preserving the previous fault atomicity. Context and
supervisor changes are part of the tag, so they do not require a generation
bump. The reference/modified write-back still touches the real TLB entry.

The synthetic harness from `/tmp/helios-speed-review` (5M instructions, three
rounds) measured:

| Case | Before | After |
| --- | ---: | ---: |
| CPU, MMU enabled | 19.76 MIPS | 33.3 MIPS |
| Machine step, MMU enabled | 13.57 MIPS | 16.8 MIPS |
| Machine + CGsix | 12.65 MIPS | 15.1 MIPS |

The same-host Solaris 2.6 boot harness reached `The system is ready.` in
150.4 host seconds after this change, versus 165.5 seconds for the main
baseline measured in the same session (the earlier 146.4-second result was
taken before significant host variance). The full test suite, including MMU
permission, flush, PTE-walk, fault atomicity and no-fault tests, passes. The next core targets are device
deadline/IRQ scheduling and decoded-instruction caching; frontend batching and
repaint reuse belong to Heliacal.
