# Interrupts, counters and guest time

Helios implements the NCR89C105-compatible system interrupt controller and
counter timers used in the sun4m map. Sources are the NCR chip specification,
SPARC V8 and the pinned QEMU/MAME revisions in [references.md](references.md).
The synthetic [timer PROM](../examples/timer.s) demonstrates the processor
counter interrupt and return to interrupted code. Original OBP now passes its
processor counter diagnostic; see [firmware.md](firmware.md) for the bounded
boot evidence and remaining gaps.

The default SS-5 and SS-20/Axil machines install only the processor-0 bank.
SS-20/Axil can install a second bank for hardware processor 2, which is the
second MBus module. Hardware processor numbers are not compact vector indexes:
processor 2 uses timer/interrupt offsets `+0x2000`, and its timer mode bit is
bit 2. On SS-20/Axil, banks 1 and 3 remain absent zero windows. Production
execution still uses processor 0; the second bank is addressable state for
later SMP work. Standalone timer/controller helpers accept hardware numbers
0–3; the machine installation API enforces the supported profile population.

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
16 bytes and the system aperture is 20 bytes. In the SS-20/Axil high map the
processor-0 timer/interrupt pages are at `0xff1300000`/`0xff1400000`; processor
2 uses `0xff1302000`/`0xff1402000`; system timer/interrupt pages are
`0xff1310000`/`0xff1410000`. The SS-5 low map has only the processor-0 bank;
the high-map absent processor numbers 1 and 3 remain zero windows.

| Address | Read | Write |
| --- | --- | --- |
| `0x71e00000` | CPU pending: soft levels in bits 31:17, hard levels in 15:1 | Ignored |
| `0x71e00004` | Zero | Write-one clear software pending bits; bit 15 acknowledges module error (IC) |
| `0x71e00008` | Zero | Write-one set software pending bits |
| `0x71e0000c` | Zero (reserved) | Ignored |
| `0x71e10000` | System source pending | Ignored |
| `0x71e10004` | System source mask | Ignored |
| `0x71e10008` | Zero | Write-one clear mask (enable source) |
| `0x71e1000c` | Zero | Write-one set mask (disable source) |
| `0x71e10010` | Zero (reserved on NCR89C105) | Ignored |

System mask bits are `0xf05dff80`; other bits read zero and writes have no effect.
Reset clears pending state and sets all implemented mask bits. A mask changes
delivery without erasing a held level-sensitive source. On SS-5 the
module-error latch is acknowledged by IC or its source-mask-set bit; the MBus
source/capture rules are described below. Pending reads show masked requests
too.

Each processor bank owns independent software-pending bits, processor-timer
pending, and a level-15 broadcast latch. A write to a bank clear/set register
affects that addressed bank regardless of which processor issued it; an absent
bank address reads zero and discards the narrow or word access. The system
mask and source state are shared by the banks. On SS-20/Axil, the Current
Interrupt Target register at system interrupt `+0x10` uses TARGET bits 1:0;
bits 31:2 are reserved and ignored on writes, so writing `0xffff_fffe` selects
processor 2 and reads back 2. On SS-5, the NCR reserved target address reads
zero and ignores writes. Changing an MBus target while a source is asserted
moves delivery; a valid but absent target leaves the source pending and delivers
it to no CPU. The processor timer and directed software bits are local and
remain pending independently of the target. Mask All (bit 31) gates delivery
from every bank, including software and processor-timer requests, while pending
state remains readable. This follows NCR89C105's "all interrupts" wording;
QEMU lets some software requests bypass the master disable. The difference is
deliberate and retained as the documented functional policy. QEMU's larger
writable target field is an implementation extension, not the Sun-4M board
register.
The processor timer contributes hard level 14 without a separate source-mask
bit. Mask All (bit 31) gates delivery of both timers and software requests.
Pending software, processor-timer and level-15 state is independent of MBus
processor arbitration: an interrupt never sets an AER processor-enable bit,
and a processor-2 bank may accumulate pending state while its module is
arbitration-disabled. See [machine profiles](machines.md#processor-arbitration-and-initiator-identity).

The software request for level n occupies bit `16+n`. It persists until cleared
through the addressed CPU bank's clear alias; clearing soft requests does not
acknowledge timer hardware. On SS-20/Axil, level-15 source status, per-CPU
capture and per-CPU acknowledgement are separate. Source bit 29 is the held
M-to-S write-buffer error; source bit 30 is the independent ModuleError input.
A source assertion is captured only in banks not disabled by that source's mask
at the instant of assertion; a masked event is not captured retroactively when
the mask is later cleared. Deasserting a source clears its system status but
does not erase a processor's already-captured level-15 bit. Each processor
clears only its own captured level-15 indication with CPU clear-pending bit 15;
the last processor acknowledgement does not fabricate deassertion of either
source. An AFSR write acknowledges the M-to-S source and releases it without
clearing CPU-0 or CPU-2's already-captured pending bit. System source-mask
writes do not acknowledge another processor's capture or either held source.
SS-5 retains the NCR single-CPU rule, where clearing pending bit 15 or setting
the source mask is also the local acknowledgement; the integrated module-error
producer remains a pulse. See [fault capture](fault-capture.md). Mask All still
gates delivery of a captured level while preserving its pending state.

| System source bit | Source | CPU level |
| --- | --- | --- |
| 7–13 | SBus levels 1–7 | 2, 3, 5, 7, 9, 11, 13 |
| 14, 15 | Keyboard/mouse, serial | 12 |
| 16 | Ethernet | 6 |
| 18 | SCSI | 4 |
| 19 | System counter | 10 |
| 22 | Floppy | 11 |
| 29 | M-to-S write-buffer error (SS-20/Axil) | 15 |
| 30 | Module error latch / independent external input | 15 |

The level-sensitive inputs are available through `InterruptController` for
wiring and tests; module errors use a separate latch. Sources sharing a level
remain independent. The machine
wires the two counter timers and asynchronous CPU SBus store errors; serial
remains polled. SuperSPARC-only error masks are retained, but their pending
bits stay clear. No absent device is fabricated by unmasking it.

The system controller's source/mask state is shared, while every installed
CPU bank has its own pending register and set/clear aliases. The bus records
the bank at the hardware processor address rather than treating a dense module
list index as the CPU number. `SystemBus::install_processor_banks` installs
both the interrupt and timer bank for a processor; default machines install
processor 0 only.

## Counter timers

All counters use one 500 ns tick derived from the machine's shared elapsed-time
input. The system counter is one device; every installed processor has an
independent count, limit, run/mode state and pending latch. A matching
increment sets the bank's pending latch, reloads the count to `0x200` and
requests level 10 (system) or level 14 (that processor). Count reads include
pending in bit 31 and do not acknowledge it. Multiple expirations while
pending coalesce into the same asserted line. A limit read or configuration
change on one processor bank does not clear another bank's pending state.

| Offset | Processor-0 at `0x71d00000`; processor 2 adds `0x2000` | System at `0x71d10000` |
| --- | --- | --- |
| `+0` | Limit: read acknowledges; write loads and resets | Same |
| `+4` | Count and pending, read only | Same |
| `+8` | Non-resetting limit write; reads zero | Same |
| `+0xc` | User Run bit, retained but no effect in counter mode | Reserved, reads zero |
| `+0x10` | Outside aperture | Configuration bit 0 selects processor user mode |

Limit reads return the limit and the sampled pending bit 31, then clear pending
without resetting the running count. Limit writes mask to bits 30:9, reset the count to `0x200` and
clear pending. Non-resetting writes retain both count and pending. If the new
limit is behind the current count, matching waits until wraparound; it does not
immediately expire or restart. Writes to the read-only counter are ignored.

A zero limit free-runs without interrupting. Overflow reloads one, skipping
zero, as specified by Sun-4M section 5.3.2. For nonzero limit N in tick units, repeated matches normally occur
every N−1 ticks because a match reloads one. Helios compares after increment:
the edge case N=1 therefore waits 2^22−1 ticks after reload. Limit writes
do not manufacture an immediate match. This edge rule and clearing pending on
a resetting write are explicit functional choices; hardware boundary validation
remains desirable.

Both counts, limits, pending latches, configuration and User Run reset to zero.
Reserved configuration bits are ignored. Atomics and doubleword accesses in
counter mode are rejected before any destructive limit read.

## Processor user timer

Configuration bit 0 at `0x71d10010` selects processor 0's user timer; bit 2
selects processor 2's user timer. Each populated hardware processor uses its
own bit, and the system counter always stays in counter mode. The same mode
write leaves an unchanged bank's run/count state alone. The user count occupies bits 62:9
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
aligned LDD/STD and LDDA/STDA at each installed processor timer base only while
that bank's user mode is selected; the processor-2 base is `+0x2000`.
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

The machine services one pending Ethernet DMA transfer, samples interrupt lines,
executes one CPU outcome, then advances devices by a base `STEP_NS = 100` ns
plus physical PROM read service time. NCR89C105 table 6-10 specifies six SBus
clocks per byte of the byte-wide PROM. The SS5/85 SBus runs at 21.25 MHz
(CPU/4), so a word fetch adds 24 clocks, about 1129.41 ns. Instruction and data
reads both count, including the reset boot alias; RAM execution adds no PROM
wait. Fractional nanoseconds carry across accesses. Direct bus inspection does
not advance time. An error or unsupported stop
does not advance time. A timer expiring during advancement is visible at the
next boundary. Host wall time, terminal I/O and scheduling have no influence.
The base quantum and serialized DMA are functional conventions. This is not a
cycle-accurate pipeline, ordinary cache, SBus arbitration or burst timing model.

`Machine::advance_ns` and `advance_seconds` advance all devices between CPU steps.
They do not execute the CPU or auto-enter handlers. Every installed processor
timer bank receives the same elapsed ticks and shared phase; there is no
per-CPU host clock or double advancement. Timer tick fractions and RTC second
fractions accumulate. Whole-second advancement handles `u64::MAX` without
multiplying into a `u64` overflow; timer arithmetic skips whole periods with
bounded work. The RTC retains its separate oscillator-stop and R/W latches.

Machine reset clears timers, timer phase and interrupt state while retaining
NVRAM/RTC, its fractional second, RAM/ROM and completed serial output. Standalone
device APIs let tests control time directly. Callers stepping `Cpu` directly
must provide their own clock advancement and interrupt wiring.

## Reference differences and verification

Sun-4M System Architecture (950-1373-01, July 19 1991, revision 50), section
5.3.2, explicitly says zero-limit free-running counters do not interrupt and
overflow reloads 500 ns. Its figure shows the limit flag in both registers.
This conflicts with NCR89C105's zero-limit-overflow interrupt prose and its
limit-register diagram. Helios now follows Sun on these points; the original
OBP processor-timer diagnostic also requires the limit flag on the acknowledging
read. The earlier NCR-based model produced a spurious level 10 interrupt once
PROM service time was added. QEMU likewise suppresses zero-limit interrupts,
but resets Run to one; Helios retains NCR's zero counter/Run reset baseline.
QEMU accepts a larger writable multiprocessor target field everywhere. Helios
instead follows the selected board: NCR reserves that address on SS-5, while
Sun-4M defines two TARGET bits on the MBus profiles. Helios applies NCR's Mask
All wording to software requests as well; QEMU lets them bypass that bit. The wording is not explicit
about this exception, so the software-mask choice needs hardware confirmation.
MAME's inspected Sun timer wiring is for sun4/sun4c and is not a sun4m oracle.
QEMU also permits doubleword MMIO in counter mode and uses its countdown limit
machinery for user mode. Helios follows NCR's user-register-only burst rule and
the full 54-bit count range. QEMU resets the count when entering user mode;
neither it nor the chip text establishes Helios's return-to-counter choices.

Regressions cover every ET/PIL/level combination, entry without a fetch,
branch-delay return, held sources, mask/set/clear semantics, exact tick and
overflow boundaries, destructive reads and a scalar counter oracle. Slice 2A
adds hardware-processor-addressed software banks, Current Interrupt Target
routing (including absent targets), per-processor level-15 acknowledgement,
independent timer limits/pending/mode bits 0 and 2, and one shared elapsed-time
advance. Large and partitioned time advances agree. `timer-demo` runs a
synthetic reset PROM and checks one entry per timer, acknowledgement and resumed
work. User-timer tests cover both word carries, overflow clear, start/stop
phase, mode changes with pending interrupts, forbidden accesses, translated
write protection and alternate-space doubleword transfers on both populated
banks. Initial original OBP and OpenBIOS probes are recorded separately.
Single-CPU original-OBP and Solaris boot evidence is recorded in
[machines.md](machines.md). No differential reference execution, SMP scheduler
or two-CPU firmware startup/prompt has been demonstrated.
