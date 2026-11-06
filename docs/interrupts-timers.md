# Interrupts, counters and guest time

Helios implements the uniprocessor NCR89C105 control registers used in the
SS-5/sun4m map. Sources are the NCR chip specification, SPARC V8 and the pinned
QEMU/MAME revisions in [references.md](references.md). The synthetic
[timer PROM](../examples/timer.s) demonstrates both counter interrupts and return
to interrupted code. Initial firmware probes stop before a prompt; see
[firmware.md](firmware.md). No firmware compatibility is established.

## CPU delivery

`Cpu::step_with_interrupts(bus, pending)` samples a 16-bit set of asserted levels
before fetching the next instruction. Bit zero is ignored. With PSR.ET=1, the
highest asserted level is accepted when it exceeds PSR.PIL, or when it is 15.
ET=0 blocks every level, including 15, without entering CPU error state. An
already latched CPU error takes precedence and cannot be cleared by an interrupt.

Entry uses TT=`0x10 | level`, preserves PIL/ICC/EF, saves old S into PS, sets S,
clears ET and decrements CWP without checking WIM. The interrupted PC/nPC are
saved in `%l1`/`%l2`. `JMPL %l1, %g0; RETT %l2` resumes the interrupted pair,
including a pending branch target. No interrupted instruction or first handler
instruction executes in the entry step. A source left asserted can retrigger
after RETT enables traps.

This is boundary sampling in a synchronous interpreter, following the V8
execution model. There are no outstanding pipeline or deferred exceptions to
arbitrate against. We do not fetch/execute the next instruction speculatively
to look for a higher-priority exception before taking an interrupt. Full
pipeline exception arbitration and interrupt latency are outside this model.

`Step::InterruptTaken` carries the interrupted PC/nPC, level and vector;
`Step::TrapTaken` continues to carry a synchronous fault. `Cpu::step` supplies no
interrupt lines. `Machine::step` supplies the wired controller outputs. The CLI
counts instructions, synchronous traps and interrupt entries separately.

## Interrupt controller

These registers accept aligned 32-bit loads/stores only. The CPU aperture is
16 bytes and the system aperture is 20 bytes; other CPU banks are not installed.

| Address | Read | Write |
| --- | --- | --- |
| `0x71e00000` | CPU pending: soft levels in bits 31:17, hard levels in 15:1 | Ignored |
| `0x71e00004` | Zero | Write-one clear software pending bits |
| `0x71e00008` | Zero | Write-one set software pending bits |
| `0x71e0000c` | Zero (reserved) | Ignored |
| `0x71e10000` | System source pending | Ignored |
| `0x71e10004` | System source mask | Ignored |
| `0x71e10008` | Zero | Write-one clear mask (enable source) |
| `0x71e1000c` | Zero | Write-one set mask (disable source) |
| `0x71e10010` | Zero (reserved on NCR89C105) | Ignored |

System mask bits are `0xf05dff80`; other bits read zero and writes have no effect.
Reset clears pending state and sets all implemented mask bits. A mask changes
delivery without erasing a held source. Pending reads show masked requests too.
The processor timer contributes hard level 14 without a separate source-mask
bit. Mask All (bit 31) gates delivery of both timers and software requests.

The software request for level n occupies bit `16+n`. It persists until cleared
through the CPU clear alias; clearing soft requests does not acknowledge timer
hardware. The chip's asynchronous-error/IC latch is not implemented, and there
is no generated hard level-15 error source yet. Software level 15 is supported.
Multiprocessor interrupt targeting is not part of this SS-5 profile.

| System source bit | Source | CPU level |
| --- | --- | --- |
| 7–13 | SBus levels 1–7 | 2, 3, 5, 7, 9, 11, 13 |
| 14, 15 | Keyboard/mouse, serial | 12 |
| 16 | Ethernet | 6 |
| 18 | SCSI | 4 |
| 19 | System counter | 10 |
| 22 | Floppy | 11 |

These level-sensitive inputs are available through `InterruptController` for
wiring and tests. Sources sharing a level remain independent. The machine
currently wires only the two counter timers; serial remains polled. Mask bits
for unimplemented error and SuperSPARC-only sources are retained, but their
pending bits stay clear. No absent device is fabricated by unmasking it.

## Counter timers

Both counters use a 500 ns tick. The current 22-bit count and limit occupy bits
30:9; low bits read zero. A matching increment sets the pending latch, reloads
the count to `0x200` and requests level 10 (system) or 14 (processor). Count reads
include pending in bit 31 and do not acknowledge it. Multiple expirations while
pending coalesce into the same asserted line.

| Offset | Processor at `0x71d00000` | System at `0x71d10000` |
| --- | --- | --- |
| `+0` | Limit: read acknowledges; write loads and resets | Same |
| `+4` | Count and pending, read only | Same |
| `+8` | Non-resetting limit write; reads zero | Same |
| `+0xc` | User Run bit, retained but no effect in counter mode | Reserved, reads zero |
| `+0x10` | Outside aperture | Configuration bit 0 selects processor user mode |

Limit reads clear pending without resetting the running count, and return only
the limit field. Limit writes mask to bits 30:9, reset the count to `0x200` and
clear pending. Non-resetting writes retain both count and pending. If the new
limit is behind the current count, matching waits until wraparound; it does not
immediately expire or restart. Writes to the read-only counter are ignored.

A zero limit matches at 22-bit overflow and still interrupts, roughly every
two seconds. For nonzero limit N in tick units, repeated matches normally occur
every N−1 ticks because a match reloads one. Helios compares after increment:
the edge case N=1 therefore waits a full 22-bit wrap after reload. Limit writes
do not manufacture an immediate match. This edge rule and clearing pending on
a resetting write are explicit functional choices; hardware boundary validation
remains desirable.

Both counts, limits, pending latches, configuration and User Run reset to zero.
Reserved configuration bits are ignored. Atomics and doubleword accesses in
counter mode are rejected before any destructive limit read.

## Processor user timer

Configuration bit 0 at `0x71d10010` selects the processor's user timer. The
system counter always stays in counter mode. The user count occupies bits 62:9
of a 64-bit register at `0x71d00000`; bits 8:0 read zero and ignore writes.
The most significant word is at `+0`, the least significant word at `+4`.
The low word's bit 31 is count data, not the counter-mode pending flag.

The 54-bit count increments once per 500 ns tick when User Run (`+0xc`, bit 0)
is set. Clearing Run freezes the count without changing it. Start/stop and
count writes retain the shared tick phase: starting 1 ns before the next tick
allows an increment 1 ns later. The system counter continues independently.
Overflow wraps the count to zero and latches bit 63 until any count write;
reads never clear it. Writes mask off bit 63 and clear overflow even when the
written count bits are unchanged. There is no processor interrupt in this mode,
including on overflow. No time is derived from the host clock.

Word accesses read/write either half, preserving the other half on writes.
NCR recommends 64-bit access for consistency and explicitly permits two-word
SBus bursts only to this register in user mode. `SystemBus` therefore accepts
aligned LDD/STD and LDDA/STDA at `0x71d00000` only while user mode is selected.
Both words are sampled or committed together. Other doubleword device addresses,
byte/halfword accesses, partial masked writes and atomics still stop without
device effects. MMU protection checks precede the whole transfer, allowing a
read-only user mapping without permitting the process to reset its clock.

The chip text does not specify transfer state across mode changes. Helios uses
a deterministic initialization rule: a changed mode clears the processor
count/overflow/pending state, retains Run and the counter-mode limit latch, and
leaves the system counter and clock phase alone. Rewriting the same mode does
nothing to the count or flags. The `+8` non-resetting limit alias still updates
the counter-mode limit latch in user mode, without altering user time. These
transition/alias choices need hardware confirmation; they are not inferred from
successful firmware execution. OBP explicitly clears both count halves and
sets Run after selecting user mode, so its probe does not establish those edges.

Power-on and software reset clear mode, Run, both counts and the overflow flag.
Large time advances use bounded integer arithmetic, including multiple wraps
and `u64::MAX` seconds.

## Time and ownership

The machine samples interrupt lines, executes one CPU outcome, then advances
the timers and RTC by `STEP_NS = 100` ns. Completed instructions and successful
trap/interrupt entries consume the same quantum. An error or unsupported stop
does not advance time. A timer expiring during advancement is visible at the
next boundary. Host wall time, terminal I/O and scheduling have no influence.
This quantum is a functional convention, not a microSPARC frequency or cycle
accuracy claim.

`Machine::advance_ns` and `advance_seconds` advance all devices between CPU steps.
They do not execute the CPU or auto-enter handlers. Timer tick fractions and
RTC second fractions accumulate. Whole-second advancement handles `u64::MAX`
without multiplying into a `u64` overflow; timer arithmetic skips whole periods
with bounded work. The RTC retains its separate oscillator-stop and R/W latches.

Machine reset clears timers, timer phase and interrupt state while retaining
NVRAM/RTC, its fractional second, RAM/ROM and completed serial output. Standalone
device APIs let tests control time directly. Callers stepping `Cpu` directly
must provide their own clock advancement and interrupt wiring.

## Reference differences and verification

The NCR specification and generic QEMU sun4m model differ. NCR documents
interrupts on zero-limit overflow and zeroed counter/Run reset state; Helios
follows those descriptions. QEMU suppresses zero-limit interrupts and resets
Run to one. QEMU also implements a multiprocessor target register, while NCR
reserves that address. Helios applies NCR's Mask All wording to software
requests as well; QEMU lets them bypass that bit. The wording is not explicit
about this exception, so the software-mask choice needs hardware confirmation.
MAME's inspected Sun timer wiring is for sun4/sun4c and is not a sun4m oracle.
QEMU also permits doubleword MMIO in counter mode and uses its countdown limit
machinery for user mode. Helios follows NCR's user-register-only burst rule and
the full 54-bit count range. QEMU resets the count when entering user mode;
neither it nor the chip text establishes Helios's return-to-counter choices.

Regressions cover every ET/PIL/level combination, entry without a fetch,
branch-delay return, held sources, mask/set/clear semantics, exact tick and
overflow boundaries, destructive reads and a scalar counter oracle. Large and
partitioned time advances agree. `timer-demo` runs a synthetic reset PROM and
checks one entry per timer, acknowledgement and resumed work. User-timer tests
cover both word carries, overflow clear, start/stop phase, mode changes with
pending interrupts, forbidden accesses, translated write protection and a
synthetic PROM using alternate-space doubleword transfers. Initial original
OBP and OpenBIOS probes are recorded separately. No differential reference
execution or real firmware prompt has been demonstrated.
