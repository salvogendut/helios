# SPARC JIT references for Helios

2026-11-19. Source inspection requested alongside the first native JIT review.
The following revisions are pinned in retained source/hash manifests under
`local/review-issue5-native-2026-11-19/`. Upstream source remains ignored local
reference material; no implementation was copied into Helios. Upstream guest
boot and performance claims were not reproduced in this review.

## QEMU: block state, precise boundaries and code invalidation

Inspected the repository's existing QEMU reference
`f71afb409a87c4ecd0f9d63a5963eb645dfab780`, plus the official TCG documentation.

QEMU's SPARC translator produces TCG operations, which a host backend turns
into machine code. Its SPARC translation context explicitly carries PC/nPC
and execution flags. `sparc_tr_init_disas_context` obtains nPC from the TB's
`cs_base`, imports the MMU index and applicable CPU-mode flags, and limits
translation to the current page. `sparc_tr_insn_start` records both PC and nPC
for precise state restoration, including dynamic and conditional targets.
[SPARC translator](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c#L5804).

State-changing helpers terminate or force a fresh block lookup. In the
32-bit alternate-store path, the translator stops continuing with static
translation assumptions after a helper that can change mappings; WRPSR
explicitly sets `DISAS_EXIT`. This directly informs the current Helios bug:
successful interpreter fallback can invalidate the conditions under which
native execution was admitted.
[ASI-store handling](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c#L1938),
[WRPSR](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c#L3518).

QEMU couples translation lookup to CPU state and physical code identity, and
tracks code writes at page granularity. Its dispatch/chaining mechanisms avoid
returning through the general interpreter for every supported instruction;
exits that may expose interrupts still return through the execution loop.
These are design references for a later persistent cache, not a reason to
skip Helios's existing bus/MMU effects or interrupt rules.
[Translator internals](https://www.qemu.org/docs/master/devel/tcg.html).

With instruction counting enabled, QEMU checks a block against an execution
budget derived from the next timer deadline. An oversized block is replaced
by a suitably limited execution block; MMIO is handled with precise boundaries.
This is not cycle-accurate CPU simulation. For the present Helios subset,
falling back for a short remaining suffix is sufficient; adopting QEMU's
whole timing framework is unnecessary.
[Instruction counting](https://www.qemu.org/docs/master/devel/tcg-icount.html).

TCG also avoids unconditional copying of every architectural register at each
block boundary. The SPARC translator maps globals and window registers into
TCG state, and TCG optimizes temporary liveness and helper synchronization.
This is relevant to Helios's measured marshalling overhead, after correctness.
[SPARC register setup](https://github.com/qemu/qemu/blob/f71afb409a87c4ecd0f9d63a5963eb645dfab780/target/sparc/translate.c#L6020),
[TCG operations](https://www.qemu.org/docs/master/devel/tcg-ops.html).

## InfiniteSPARCjit: QEMU/Wasm diagnostics and address-width discipline

Inspected `elevenarms/InfiniteSPARCjit` at
`a03c746dc96d348748c51f045b52e425ac1a2aac` (2026-10-21), including its README,
build script, QEMU patch and Node harness.

This project uses a QEMU 8.2-derived `ktock/qemu-wasm` fork for a browser
SPARCstation 5. Its backend initially interprets blocks, then instantiates
WebAssembly modules for hot blocks. It relies on QEMU's SPARC machine and
translator rather than implementing an independent Rust SPARC core.
[Project description](https://github.com/elevenarms/InfiniteSPARCjit/blob/a03c746dc96d348748c51f045b52e425ac1a2aac/README.md).

The useful patch is concrete: `tcg_wasm_out_guest_addr` explicitly zero-extends
32-bit guest addresses before inline memory/TLB operations and slow helpers.
Other changes add release-time bounds checks, fix helper/label buffer clearing,
and provide diagnostic controls for compilation thresholds, PC ranges,
excluded operations and self-loop routing. These allow a failing guest block
to be isolated without abandoning the interpreter comparison.
[Wasm/SPARC patch](https://github.com/elevenarms/InfiniteSPARCjit/blob/a03c746dc96d348748c51f045b52e425ac1a2aac/emu/patches/qemu-wasm-sparc32.patch).

The transferable lesson is explicit guest-address semantics and reproducible
compiler bisection. Its zero-extension bug is distinct from Helios's missing
31-bit physical mask; both show why host-width addresses cannot replace the
guest memory contract. Browser stacks/fibers and display details do not need
to enter the current native milestone. Boot timings remain the author's
measurements on a different runtime, guest, firmware and machine configuration.

## UltraViolet: the closest small native-engine comparison

Inspected `MW0MWZ/UltraViolet` at
`629a296f75f910d97b61d2eb102ff3690c4b0df7` (2026-11-15), including execution,
MMU, x86-64 emitter, executable-buffer and differential-test sources.

It targets UltraSPARC IIi / SPARC V9 / sun4u. Its execution-engine organization
is useful to Helios; hardware semantics still need the V8/sun4m sources.
Decoded blocks have a run count, and native compilation starts at a threshold
of 16 in the inspected source. It includes x86-64 and AArch64 backends.
[CPU/block declarations](https://github.com/MW0MWZ/UltraViolet/blob/629a296f75f910d97b61d2eb102ff3690c4b0df7/src/cpu/ultrasparc/ultrasparc.h).

`run_blocks` checks all of the following before native entry: the block fits
the remaining instruction/timer budget, nPC equals PC+4, and the block's
compiled mode matches the current mode. A fallback instruction can set an
attention flag that ends execution. Its `code_written` callback invalidates
blocks when writes touch tracked code lines; stores to neighboring data need
not invalidate them. `flush_blocks` handles changed host mappings.
[CPU execution and invalidation](https://github.com/MW0MWZ/UltraViolet/blob/629a296f75f910d97b61d2eb102ff3690c4b0df7/src/cpu/ultrasparc/ultrasparc.cpp#L742).

The native emitter retains frequently used guest registers in host registers
and keeps pointers to the relevant register sets/windows. It writes back
changed values and reloads them around helpers that can change architectural
state. This addresses the same class of overhead as Helios's unconditional
32-register copy in/out. Its OS-specific executable-buffer abstraction also
separates allocation/protection from instruction generation.
[x86-64 emitter](https://github.com/MW0MWZ/UltraViolet/blob/629a296f75f910d97b61d2eb102ff3690c4b0df7/src/cpu/ultrasparc/jit_x64.cpp),
[executable buffers](https://github.com/MW0MWZ/UltraViolet/blob/629a296f75f910d97b61d2eb102ff3690c4b0df7/src/jit/exec_buffer.cpp).

Its test suite covers ALU/flags, delay/annul behavior, code writes, windows,
page transitions and bounded loops. The runtime `jit_run_checked` checker is
useful but narrower than a complete machine-state oracle: it compares selected
integer state for eligible self-contained blocks and abandons replay when
control leaves the linear instruction sequence. Retain Helios's broader
snapshot checks and independent expected-state fixtures.
[JIT tests](https://github.com/MW0MWZ/UltraViolet/blob/629a296f75f910d97b61d2eb102ff3690c4b0df7/tests/jit_tests.cpp),
[runtime checker](https://github.com/MW0MWZ/UltraViolet/blob/629a296f75f910d97b61d2eb102ff3690c4b0df7/src/cpu/ultrasparc/ultrasparc.cpp#L934).

## Apply the research without expanding this milestone

First apply the three small corrections in [the native review](jit-native-review.md)
and promote their fixtures. Finish validation and the default-off milestone.

For later measured improvements, the most relevant order is: selective
register/ICC marshalling, persistent bounded hot-block caching with precise
code invalidation, and only then broader instruction/memory coverage and
chaining. A source/destination register mask can reduce copies while retaining
Helios's existing `read_reg`/`write_reg` window semantics; no raw access to Rust
CPU internals is needed for that first improvement. Each item belongs in a
separate bounded follow-up, not the current merge gate. None of these references
justifies altering guest timing, MMU replacement, or the accepted parking policy.
