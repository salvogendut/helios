# microSPARC-II floating-point state and execution

Helios implements the initial FPU state and transfer subset: 32 word registers,
LDF/LDDF/LDFSR, STF/STDF/STFSR, FMOVs/FNEGs/FABSs and all FBfcc conditions.
Single/double comparisons and division, deferred IEEE exceptions, sequence
errors and STDFQ queue draining are implemented. Conversions, other arithmetic,
quad-operation traps and unfinished operations remain explicit emulator stops
when the FPU is enabled.

The primary sources are SPARC V8 sections 4.4, 5.1.7 and 7, appendices B.2,
B.5, B.22, B.34 and N.3–N.6, and the microSPARC-II User's Manual sections
4.3–4.5, especially figure 4.8 and table 6. [References](references.md) records the manual and pinned
QEMU/MAME comparisons.

## Register state and transfers

`Cpu::fpu` owns 32 `u32` bit patterns. All registers, including f0, are writable
and independent of integer windows and g0. Loads, stores and moves preserve raw
bits, including signed zero and NaN payloads; they do not use host floating-point
arithmetic or perform byte swaps. Debugger access uses `read_reg`/`write_reg`;
guest execution checks PSR.EF separately.

Single transfers use aligned words. Double transfers require an eight-byte-aligned
address, placing the word at the lower address in the even register and the next
word in its odd partner. They reuse the checked bus doubleword operation, so a
failed transfer does not commit half a destination or half a store. The effective
address is the wrapping integer-register sum with a sign-extended immediate or
second register, using supervisor/user data ASI 11/10 according to PSR.S.
Translation, permissions, R/M updates, NF suppression and bus-error capture follow
the existing [memory system](memory.md). Unsupported MMIO stays a host stop.

V8 reserves the low rd bit for double transfers and recommends, without requiring,
an invalid-register exception for an odd encoding. Helios ignores that bit and
uses the containing even/odd pair, matching MAME's load decoder. The microSPARC-II
FSR table does not list the optional invalid-register FTT. This is an explicit
choice for nonportable encodings, not proof of physical silicon behavior.
LDFSR/STFSR ignore rd; no alternate-space FPU transfers are implemented for V8.

## Status register

FSR reads combine the writable fields with the fixed Meiko version value 4 in
bits 19:17. LDFSR accepts mask `0xcf800fff`:

| Bits | Field | Behavior |
| --- | --- | --- |
| 31:30 | RD | Division: nearest/even, toward zero, +infinity, -infinity |
| 27:23 | TEM | IEEE trap enables; implemented for comparisons/division |
| 22 | NS | Always zero on microSPARC-II |
| 19:17 | VER | Always 4 |
| 16:14 | FTT | Retained across LDFSR/STFSR; 1 for IEEE exceptions, 4 for sequence errors |
| 13 | QNE | One while a deferred entry awaits STDFQ |
| 11:10 | FCC | E/L/G/U encoded as 0/1/2/3 |
| 9:5 | AEXC | Stored accrued exceptions |
| 4:0 | CEXC | Stored current exceptions |

Other bits read zero. STFSR does not clear FTT, following microSPARC-II section
4.5. FMOVs, FNEGs and FABSs clear FTT and CEXC on completion, retaining AEXC,
FCC, RD and TEM. They cannot generate IEEE exceptions, even for signaling NaN
bit patterns. FNEGs toggles only the sign bit; FABSs clears only that bit.
A failed LDFSR preserves the previous status.

CPU construction, power reset and the current software-reset baseline zero the
FPU registers and writable status; FSR still reads `0x00080000`, and PSR.EF is
clear. This is deterministic initialization of unspecified state, not a claim
that every FPU bit has that hardware reset value.

## Branches and traps

FBfcc uses FCC independently of ICC, with the same delayed-control-transfer and
annul rules as Bicc. Conditional taken branches execute their delay slot;
not-taken annulled branches and FBA,a skip it. FBN and FBA still check EF.
FSR updates are visible immediately. Portable guest code should retain the
manual's spacing after LDFSR and FPop2 instructions; pipeline interlocks
and timing are not modeled.

Recognized floating-point instructions check EF before data access or branch
annul. EF=0 raises `fp_disabled` (TT `0x04`), including FPop1/FPop2 operations
that are otherwise unimplemented. ET=1 enters the existing precise trap path,
preserving faulting PC/nPC including a pending branch target. ET=0 latches CPU
error state. Instruction-fetch faults still precede instruction execution.

STDFQ checks supervisor privilege, EF, then eight-byte alignment. When the queue
is empty, an aligned enabled STDFQ sets FTT=4 and raises an immediate
`fp_exception` (TT `0x08`) before translation or a memory write.
The FPU remains in its execution state and QNE remains zero, as microSPARC-II
specifies for an empty queue. Ordinary transfers and status reads do not clear
this FTT; a completed nontrapping FPop does.

## Comparisons

FCMPs/FCMPd and FCMPEs/FCMPEd compare raw IEEE encodings without using host
floating-point operations. Signed zeros compare equal, subnormals retain their
value, and finite values/infinities follow numeric order. Double comparisons use
both words of each even/odd register pair; the reserved low register bit is
ignored, as for double transfers. FCC records equal/less/greater/unordered.
No source register is modified, and RD does not affect comparisons.

FCMP raises invalid only for a signaling NaN. FCMPE raises invalid for either
quiet or signaling NaNs. The most significant fraction bit distinguishes quiet
NaNs; a nonzero fraction with that bit clear is signaling. For a masked invalid
operation, FCC becomes unordered, CEXC.NV is set and AEXC.NV accumulates.
An ordinary successful comparison clears CEXC/FTT, updates FCC and retains AEXC.

With TEM.NV enabled, the comparison preserves FCC, AEXC and all FPU registers,
sets CEXC.NV alone and FTT=1, records its PC/instruction and enters
exception-pending state. The integer instruction stream advances without an
immediate trap. These status rules follow V8's inhibited-result requirements;
the deferred timing follows microSPARC-II section 4.4.

## Division

FDIVs (`opf=0x4d`) and FDIVd (`opf=0x4e`) divide rs1 by rs2. Double operands
and destinations use both words of the containing even/odd pair, ignoring the
reserved low register bits as with transfers and comparisons. Both sources are
read before writing the destination, including overlapping pairs and f0.

The implementation uses normalized integer significands and an exact `u128`
quotient/remainder, never host floating-point arithmetic or host rounding state.
It rounds once onto the destination's normal or subnormal grid. RD selects
nearest with ties to even, toward zero, toward positive infinity or toward
negative infinity. Finite exact results retain every bit; signed zero and
infinity use the XOR of operand signs. Quiet NaNs follow the rules below.

The five CEXC/AEXC bits are NX=1, DZ=2, UF=4, OF=8 and NV=16. A completed,
nontrapping division sets CEXC to its flags, ORs them into AEXC, clears FTT and
preserves FCC/RD/TEM. An exact ordinary result therefore clears stale CEXC
without losing accrued exceptions. If an exception is enabled, no destination,
FCC or AEXC changes: only the highest-priority enabled cause enters CEXC
(NV, OF, UF, DZ, NX), FTT becomes 1 and the original instruction is deferred.
For example, masked underflow with enabled inexact reports NX alone on the trap.

| Case | Result with the relevant exception masked | Flags |
| --- | --- | --- |
| Nonzero finite / signed zero | Signed infinity | DZ |
| Infinity / zero or finite | Signed infinity | None |
| Finite / infinity | Signed zero | None |
| Zero / zero or infinity / infinity | Positive default quiet NaN | NV |
| Rounded overflow | Infinity or maximum finite, according to RD/sign | OF and NX |
| Tiny, inexact result | Gradually rounded subnormal or zero; may round up to minimum normal | UF and NX |
| Tiny, exact result | Exact subnormal | None when UFM=0; UF trap when UFM=1 |
| Other inexact finite result | Rounded quotient | NX |

Tininess is detected **before rounding**, as required by V8 N.5/N.6. Underflow
therefore survives rounding up to minimum normal. Enabled UFM also detects
exact nonzero tiny results. Overflow is detected after rounding with an
unbounded exponent; directed rounding chooses infinity only when its direction
agrees with the result's sign, otherwise maximum finite. Nearest uses infinity
for overflow. Results too small even for half a minimum subnormal still obey
all directed rounding modes, without oversized integer shifts.

microSPARC-II figure 4.8 defines the Meiko NaN behavior. A quiet NaN preserves
its payload and clears its sign; if both operands are quiet NaNs, rs1 wins.
Any signaling NaN raises NV and yields positive `ME_NaN`: `0x7fff0000` for
single or `0x7fffe00000000000` for double when masked. For invalid numeric inputs
(0/0 and infinity/infinity), Helios uses V8 N.4's recommended positive quiet NaN
with all fraction bits set (`0x7fffffff` / `0x7fffffffffffffff`). The chip's table
labels numeric/numeric results as IEEE 754 but does not specify this payload;
that choice has not been verified against silicon. These rules deliberately
follow the chip manual where generic QEMU NaN propagation differs.

## Deferred exceptions and queue

`Fpu::state()` exposes execution, exception-pending and exception states.
`deferred_entry()` inspects the captured PC/instruction without consuming it.
The following rules apply after instruction fetch, privilege, EF and alignment
checks, and before data translation or any FP instruction effects:

| Current state | Floating instruction | Result |
| --- | --- | --- |
| Execute | Enabled IEEE exception from comparison/division | Capture original FPop, enter pending; advance integer PC |
| Pending | Any FP operation, load, store or branch | Enter exception state and take TT `0x08`; detecting instruction does not execute |
| Exception | FPop or FP load, including LDFSR | Set FTT=4, return to pending; skip instruction effects and retain the original queue entry |
| Exception | FP branch, STF/STDF/STFSR | Execute normally, preserving the queue |
| Exception | Successful STDFQ | Write and remove the queue entry; return to execute |

Integer instructions and interrupts leave FPU state pending until another FP
instruction is attempted. Trap entry saves the detecting instruction's PC/nPC,
including a branch target for a delay-slot fault. The queue retains the original
exception-causing FPop's address and instruction. With ET=0, CPU error state is
latched when the deferred exception is detected, not when the FPop issues.
The issuing or sequence-error instruction is counted as an executed integer
instruction even though its FP result is inhibited.

STDFQ stores the original PC at the effective address and the instruction at
address+4, using the checked doubleword store. A failed or unsupported store
preserves the entry. A successful store clears QNE but retains FTT/CEXC until a
subsequent FPop changes them. The current PCR.NF contract reports a suppressed
memory fault as a completed store; that completion also commits queue removal.
Both CPU reset paths clear the queue and return the FPU to execution state.

The hardware has three pipeline entries. This interpreter resolves one FPop
before issuing another, so at most the original faulting entry remains queued;
there are no younger in-flight FPops to save. The entry format and state
transitions are implemented, while multi-entry pipeline timing is not modeled.
There is no fabricated pipeline occupancy or host-dependent completion order.

An unsupported FPop in execution state remains a retryable
`UnsupportedInstruction` host stop. In pending/exception state the architectural
state checks take precedence, just as they do for supported FP instructions.
Arithmetic other than division, conversions, quad-operation traps and unfinished
operations remain pending; this is not a complete IEEE arithmetic implementation.

## Validation and firmware

Tests cover every FPU register, raw bit patterns, all branch condition/FCC/annul
combinations, signed displacements, wrapped addresses, masks, exception priority,
delay-slot trap state, MMU permissions/NF, failed doublewords, unsupported MMIO,
SBus asynchronous stores and resets. These are focused regressions, not an FPU
conformance suite.

Comparison tests cover ordered/subnormal/infinite values, signed zeros, NaN
classes and TEM settings, inhibited results, delayed detection, priority,
sequence errors, queue writes/retries, NF, interrupts, delay slots and reset.

Division tests cover both precisions, all rounding modes/signs, destination
aliases, gradual underflow, halfway cases, exact-tiny traps, overflow, NaN
payload/sign rules, all five exceptions, inhibited results and deferred recovery.
The independent exact-fraction oracle described in [HACKING.md](../HACKING.md)
passes 48000 guest divisions with seed `0x535335` and 2048 random operand pairs
per precision, plus targeted boundary vectors. It validates result words, FSR
and queue presence; this is not exhaustive or a silicon comparison.

Original OBP now passes the two FCMPs, following floating branches and its first
FDIVs (`100 / 4 = 25`), reaching unsupported FADDs at `0x70002864`.
It has no serial output or firmware prompt.
[Firmware records](firmware.md) retain the bounded checkpoint and exact limits.
