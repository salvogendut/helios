# Console and NVRAM devices

`Machine::sun4m` installs the following physical byte apertures. These are a
functional polling subset, tested with a synthetic PROM, not a firmware-boot
compatibility claim. The machine also installs word-accessed
[interrupt control and timers](interrupts-timers.md), and [reset/status](reset.md).
`Machine::bare_metal` continues to install RAM only.
Sources and reference revisions are listed in [references.md](references.md).

| Physical address | Device/register |
| --- | --- |
| `0x71000000`, `0x71000002` | Keyboard/mouse SCC channel B control, data (mouse) |
| `0x71000004`, `0x71000006` | Keyboard/mouse SCC channel A control, data (keyboard) |
| `0x71100000`, `0x71100002` | SCC channel B control, data |
| `0x71100004`, `0x71100006` | SCC channel A control, data (TTYA) |
| `0x71200000`–`0x71201fff` | M48T08 NVRAM/RTC, 8192 bytes |
| `0x71201fd8`–`0x71201fe7` | First 16 IDPROM bytes, including checksum |
| `0x71201ff8`–`0x71201fff` | RTC control and seven calendar registers |

The SCC ignores address bit zero within its eight-byte aperture. Wider, masked,
doubleword and atomic accesses to these byte devices are unsupported. `SystemBus`
routes plain bytes directly; a status read cannot consume a neighboring data
register. Mapping RAM/ROM or loading a raw image over a device is rejected.

## NVRAM, identity and clock

Fresh NVRAM has zeroed configuration SRAM and an IDPROM with format 1, SS-5
machine type `0x80`, Sun-style MAC `08:00:20:c0:ff:ee`, manufacture
date zero and serial `c0ffee` (hostid `80c0ffee`). Byte 15 makes the XOR of the
first 16 bytes zero. `Nvram::new` accepts another MAC and three-byte serial.
Imported images and guest writes are retained without automatic checksum
repair. No Sun environment defaults or OpenBIOS configuration partitions are
fabricated; firmware-specific initialization remains separate work.

The RTC defaults to 2000-01-01 00:00:00, weekday 7. Its two-digit year is `0x32`
using the SS-5 software origin of 1968. The chip stores no century. Registers
are BCD seconds, minutes, hours, weekday, date, month and year, in that order.
The calendar uses 24-hour time, weekday 1–7, a leap day every fourth encoded
year and year rollover from 99 to 00.

Control bit 6 (R) freezes visible transfer registers while internal time keeps
advancing. Bit 7 (W) similarly holds the transfer registers for editing;
clearing W loads all fields into the counter together. The seconds register's
bit 7 stops the oscillator. Invalid BCD/calendar fields or unsupported test
bits stop before a W-clear commits. Frequency calibration is unsupported.

The machine supplies elapsed time from its 100 ns base step quantum plus
documented physical PROM read wait states
and `advance_ns`/`advance_seconds` APIs. The standalone `Nvram` device still uses
explicit `advance_seconds` calls. Host wall time is never consulted. Very large
advances use bounded calendar work. Machine reset retains SRAM, clock state,
latches and its fraction of an RTC second. See [guest time](interrupts-timers.md).

`Nvram::from_image` requires exactly 8192 bytes and valid supported clock fields.
`Nvram::image` exports raw SRAM/register contents. With R/W clear, this includes
the current counter; with a latch held, it includes the held/staged fields.
This file is not a machine snapshot: it cannot preserve both a latched register
value and a separately advancing hidden counter. A partially edited invalid
clock can be exported but will be rejected on import until repaired.

## Z8530 polling subset

There are two independent SCC instances: console TTYA/TTYB and keyboard/mouse.
The latter exposes the same polled register and byte-queue behavior; keyboard
commands, mouse packets and host input translation are pending. The CLI connects
only TTYA. Installing the second SCC lets OBP initialize its registers without
an artificial unmapped-address trap.

Both channels have separate receive/transmit state and a shared control-register
pointer. WR2 (interrupt vector) and WR9 (master/reset control) are also shared.
A control write selects the next register, including WR0's point-high command;
the next control access clears the pointer. Direct data access leaves it alone.
RR8/WR8 also access the data port. Legacy read aliases follow the Z8530 map.

The model accepts asynchronous initialization, character lengths of 5–8 bits,
receiver/transmitter enable, baud-generator register latches, local loopback and
channel/hardware resets. RR0 reports receive availability and transmit-buffer
empty; RR1 reports all-sent. DCD and CTS stay asserted for the connected virtual
endpoint. RR3 reports no pending interrupts; RR2B supplies the no-interrupt code.
An empty receive-data read returns zero as a deterministic emulator choice.
WR14 accepts DPLL clock-source and encoding configuration while the DPLL is
disabled, including OBP's `0x82` write. Only Enter Search Mode enables the DPLL;
that command remains unsupported. This follows Zilog section 5.2.16.

`Serial::receive(channel, byte)` accepts a byte only when reception is enabled
and its three-byte FIFO has room. A false result means the caller retains the
byte for later delivery. Data is masked to the configured character length.
A disabled transmitter holds one byte until enabled. Enabled transmission
completes immediately into the host output queue, or into the receiver in local
loopback. Bytes offered to a disabled loopback receiver are discarded.

Each channel's completed-output queue is bounded at 4096 bytes. `take_output`
drains it. Full output, a full loopback receiver or a second write to a disabled
transmitter's occupied buffer returns `WouldBlock` before mutation. This is a
host stop: it neither fabricates a guest hardware exception nor advances the
faulting instruction. After draining the endpoint, callers can retry. The host
queue is not an emulated UART FIFO and does not affect RR0's TX-empty status.
Reset clears receive and pending transmit state but preserves completed output.

Interrupt sources, DMA, synchronous/SDLC operation, transmit break, DPLL
operation, auto echo and enhanced FIFO modes are unsupported and stop on their
enabling commands. Framing/parity errors, line bit timing, modem transitions and
receive overrun from an uncontrolled external source are not modeled. Byte
delivery uses backpressure, not host scheduling. Accepted baud/parity/stop-bit
settings do not introduce delays or generate line-level errors. Keyboard/mouse
serial at `0x71000000` is not installed.

## CLI and validation

`serial-demo` boots [examples/serial.s](../examples/serial.s), verifies IDPROM,
prints `HELIOS`, echoes `!`, and writes a newline. Its host checks the identity
and completion marker after 256 steps. It requires no guest toolchain or ROM.

`boot` and `serial-demo` write TTYA bytes to stdout. Trace and stop reports go to
stderr, so redirecting stdout captures a clean console transcript. `boot
--serial-in FILE` feeds bytes once the guest enables its receiver and as space
becomes available. There is no interactive terminal input yet; channel B is
available through the library. No serial input file is modified.

`boot --nvram FILE` imports raw NVRAM; omitting it creates the deterministic
default. `--nvram-out NEW_FILE` reserves a new file before execution and saves
the resulting image at the step budget or an execution error. Existing files
are never overwritten, including the input image. Invalid inputs are rejected
before creating the output. An interrupted host process is not a save point.

Regressions exercise guest TX/RX, MMU-translated byte accesses, unsupported
widths without destructive reads, backpressure/retry, channel reset, clock
latches and calendar boundaries, identity checksums and CLI file behavior.
CPU interrupt delivery and counter timers have a separate synthetic PROM demo.
Serial interrupt generation remains pending. Original OBP now has a responsive
[serial recovery prompt](firmware.md#serial-ok-prompt); OpenBIOS and OS boot
have not been demonstrated.

## NCR89C105 auxiliary ports

AuxIO1 at physical `0x71900000` accepts plain byte reads and writes. Figure 6-21
of the NCR specification defines writable link-test enable (bit 3), monitor/mouse
mux (bit 2), and front-panel LED (bit 0). Bit 5 reflects the external floppy
high-density input and ignores writes; its default disconnected value is zero.
Reserved bits 7:6 and 4 always read zero. The input can be supplied through
`Auxiliary::set_floppy_high_density` and survives reset; output latches reset zero.

Each write with bit 1 set emits a terminal-count pulse event; zero emits none.
The bit always reads zero. `take_terminal_count_pulses` drains these events for a
future floppy data-transfer engine. The hardware's four-SBus-clock pulse width is not
modeled. Reads do not consume pulses, and writes replace the output bits.

Wrong widths, atomics, masked stores and image loads stop before device effects.
This implements the documented base registers only; address mirrors,
modem/configuration registers, floppy media transfers and
Ethernet link-test behavior remain pending. Pin-state storage and TC events do
not establish functioning floppy or Ethernet devices.

AuxIO2 at `0x71910000` also requires byte accesses. Its otherwise unused bits
3:2 retain writes. The power-failure input is not asserted in this model; bit 5
therefore reads zero, and writing the clear bit (1) has no pending failure to
acknowledge. Reserved bits 7:6 and 4 read zero. A write requesting power-off
(bit 0) stops explicitly before changing device state: powered-down machine
execution is not implemented. External power-failure detection and its
configuration/interrupt wiring remain pending. Reset clears the scratch bits.

## NCR82077 floppy initialization and positioning

The byte-only controller aperture at `0x71400000` exposes DOR (`+2`),
MSR/DSR (`+4`), FIFO (`+5`) and DIR/CCR (`+7`). System reset holds DOR.nRESET
low; writing it high releases the core. NCR-specific DOR bits control eject,
density, motor and interrupt gating; reserved bits 5/1 read zero by convention.
Wider transfers throughout the aperture retain the NCR SBus-error behavior.
Unimplemented register offsets/mirrors and masked/atomic accesses stop explicitly.

MSR distinguishes reset, command-ready, parameter collection, result-read and
per-drive busy state. FIFO reads/writes in the wrong direction return retryable
backpressure before modifying state. Implemented commands are SPECIFY, SENSE
DRIVE STATUS, SENSE INTERRUPT STATUS, VERSION, CONFIGURE, DUMPREG,
PERPENDICULAR MODE, LOCK/UNLOCK, SEEK and RECALIBRATE. Unsupported commands,
including data transfers and unclassified invalid encodings, stop explicitly.
The model stores configuration without claiming a working sector FIFO engine.
VERSION returns `0x90`; non-interrupt commands do not invent completion IRQs.

Reset clears command/results, movement and pending interrupts. SPECIFY survives
pin reset. LOCK preserves EFIFO/FIFOTHR/PRETRK across DOR/DSR reset; those fields
return to defaults when unlocked, while EIS/POLL retain their configuration.
Software reset clears perpendicular WGATE/GAP and preserves its drive bits;
pin reset clears LOCK and perpendicular configuration. DSR reset self-clears;
DSR power-down is unsupported and stops before changes.

Reset polling produces one interrupt and four SENSE INTERRUPT results (`c0`
through `c3`, PCN zero). A functional deadline of 250 microseconds at 1 Mbit/s
scales with the data-rate selection. Intel documents the CONFIGURE suppression
window; using its endpoint as the modeled deadline is a scheduling convention,
not measured NCR timing. Entering command/result processing pauses polling;
CONFIGURE.POLL disables it. DOR.DMAGATE gates the source, connected to NCR
bit 22 / CPU level 11. SENSE INTERRUPT acknowledges pending completion state.

There are **no attached floppy drives or images**. External TRK0, write-protect
and disk-change assertions are inactive. ST3 includes fixed bits 5/3 and the
requested unit/head; otherwise tri-stated DIR bits deterministically read zero.
SEEK advances the internal cylinder after a delay derived from SPECIFY.SRT and
the data rate; no physical position feedback is required. RECALIBRATE fails with
EC/SE and abnormal status `0x70 | unit` because TRK0 never asserts. Intel section
5.2.2 says 79 pulses while the ST0 table says 80; the model uses 79 and does not
model individual pulse edges. PCN is cleared at recalibration start and seek
position is committed on completion. Busy persists until SENSE INTERRUPT.
Concurrent completions within one host advance are queued in unit order.

No motor/rotation/index timing, media read/write, DMA/PIO sector transfer,
relative seek, terminal-count transfer termination or power-down is implemented.
AuxIO1's terminal-count events remain available for the future media engine.
These limits do not prevent OBP from initializing the controller, observing
recalibration failure, and continuing to its serial monitor.

## SS-5 AFX slot-presence input

The board-level presence byte at `0x6e000000` reads zero for the selected SBus
CGsix profile: no AFX card is attached. Its presence port exists independently
of a framebuffer. Original OBP's `afx-slot-empty?` reads this byte and tests
bit 0; the pinned QEMU SS-5 map independently identifies the same address.
The microSPARC-II manual appendix A describes the separate static PRES-L pin,
and the SPARCengine 5 manual distinguishes AFX and SBus card configurations.
The board's exact decode, other bits, write behavior and wider accesses have
not been established from schematics; only the observed byte read is modeled.
Writes and wider accesses stop explicitly. No TCX framebuffer or register RAM
is installed. RAM/ROM mappings cannot overlap this board input.

## NCR89C100 SCSI DMA initialization

The onboard MACIO identifies itself with the read-only word `0xfe810103` at
`0x78000000`. The ID also permits byte and halfword reads (NCR chip-level
map table 2-7, footnote 3), with normal big-endian lanes. Its DMA2 SCSI registers at `0x78400000` implement the control/status,
32-bit address and 24-bit byte-count registers. The hardwired DMA revision is
`0xa`; reserved and read-only fields cannot be changed by stores. Reset remains
asserted until software clears it and holds the interface idle. Address/count
reset values are unspecified by the manual; Helios chooses zero.

The current/next address and byte-count latches support the documented initial
chaining sequence. Loading the first next address transfers it and the next
count into the current latches. Later loads remain pending. Address validity and
DMA readiness are computed from these latches and the enable bit. Enabling DMA
does not fabricate requests or move memory. The SCSI controller's IRQ input is
level-sensitive, gated by D_INT_EN, and wired to NCR source 18 / CPU level 4.

The FIFO is empty until a transfer engine is implemented, so invalidation has
no drain delay. D_INVALIDATE reads zero and clears both address-valid bits in
chaining mode. It does not acknowledge the controller IRQ. D_SLAVE_ERR latches
incorrect slave widths and clears on a write of one; it is not a DMA memory
error and does not assert the DMA IRQ. DMA registers require aligned words;
ordinary narrow/doubleword accesses return SBus error. Atomic, masked and image
operations stop explicitly without changing device state.

Diagnostic FIFO accesses, reserved burst modes and illegal combinations of
count/chaining or transfer-cycle modes stop before mutation.
DMA requests, FIFO/data movement, terminal-count/error generation and the
parallel channel remain pending. The Ethernet channel is described in
[ethernet.md](ethernet.md). NCR's reset and W1C semantics take
precedence over QEMU's generic DMA register array.

## NCR53C9X SCSI controller initialization

The integrated ESP/Fast SCSI core occupies `0x78800000..0x7880003f`, with byte
registers on four-byte boundaries. Implemented behavior includes:

- Separate transfer-count latches and running counter. DMA NOP loads the counter;
  ordinary NOP does not. Zero encodes the maximum count and does not immediately
  set terminal count. A compatibility-mode DMA NOP clears the disabled upper
  counter byte in this functional model; no transfer engine decrements it yet.
- A 16-byte FIFO, including empty reads, flush, and gross-error status on a
  full-FIFO overwrite. FIFO overflow alone does not generate an IRQ.
- Configuration 1/2/3, destination ID, timeout, synchronous parameters and clock
  conversion. Configuration 2/3 retain all written bits as the manual specifies.
- Family 0 / revision 2 identification, visible through the high counter only
  with Features Enable, a DMA NOP, and no high-count write since chip reset.
- Chip reset and the required following NOP; count/counter, destination and
  timeout survive reset. FIFO flags/bottom element and configuration reset.
- Enable/disable selection commands in the disconnected state, including the
  disable command's function-complete interrupt. No target is attached.
- SCSI bus reset detection and its optional interrupt, without clearing chip
  configuration. A functional reset pulse uses 130 CCF clocks at the SS5's
  40 MHz SCSI clock (26 us for CCF=8), following the manual's approximate pulse
  duration and reset timing scale. Commands requiring a second pending event
  stop explicitly; a complete command/interrupt queue is not implemented.

Reading the interrupt register while IRQ is asserted acknowledges the event and
clears latched status except terminal count. The level reaches DMA2 and then
NCR CPU level 4. DMA2 reset holds the core reset; accesses complete without data
transfer (reads choose zero) until reset is released. The NCR89C100 omits the
standalone core's RESETO pin, so unacknowledged SCSI reset does not reset the CPU.

The implementation supports byte accesses only. Padding, reserved/test registers,
wide accesses, commands issued in chip test mode, active selection, data-transfer commands and
stacked interrupts stop before mutation. SCSI bus arbitration, targets, sector
I/O and the DMA request/data path remain pending. Creating `sd` and `st` nodes
in OBP is evidence of firmware enumeration, not working disks or tapes.

CFG1 chip-test-enable can be written and read back during register diagnostics.
Its internal test state persists even if the bit is subsequently written zero;
chip reset clears it before normal commands resume. Test-register execution
remains unsupported.

`Serial::set_break(channel, asserted)` supplies a decoded asynchronous receive
BREAK. RR0 bit 7 follows its transitions through the WR15-controlled status
latch; WR0 Reset External/Status opens the latch. Termination queues one null
byte. A full FIFO makes termination retryable without losing the BREAK state.
Odd-parity termination stops explicitly; error/parity FIFO and bit timing are
pending. This headless input lets original OBP abort network boot normally.
The current CLI has no interactive BREAK command.
