# Helios OpenBIOS port

`helios.patch` applies to official OpenBIOS commit
`e5ac46dd24e6216c36aa80462af25457e7029440`. It is a source port of that
GPL-2.0 firmware; upstream file notices and its COPYING terms apply. No Sun
PROM, TurboGX ROM, upstream source tree or compiled firmware is bundled here.

Run `python3 tools/build_openbios.py` from the Helios checkout. The helper
always uses a fresh checkout: incremental upstream builds do not reliably
notice linker-script changes. Keep the resulting ELF and provenance JSON
together. See [firmware documentation](../../docs/firmware.md#automatic-openbios-fallback)
for prerequisites, platform differences, configuration search and current
compatibility limits. The Rust core's opt-in firmware interface remains
separate from the physical Sun hardware model.

`checks.fs` can be sent line-by-line at the port's serial prompt. It checks
shortening, empty and 255-byte legacy counted strings, their terminator and
neighboring sentinels, and preservation of ordinary `pack`. The NUL terminator
is a Sun version-1 compatibility convention, not a new IEEE requirement;
version-1 destination buffers need length + 2 bytes. TurboGX's original FCode
also exercises the token binding during the graphical acceptance run.

The startup clear remains a real guest RAM write over the full original range.
Its 64-byte blocks require the existing MiB-aligned RAM sizes and 0/16 MiB
start addresses. Both source registers are zero; using `stda %g0` would also
store `%g1` (the RAM size). Console initialization still follows the clear.

The target uses `-O2` and a native `find-wordlist` chain walk. The native
primitive is part of the guest firmware: every read and instruction still
passes through Helios. It uses OpenBIOS's target-cell and dictionary-pointer
accessors, folds only ASCII lowercase letters, and follows the current link
chain on every call. Search order, locals and immediate-word flags stay in
the existing Forth callers; there is no lookup cache to invalidate.

`search-checks.fs` compares the native primitive with the retained original
`(find-wordlist-reference)` and checks independently expected results. It covers
header alignment, case and punctuation, misses and stack preservation, empty
chains, immediate flags, search-order changes and newly redefined words. Send
its lines individually (each fits the console's input limit), accepting the
stack-depth prompt while a definition or intermediate result is pending.
The final `HELIOS-OPENBIOS-CHECK-PASSED` follows a zero-depth assertion.
