# microSPARC-II floating-point state and execution

Helios implements the initial FPU state and transfer subset: 32 word registers,
LDF/LDDF/LDFSR, STF/STDF/STFSR, FMOVs/FNEGs/FABSs and all FBfcc conditions.
Single/double comparisons, division, addition/subtraction, multiplication and
square root, deferred IEEE exceptions, sequence errors and STDFQ queue draining
are implemented. Integer, single and double precision conversions are
implemented. Other arithmetic, quad-operation traps and unfinished operations
remain explicit emulator stops when the FPU is enabled.

The primary sources are SPARC V8 sections 4.4, 5.1.7 and 7, appendices B.2,
B.5, B.22, B.34 and N.3–N.7, and the microSPARC-II User's Manual sections
4.3–4.5, especially figures 4.8–4.9 and table 6. [References](references.md) records the manual and pinned
QEMU/MAME comparisons.

## Conversions

FITOS (`opf=0xc4`) and FITOD (`opf=0xc8`) interpret the source FP word as a
signed 32-bit integer. FITOD is always exact; FITOS rounds with FSR.RD and
raises NX when low integer bits cannot be represented. Both preserve FCC and
accrued exceptions, clear stale CEXC/FTT on exact completion, and use the same
deferred exception path as arithmetic. An enabled NX inhibits the destination
and AEXC update. Integer bit operations produce the result without host floating
point. The source word is read before either destination word is written;
double destinations use the established containing-pair convention.

FSTOI (`0xd1`) and FDTOI (`0xd2`) always truncate toward zero, independently
of RD. Fractional results raise NX; NaN, infinity and results outside signed
32-bit range raise NV. Range is checked after truncation, so -2147483648.75
produces -2147483648 with NX. Numeric invalid results saturate by operand sign.

FSTOD (`0xc9`) widens all finite values exactly. FDTOS (`0xc6`) rounds once,
including gradual underflow, tininess before rounding and overflow after
rounding. Exact tiny results raise UF only when UFM is enabled. Signed zero
and infinity retain their sign in both directions.

NaNs follow microSPARC-II figure 4.9 (visually checked), which differs from
V8's recommended defaults. FSTOD preserves quiet payloads with sign cleared
and canonicalizes signaling inputs to the double ME_NaN. FDTOS returns the
single ME_NaN for every NaN. Only signaling NaNs raise NV on format conversion.
For integer conversion every NaN raises NV; a positive quiet NaN returns the
32-bit ME_NaN word (`0x7fff0000`), other positive invalid operands return
`0x7fffffff`, and negative invalid operands return `0x80000000`.

All conversions preserve FCC, read the complete source before writing any
result, and use the same exception/queue handling as arithmetic. Quad
conversions remain explicit host stops. QEMU's pinned round-to-zero helpers
agree with V8 for integer results; MAME passes a general SoftFloat rounding
mode for FSTOI/FDTOI, so that call is not used as a rounding oracle.

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

FSR reads combine the writable fields with the selected CPU model's fixed
version in bits 19:17: 4 (Meiko) for microSPARC-II and 0 for SuperSPARC-II.
The CPU model is selected by the machine profile before power-on reset.
LDFSR accepts mask `0xcf800fff`:

| Bits | Field | Behavior |
| --- | --- | --- |
| 31:30 | RD | Arithmetic: nearest/even, toward zero, +infinity, -infinity |
| 27:23 | TEM | IEEE trap enables; implemented for comparisons/arithmetic |
| 22 | NS | Always zero on microSPARC-II |
| 19:17 | VER | Model-specific: 4 on microSPARC-II, 0 on SuperSPARC-II |
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
nontrapping arithmetic operation sets CEXC to its flags, ORs them into AEXC,
clears FTT and preserves FCC/RD/TEM. An exact ordinary result therefore clears stale CEXC
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
(0/0, infinity/infinity, infinity minus itself and zero times infinity), Helios uses V8 N.4's
recommended positive quiet NaN with all fraction bits set (`0x7fffffff` / `0x7fffffffffffffff`). The chip's table
labels numeric/numeric results as IEEE 754 but does not specify this payload;
that choice has not been verified against silicon. These rules deliberately
follow the chip manual where generic QEMU NaN propagation differs.

## Addition and subtraction

FADDs/FADDd (`opf=0x41/0x42`) add rs1 and rs2; FSUBs/FSUBd (`0x45/0x46`)
subtract rs2 from rs1. They share division's register-pair choices, rounding,
NaN propagation, status updates and deferred traps. Operand order for NaN
selection is preserved even when the numeric computation orders magnitudes.

Integer significands are aligned with three low guard/round/sticky bits. A
right shift folds discarded nonzero bits into the sticky bit, including shifts
of 64 or more. Opposite signs subtract magnitudes; close operands cancel exactly
before normalization. When alignment discards bits, cancellation needs at most
one left shift, retaining enough information for the final rounding decision.
All intermediates fit in `u64`; the largest sum occupies 57 bits for double
precision. No host floating-point operations or rounding state are used.

Exact cancellation produces positive zero except under rounding toward negative
infinity, which produces negative zero. Equal-sign zero operands retain their
sign after applying subtraction's sign change: for example, `-0 - +0 = -0`.
A finite same-format sum/difference below minimum normal is an exact multiple of
the minimum subnormal. These tiny results raise UF when UFM is enabled, but do
not raise UF or NX when UFM is masked. Exact zero raises neither exception.

Infinity plus its opposite, or infinity minus itself, raises NV and uses the
same documented numeric-invalid NaN default as division. Other infinite results
retain the appropriate sign. Addition/subtraction never raises DZ.

Overflow is checked after rounding, not merely because the exact magnitude
exceeds maximum finite. For example, maximum finite plus a minimum subnormal
rounds back to maximum finite with NX alone under nearest rounding; toward
positive infinity it yields infinity with OF and NX. Tests also cover the
halfway overflow boundary, negative results and enabled OF versus NX traps.

## Multiplication

FMULs/FMULd (`opf=0x49/0x4a`) multiply rs1 and rs2 in the same precision.
Both sources are read before committing a destination, including overlapping
pairs. Register-pair choices, NaN propagation, FSR updates and deferred exception
handling are shared with the other arithmetic instructions.

The exact product of integer significands occupies at most 48 bits for single
precision and 106 bits for double, so `u128` retains all product bits. The result
is rounded once directly onto its normal or subnormal grid. Discarded bits are
compared with half the grid spacing for ties-to-even and directed rounding;
extremely small products are handled without oversized shifts. Tininess is
checked before rounding, overflow after rounding. Exact tiny results trap with
UFM enabled, while masked inexact tiny results set UF and NX, including when
rounding up to minimum normal.

Finite products, zeros and infinities use the XOR of operand signs. Zero times
infinity raises NV with the documented numeric-invalid NaN default; infinity
times a nonzero finite value or infinity returns signed infinity. Multiplication
never raises DZ. The shared Meiko rules classify NaNs before numeric cases.

microSPARC-II section 4.1 says FMULs/d complete in hardware even when the fast
multiplier restarts work in the Meiko unit. Helios models the resulting bits and
exceptions, without pipeline timing. FsMULd (single operands to double result),
which has chip-specific unfinished cases, and quad multiplication remain explicit
unsupported host stops; their arithmetic/trap behavior is not implemented yet.

## Square root

FSQRTs (`opf=0x29`) and FSQRTd (`opf=0x2a`) use an exact integer square-root
search with a single round onto the destination grid. Positive finite inputs,
including subnormal values, are normalized to an integer significand and a
power of two; the result is compared against the candidate endpoint by exact
scaled-integer comparisons, so no host floating-point arithmetic or double
rounding is used. Round-to-nearest-even, toward zero and directed rounding are
supported. Exact results clear stale CEXC; inexact results set NX. Negative
finite inputs, including negative infinity, produce the positive canonical
all-fraction-ones quiet NaN with NV. `sqrt(-0)` preserves negative zero,
`sqrt(+0)` is positive zero, and positive infinity is unchanged. Quad square
root remains an explicit host stop.

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
Widening multiplication, quad conversions/operation traps and
unfinished operations remain pending; this is not a complete IEEE arithmetic implementation.

## Validation and firmware

Tests cover every FPU register, raw bit patterns, all branch condition/FCC/annul
combinations, signed displacements, wrapped addresses, masks, exception priority,
delay-slot trap state, MMU permissions/NF, failed doublewords, unsupported MMIO,
SBus asynchronous stores and resets. These are focused regressions, not an FPU
conformance suite.

Conversion tests cover signed integer bounds, fractional truncation, rounding
ties, binade carry, signed zero/infinity, exact and inexact tiny results,
overflow thresholds, Meiko NaNs, register overlap and deferred exceptions.
An additional local check passes 14400 finite conversion/rounding cases against
an independent Python Fraction oracle (seed `0x535335`). It finds adjacent
representable values by binary search over numeric encodings, independently of
the implementation's shift/round algorithm. This supplements the directed CPU
state tests; it is not exhaustive or a comparison against physical hardware.

Comparison tests cover ordered/subnormal/infinite values, signed zeros, NaN
classes and TEM settings, inhibited results, delayed detection, priority,
sequence errors, queue writes/retries, NF, interrupts, delay slots and reset.

Division tests cover both precisions, all rounding modes/signs, destination
aliases, gradual underflow, halfway cases, exact-tiny traps, overflow, NaN
payload/sign rules, all five exceptions, inhibited results and deferred recovery.
Addition/subtraction tests cover destination aliases, cancellation, signed zero,
halfway rounding and adjacent values, discarded operands, subnormal sums,
overflow boundaries, NaN rules and deferred exception recovery.
The independent exact-fraction oracle described in [HACKING.md](../HACKING.md)
passes 208464 guest operations: 48000 divisions, 55168 additions, 55168
subtractions and 50128 multiplications. Seed `0x535335` and 2048 sampled operand pairs per precision and
operation, plus targeted boundary vectors, reproduce the run. The add/sub
samples include nearby magnitudes to expose cancellation. The checker validates
result words, FSR and queue presence; this is not exhaustive or a silicon comparison.

Multiplication regressions cover exact products, retained low bits, normal and
subnormal ties, extreme exponents, overflow, NaNs, enabled exceptions and queue
recovery. Original OBP now executes single/double division, addition, subtraction,
multiplication, comparisons, branches and twelve deferred exception/queue cycles.
Its 30000000-step probe reports a missing processor-timer interrupt on serial,
resets and continues firmware initialization. Later chip-revision, auxiliary and floppy-controller work now reaches a serial
recovery prompt after a device-probe error. Clean POST remains unverified.
[Firmware records](firmware.md) retain the bounded checkpoint and exact limits.
