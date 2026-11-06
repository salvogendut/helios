# microSPARC-II floating-point transfers

Helios implements the initial FPU state and transfer subset: 32 word registers,
LDF/LDDF/LDFSR, STF/STDF/STFSR, FMOVs/FNEGs/FABSs and all FBfcc conditions.
Disabled-FPU traps and the empty-queue STDFQ sequence error are implemented.
Comparisons, conversions, arithmetic and deferred exception queues remain
explicit emulator stops when the FPU is enabled.

The primary sources are SPARC V8 sections 4.4, 5.1.7 and 7, appendices B.2,
B.5, B.22 and B.34, and the microSPARC-II User's Manual sections 4.3–4.5,
especially table 6. [References](references.md) records the manual and pinned
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
| 31:30 | RD | Stored rounding direction; arithmetic is pending |
| 27:23 | TEM | Stored IEEE trap enables; arithmetic is pending |
| 22 | NS | Always zero on microSPARC-II |
| 19:17 | VER | Always 4 |
| 16:14 | FTT | Retained across LDFSR/STFSR; set to 4 by empty STDFQ |
| 13 | QNE | Zero in this subset; no queued FPop is executed |
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
manual's spacing after LDFSR and future FPop2 instructions; pipeline interlocks
and timing are not modeled.

Recognized floating-point instructions check EF before data access or branch
annul. EF=0 raises `fp_disabled` (TT `0x04`), including FPop1/FPop2 operations
that are otherwise unimplemented. ET=1 enters the existing precise trap path,
preserving faulting PC/nPC including a pending branch target. ET=0 latches CPU
error state. Instruction-fetch faults still precede instruction execution.

STDFQ checks supervisor privilege, EF, then eight-byte alignment. Because no
queued operation exists yet, an aligned enabled STDFQ sets FTT=4 and raises an
immediate `fp_exception` (TT `0x08`) before translation or a memory write.
The FPU remains in its execution state and QNE remains zero, as microSPARC-II
specifies for an empty queue. Ordinary transfers and status reads do not clear
this FTT; a completed nontrapping FPop does.

The real microSPARC-II has a three-entry deferred-trap queue and execution,
exception-pending and exception states. Those transitions, nonempty STDFQ,
IEEE exceptions, quad-operation traps and unfinished operations are pending.
With EF=1, unsupported FPop execution returns `UnsupportedInstruction` without
changing FPU state, taking a guest trap or latching CPU error. No host arithmetic
result or fabricated empty-queue success substitutes for this missing behavior.

## Validation and firmware

Tests cover every FPU register, raw bit patterns, all branch condition/FCC/annul
combinations, signed displacements, wrapped addresses, masks, exception priority,
delay-slot trap state, MMU permissions/NF, failed doublewords, unsupported MMIO,
SBus asynchronous stores and resets. These are focused regressions, not an FPU
conformance suite.

Original OBP now completes 66 LDDF and 66 STDF instructions, reaching the first
unsupported FCMPs at `0x70002728`. It has no serial output or firmware prompt.
[Firmware records](firmware.md) retain the bounded checkpoint and exact limits.
