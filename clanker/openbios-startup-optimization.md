# OpenBIOS startup optimization — 2026-11-16

Follow-up to the [cost profile](openbios-cost-profile.md). The previous image
spent most guest instructions in the C Forth dispatcher and stack primitives.
The framebuffer palette loop performs 256 dictionary searches for `color!`,
including full-chain misses when TurboGX keeps that method private. Translating
that search into guest-native SPARC code removes most of the repeated dispatch
work without changing the palette loop or adding a dictionary cache.

## Implementation and provenance

`tools/openbios/helios.patch` now selects `-O2` and adds one primitive at the
end of the existing primitive tables, preserving their earlier indices.
`find-wordlist` calls it; `(find-wordlist-reference)` retains the original
Forth walk for comparison. The primitive follows each current link, handles
header padding and length, folds only ASCII a–z, returns the same XT/flag on
success, and preserves the query address/length on a miss. All dictionary
access uses the upstream cross-compiler accessors. Search order, locals and
immediate flags remain in the Forth callers. This is guest firmware code;
Helios still executes and accounts for every SPARC instruction.

`tools/openbios/search-checks.fs` covers aligned/padded headers, case,
punctuation, length mismatches and absent names, empty chains, stack results,
immediate flags, different wordlists/search orders and live redefinition.
It compares against the original walk and independently known values. Input
lines fit the firmware console's limit; the runner accepts the stack-depth
prompt while a definition or intermediate result is pending. Earlier private
fixture attempts exposed harness line truncation and an incomplete search
order; the final tracked fixture corrects both and rejects undefined words.

The build helper records `-O2` in the provenance manifest. A private `-O3`
experiment stopped on an unsupported MMIO width and was rejected; no device
width checks were relaxed. The existing full RAM clear, timer delays, SCSI
sequencing, CPU clocks and UI-independent device execution are unchanged.

Final image: `665b42e56419fec7defb41cda68d027f6077cf52d9a36329082603a862b18947`.
Patch: `d6503b164c4f06e3031a243482bd50c00fc51b34fe0fbc7c8ee10cfb5a16e3b3`.
Upstream: `e5ac46dd24e6216c36aa80462af25457e7029440`.
Compiler: SPARC GCC 16.2.1 (20260819), fresh checkout/build, target `-O2`.

## Measurements

Same SS-5, 256 MiB, one CPU, fast clocks, read-only Solaris disk at SCSI 3,
release profiler with `perf-counters`, no SDL/PTY/network. Final runs were
sequential and finished before other validation/build work started. The old
image is the single baseline run from the cost profile; final values are
medians of three runs, so wall-time ratios are local observations rather than
a claim of controlled cross-host performance.

| Milestone | Previous image | Optimized image | Previous outcomes | Optimized outcomes |
| --- | ---: | ---: | ---: | ---: |
| Serial `0 >` | 43.605 s | 15.176 s | 546,150,000 | 165,540,000 |
| First CGsix banner pixels | 157.649 s | 21.364 s | 2,014,000,000 | 243,000,000 |

Serial runs: 15.176 / 15.129 / 15.182 s. Graphics: 21.306 / 21.364 / 21.568 s.
All three final runs have identical outcome counts: serial 165,539,880
instructions / 120 synchronous traps / zero interrupts; graphics 242,999,862
instructions / 138 traps / zero interrupts. These expected trap counts include
firmware probes and window traps, not error stops. Walks remain small (1,154
serial, 1,743 graphics). `-O2` alone took 385.53M outcomes / 32.588 s on serial;
the native search accounts for the larger improvement.

The graphical stop still means three nonuniform scanouts after the UUID line,
not a completed prompt. The actual Heliacal console is checked separately.
Full RAM clearing still takes 62,914,624 startup outcomes for 256 MiB, including
entry setup; further speedups must not simply skip its writes or reset effects.

## Acceptance evidence

Evidence is retained under `local/openbios-speed-2026-11-16/`: manifests,
measurement JSON/logs/samples, probe and harness sources, firmware checks,
OS logs and Heliacal captures. Source disks and the live catalog are excluded
from mutations. A private clone with `scsi3` in its filename is used for writes.

Final acceptance:

- Final-image search fixtures on SS-5/128 MiB and two-CPU SS-20/512 MiB;
  counted-string fixtures on SS-5; two-CPU Axil/512 MiB serial prompt and tree.
- Eight full-RAM cold/reset checks: SS-5 32/256 MiB and SS-20/Axil 512 MiB.
  Every installed RAM word is zero and entry-state registers are preserved.
  This build's RAM fixture stops at low `clear_done` PC `0x6ca4`.
- Actual Heliacal/Xvfb software-renderer sessions on SS-5/256 MiB/one CPU,
  SS-20/128 MiB/two CPUs and Axil/128 MiB/two CPUs, all using TurboGX.
  The SS-5 complete prompt is visible by the 25-second capture. All three
  accept captured `1 2 + . cr`, display `3`, release input and quit with status 0.
  These are disk-free firmware checks, not graphical OS acceptance.
- NetBSD 11.1 on the SS-20/128 MiB CHD clone: single-user shell, guest write,
  clean halt, reopen and reread; `/openbios-test` is `3147149250 24` both times.
- Solaris 2.6 CD on Axil/128 MiB/two CPUs: `boot cdrom -s`, single-user
  shell, `uname -a`, `psrinfo -v` reports processors 0 and 2 online at 75 MHz,
  then `sync; halt` powers off cleanly. The source CHD hash is unchanged
  (`9042243c22e44592fe5ad59838be2e66f4031135630f0be0bdb68ace4db79725`).
- Core: 683 unique default and 700 all-feature tests, formatting, both strict
  Clippy configurations and all four demos. Raw successful counts are 684/701
  because each suite reruns one isolated MXCC child. Heliacal: 81 tests, fmt,
  strict Clippy, release rebuild, SDL smoke and original SS-5 OBP display smoke.

This performance follow-up changes the firmware patch/build metadata and
related frontend notice/docs; it does not change Rust CPU/device behavior.
The final installation check also exposed an empty `--prom ""` CLI override
being passed to `absolute()`. Heliacal now preserves that empty sentinel,
consistent with the configured automatic selection. The user confirmed the
installed result works and authorized committing, pushing and merging the
issue #8 core/frontend changes on 2026-11-16.

The tested ELF/provenance are installed at
`local/firmware/openbios-helios-sparc32.{elf,json}`. The rebuilt frontend's
installed-image smoke test explicitly clears a configured Sun ROM with
`--prom ""`, selects that default ELF and produces display output. Restart or
cold-replace an existing machine to use the new image; an already running guest
keeps its loaded firmware.

Cleanup removed private upstream/build trees, intermediate images, the written
CHD clone, probe executables, test NVRAM/locks and redundant large captures.
Logs/captures/provenance and harness sources remain (~1.4 MiB); large traces are
losslessly compressed. The previous baseline ELF is retained for comparison.
No source disk, live configuration, original OBP ROM or user NVRAM was changed.
`cleanup.json` lists the removals. To rerun probes, copy their retained Rust
sources to temporary Cargo examples, build `openbios_cost` with `perf-counters`
and `openbios_probe` with `chd`, then remove those examples again. The official
builder can recreate a fresh source/output tree from the tracked patch.
