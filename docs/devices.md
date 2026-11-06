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
machine type `0x80`, locally administered MAC `02:00:00:00:00:01`, manufacture
date zero and serial `000001` (hostid `80000001`). Byte 15 makes the XOR of the
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

The machine supplies elapsed time from its deterministic 100 ns step quantum
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
Serial interrupt generation is still pending; no Sun PROM, OpenBIOS prompt or
OS boot has been demonstrated.

## NCR89C105 auxiliary ports

AuxIO1 at physical `0x71900000` accepts plain byte reads and writes. Figure 6-21
of the NCR specification defines writable link-test enable (bit 3), monitor/mouse
mux (bit 2), and front-panel LED (bit 0). Bit 5 reflects the external floppy
high-density input and ignores writes; its default disconnected value is zero.
Reserved bits 7:6 and 4 always read zero. The input can be supplied through
`Auxiliary::set_floppy_high_density` and survives reset; output latches reset zero.

Each write with bit 1 set emits a terminal-count pulse event; zero emits none.
The bit always reads zero. `take_terminal_count_pulses` drains these events for a
future floppy controller. The hardware's four-SBus-clock pulse width is not
modeled. Reads do not consume pulses, and writes replace the output bits.

Wrong widths, atomics, masked stores and image loads stop before device effects.
This implements the documented base registers only; address mirrors,
modem/configuration registers, an actual floppy controller and
Ethernet link-test behavior remain pending. Pin-state storage and TC events do
not establish functioning floppy or Ethernet devices.

AuxIO2 at `0x71910000` also requires byte accesses. Its otherwise unused bits
3:2 retain writes. The power-failure input is not asserted in this model; bit 5
therefore reads zero, and writing the clear bit (1) has no pending failure to
acknowledge. Reserved bits 7:6 and 4 read zero. A write requesting power-off
(bit 0) stops explicitly before changing device state: powered-down machine
execution is not implemented. External power-failure detection and its
configuration/interrupt wiring remain pending. Reset clears the scratch bits.
