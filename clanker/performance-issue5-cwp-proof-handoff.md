# Issue #5: prove the idle loop across register windows

Status: proof checkpoint accepted; tests and evidence remain uncommitted.
Issue #5 remains open. Production admission is unchanged and still rejects
CWP 7. Independent [review](performance-issue5-cwp-proof-review.md) confirms
synthetic full-state invariance and the real paired instruction traces, with
timed device-fingerprint attribution explicitly limited below. The next slice
is the [narrow CWP admission change](performance-issue5-cwp-admission-handoff.md).

## Established result

Helios `ff078fa` and baseline `316ea8a`, with Heliacal `8e8d82a` and the retained
diagnostic patch, execute matching work in common guest-time intervals. The
extra idle execution reaches `PC=0xf006fa24`, `nPC=0xf006fa28` with
`PSR=0x404000e7`. The frontend and core currently require `0x404000e0`.
Only CWP differs: window 7 instead of window 0. Candidate filtering therefore
excludes the state before the mapped proof/readiness checks run.

Both versions exhibit CWP 7. All four pre-idle traces switch 7 to 0 at
112.6472248 s; typed post-idle first observes 0 to 7 at 120.9667565 s.
Baseline also observes window 7 in its later command phase. Different input
delivery guest times and host-window endpoints remain explicit limits.
See [investigation and reviewed arithmetic](performance-issue5-later-idle-investigation.md).

## Scope

Establish whether the captured 14-instruction loop preserves the complete
architectural state when entered under CWP 7. This is a proof/test slice;
production admission stays unchanged. An unexplained transition does not
justify ignoring CWP, and matching PCs alone do not establish parkability.

## Work

1. Capture a real CWP-7 loop entry and a CWP-0 control without changing either
   state. Retain PC/nPC, the entire PSR, WIM, all register windows/aliases and
   the logical `%o0`, `%i0`, `%o7`, `%l0` values. Record the mapped instruction
   path, three load dependencies, relevant MMU translation/diagnostic state,
   and pending device/interrupt state through read-only observation. Do not
   normalize CWP or fetch diagnostic instructions through the live MMU.
   Existing traces record PSR/nPC but not this complete replay state.
2. Around the first post-idle transition, retain a small bounded ring of
   returned execution/trap/interrupt metadata and pre/post CWP. Identify the
   actual CWP-changing instruction or trap return if possible. A transition
   record at the next idle-loop visit alone is not that instruction. Collect
   this while capturing the replay state; no separate broad timing campaign
   is required. Report any remaining causal limit.
3. Add independent replay fixtures for CWP 7 and the CWP-0 control. Execute
   the exact 14-instruction path normally and assert its raw PCs/words,
   branch/delay behavior, lack of traps and return to the same PC/nPC. Require
   full before/after CPU equality, including all inactive windows, aliases,
   WIM, PSR, Y, FPU and guest-visible MMU state, plus unchanged RAM and device
   state for the CPU-only replay. Preserve the captured CWP throughout.
   The existing `tests/support::iteration` hardcodes the CWP-0 PSR; add a
   parameterized test helper without weakening the existing CWP-0 assertions.
4. Extend the synthetic fixture to each implemented window index (0 through
   7), with distinguishable contents in all physical windows and independently
   correct logical operands. Seed sentinels before the checkpoint so alias
   corruption is detectable. Use a guest PSR write or an established test
   setup, not a production CPU behavior change. Keep all non-CWP PSR bits
   exact. The loop contains no SAVE/RESTORE; verify that fact from its retained
   raw words rather than assuming register-bank identity is irrelevant.
5. Include counterexamples: expected operands in the wrong physical window,
   wrong current-window dependencies, unsteady `%o7`/`%l0`, changed helper or
   code/mapping, and non-CWP PSR differences. Distinguish cases that change the
   actual loop from conservative policy exclusions. Existing MMU/PTP2,
   diagnostic-fill, permission/fault and stale-token regressions remain
   required. Do not claim production token acceptance for CWP 7: the current
   API must continue to reject it in this slice.

For the state-invariance lemma, an isolated CPU replay without advancing
device time can use `Machine::diagnostic_snapshot()` as the existing loop
fixture does. Label that scope precisely. It does not establish deadline,
interrupt-wakeup or real frontend parking behavior. If warming is needed for
a synthetic fixture, record it explicitly; do not silently replace an
unsteady real captured checkpoint with a warmed one and claim both are equal.

## Acceptance and stopping point

Return the exact capture/replay artifacts, tracked tests, and a field-by-field
argument about whether CWP changes the existing loop proof. Preserve failure
evidence if any loop is not invariant; do not mask it to make the test pass.
Run the project-required checks for the test changes and retain their results.

If invariance is demonstrated, propose the smallest subsequent production
change for review: only the proved CWP states, all other PSR bits exact,
current-window operand validation, and token snapshots/validation that retain
the actual full CWP and all windows. Core admission, token validation and
both frontend paths must ultimately agree. A token must become stale if CWP
or any window changes after issuance, even if the new state is also eligible.
Do not implement this predicate change in the proof slice.

Keep the 2 ms cap, mapped proof, wakeup/readiness, default-off behavior and
SMP exclusion. Put diagnostics behind the existing default-off feature;
retain their exact patch/build recipe and use separate targets per core
revision. Preserve source disks/NVRAM and the live catalog, clean disposable
builds/media, and return uncommitted. No new host-CPU speedup is claimed from
these diagnostic or synthetic tests.


## Proof result

A real CWP-7 `MachineSnapshot` was captured at `PC=0xf006fa24`,
`nPC=0xf006fa28`, CWP 7, WIM 1, under the controlled diagnostic run:
`local/issue5-later-idle-2026-11-18/captured-cwp7.txt` (505 KB).
The retained capture patch/build recipe is
`diagnostic-cwp7-capture.patch` / `diagnostic-capture-build.md`.

New tracked tests in `tests/parking_windows.rs` seed distinguishable contents
in all eight physical windows, choose exact non-CWP PSR bits, and replay the
complete 14-instruction path for every CWP 0–7:

- raw instruction words, expected PCs, branch/delay behavior and no traps;
- full `Cpu` equality after replay, including every inactive window, aliases,
  WIM, PSR, Y, FPU and MMU state;
- unchanged RAM/DRAM digests, video/MXCC digests; scheduler and clock remain
  zero/unchanged for the CPU-only replay;
- return to the same `PC`/`nPC` with the original CWP;
- independent tests retain the CWP-0 control and reject changed helper,
  wrong operand/helper registers, non-CWP PSR differences and production
  CWP-7 admission.

All nine corrected tests pass. The loop contains no SAVE/RESTORE; its raw words were
checked by the replay. No trap or interrupt entry occurs on the tested path.
The full CPU snapshot is unchanged, so CWP is not architecturally implicated
in the loop’s logical operands or window aliases when all windows are seeded
and non-CWP bits are exact.

The submitted CPU-only replay changes `IoControl.memory_address` from the
fixture's WRPSR fetch at physical `0x2000` to the final loop fetch at `0x06fa64`.
This field is readable at IoControl offset `0x1054` and included in the
parking device digest. It cannot be excluded as an unobservable latch or
removed from the proof comparison. Independent review shows that one explicit
ordinary replay after seeding produces a stable synthetic checkpoint; the
following replay then has **no MachineSnapshot differences** for every CWP
0–7, including the device digest. This correction is now in the submitted
tests, alongside a separate pre-warmup latch-change test and strengthened
wrong-window, bad-current-operand and PIL-policy cases.

The original real capture is an unsteady first entry: at CWP 7, `%o7=0xf006fa44`
and `%l0=0xf025f800`, whereas the existing mapped proof requires
`%o7=0xf006fa58` and `%l0=0xf0071708`. It was captured without replay; the
tracked tests construct a separate synthetic machine. The text snapshot
contains memory fingerprints, not the RAM/code bytes needed to restore a
whole real machine. That original capture remains preserved.

New real samples are retained in
`local/issue5-cwp-proof-2026-11-18/steady-capture/`: CWPs 0–5 and 7 were
observed; no CWP-6 sample was captured in this run. CWP 0 and 7 have the
expected logical operands, PC/nPC and non-CWP PSR state, with WIM 2 and 1
respectively. This is useful real-state evidence, separate from synthetic
coverage of all eight windows.

The intermediate steady-sample hook compares only `%o7`/`%l0` with the previous visit for that CWP
and then saves **one** `MachineSnapshot`. It neither retains the previous full
snapshot nor verifies the 14 intervening outcomes; visits for a CWP may be
separated by other execution. Full snapshots at individual visits do not yet
prove full-state equality across a real iteration. The subsequent paired
capture below supplies the missing full instruction/state comparison.

The remaining causal limit is the real guest transition into CWP 7 at
120.967 s; the proof covers loop invariance for a statically seeded CWP-7
state, not why the guest enters it. No host-CPU speedup is claimed.

### Smallest production proposal, not implemented

If reviewed, the smallest change is:

1. Keep the exact mapped proof and every non-CWP PSR bit requirement, but
   admit the proved CWP values through a single shared PSR mask/validator.
2. Have the frontend candidate predicate and core token admission use that
   same validator, with current-window operand checks unchanged.
3. Keep the full CPU snapshot (all windows, CWP, WIM, PSR, MMU/MXCC) in token
   issuance and validation so any CWP or window change after issuance is stale.

Do not implement this predicate change in the proof slice. The 2 ms cap,
wakeup/readiness, default-off behavior and SMP exclusion remain.


## Real observed loop pairs

A normal-execution capture run (parking disabled, `rtc_host_time=false`,
disposable media/NVRAM) retained before/after complete `MachineSnapshot` pairs
and all 14 returned `Step` records for one CWP-0 and one CWP-7 iteration:

| Pair | Logical time advanced | Time applied to devices | Deferred ns before → after | Steps |
| --- | ---: | ---: | ---: | ---: |
| CWP 0 | 1,400 ns | 2,000 ns | 1,100 → 500 | 14 |
| CWP 7 | 1,400 ns | 2,000 ns | 600 → 0 | 14 |

Applied device time is `deferred_before + logical_delta - deferred_after`.
It includes time deferred from before the pair. The submitted summary's
`device_time_advanced_ns=1400` field is the logical delta, not the applied
device-time total; retain this distinction when reusing the raw evidence.

Both pairs executed the same 14 raw instruction words and returned to
`PC=0xf006fa24`, `nPC=0xf006fa28`. Independent comparison of the full retained
text snapshots finds differences only in `logical_ns`, `device_ns` (deferred
time), and `devices_digest`. CPU/MMU/all windows/aliases and RAM/DRAM are
unchanged. The device hash changes while normal device time advances; the
hash alone cannot identify which device fields changed or prove that every
change was time-derived. No individual device-field equality is claimed from
this hash. The separate warmed CPU-only fixtures provide full device-inclusive
snapshot equality with zero device-time advancement.

Files: `loop-pairs/cwp0.steps`, `loop-pairs/cwp7.steps` and
`loop-pairs-summary.json`. Capture patch/build recipe:
`diagnostic-loop-pair-capture.patch` and `diagnostic-loop-pair-capture.md`.
The earlier first-entry unsteady capture remains preserved separately.

Tracked `tests/parking_windows.rs` now has nine tests: full warm-up snapshot
equality including devices for CWP 0–7, preserved initial latch observation,
bad-current operand, wrong physical window, bad helper operand/helper code,
captured-operand convergence, PIL-only conservative policy rejection, and
production CWP-7 rejection. Core validation is **792 default / 803
parking-profile** successful executions, with formatting, Clippy and all demos
passing.

The remaining limit is still the real guest’s transition into CWP 7; the
measured pair proves one natural CWP-7 iteration, not the transition cause.
No production admission change was made.

Independent pair audit source/results are retained under
`local/review-issue5-cwp-proof-corrected-2026-11-18/pair_audit.{py,json}`.
They verify every raw instruction/PC, PC/nPC chaining, constant CWP, 100 ns
per completed instruction, final boundary and all snapshot text outside the
three reported fields. No new guest run or source change was needed for this
acceptance review. The transition cause and performance benefit of broader
admission remain outside this proof checkpoint.
