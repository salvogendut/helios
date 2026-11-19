# Count-only span review

Helios `fd2155c` improves the four reported fixed loops. Heliacal `b33eb55` still uses the detailed API. Adopt the shared machine-boundary correction before switching the worker to count-only execution.

## Reproduced differences

Nine additional fixtures compare detailed and count-only execution. Five failed on the submitted commit.

| Case | Detailed | Submitted count-only |
|---|---|---|
| Guest store to the reset register | SoftwareReset with issuing PC, nPC and store word at this boundary | Executed; the reset remains pending for the next call |
| UNIMP first enters CPU error mode with traps disabled | Watchdog bit latches on the error edge | The CPU returns the same fault but the watchdog bit stays clear |
| Budget-one native bridge with device-service guards enabled | One native retirement returned | Zero returned despite the native call |
| Unconditional device service, interpreted fallback | Zero native calls | One native call |
| Unconditional device service, native bridge | One native retirement returned | Zero returned despite the native call |

The submitted fixture checked fault equality without checking machine state after the fault. Its loop used interpreted fallback with service guards enabled. The additional fixtures also cover branch annulment, the deferred-time boundary, RAM LD/ST, a preexisting reset, single trap and interrupt entry, and graceful power-off.

## Reviewed corrections

The core patch gives both APIs the same machine-step implementation, including device service, watchdog handling before an error is returned, single-word accounting, device time and guest-store reset attribution. The detailed output records each step. The compact output records the number of outcomes, native count and first step directly, including for RAM pairs. Heliacal's worker patch changes its UP span call and reads the returned outcome count. The existing execution-mode, topology, input and parking checks still choose the worker path.

From the Helios checkout, apply the core patch at `fd2155c` and the frontend patch to Heliacal at `b33eb55`.

```sh
ev="$PWD/local/review-workstation-jit-count-only-2026-11-19"
git apply --check "$ev/shared-span-boundary.patch"
git apply "$ev/shared-span-boundary.patch"
git -C ../heliacal apply --check "$ev/heliacal-count-only.patch"
git -C ../heliacal apply "$ev/heliacal-count-only.patch"
```

## Matched fixed work after correction

Each arm used the same uninstrumented release binary, 24,000 warm-up outcomes and 2.4 million timed outcomes. Detailed and count-only ran in alternating order across three pairs per loop. The interpreter was measured in the same build. Checksums, final PC/nPC and virtual time passed in every arm. Native counts matched between JIT APIs.

| SS-20 loop | Interpreter | Detailed JIT | Count-only JIT |
|---|---:|---:|---:|
| Short ADD/branch | 55.63 | 80.69 | 66.08 |
| ADD and word LD | 67.76 | 103.11 | 90.97 |
| Fourteen ADDs | 56.61 | 40.08 | 34.45 |
| Long loop with DMA reset held | 51.32 | 73.43 | 60.00 |

Values are median host ns/outcome. Count-only won every JIT pair. The short loop took 18.1% less time than detailed JIT and still took more time than the interpreter. The submitted timing JSON contains the per-pair counts and timings but omits the binary hash. The corrected timing JSON records both binary and probe hashes.

## Validation

The corrected core passed 814 default and 953 combined JIT-profile/perf-counter test executions, including the ten count-only tests. Formatting, Clippy with warnings denied in both core configurations, and all four demos passed. With the worker patch applied, Heliacal passed formatting and 126 tests under `jit-profile`, including budget, lifecycle and power-off checks.

## Solaris decision

The previous SS-20 Solaris gate remains open. After adopting both patches, use the same Heliacal release binary for the existing interpreter/JIT Solaris command comparison on disposable clones. Run the first pair with the existing 330-second limit per arm. Stop if JIT is slower than the interpreter on the fixed guest command or either arm fails the guest checks. If it remains competitive, finish the original three alternating pairs in total. Acceptance still requires correct guest results and a JIT median at least 10% below the matched interpreter median.

The patch files, failing and corrected fixtures, fixed-work probe and timings are under `local/review-workstation-jit-count-only-2026-11-19/`. This review used disposable checkouts. No Solaris boot, live configuration change or production commit was made.
