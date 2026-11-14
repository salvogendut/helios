# SS-5 system reset and status

The NCR89C105 system control/status register is a single aligned word at
physical `0x71f00000`. The source is figure 6-6 and the reset discussion in the
[NCR chip specification](https://www.ibiblio.org/pub/historic-linux/early-ports/Sparc/NCR/NCR89C105.txt).
QEMU's pinned `slavio_misc.c` independently confirms the address, access width,
software-reset request and software-reset status bit. See [references.md](references.md).

| Bit | Meaning | Behavior |
| --- | --- | --- |
| 0, SR | Software-reset request | Writing one requests a whole-system reset; always reads zero |
| 1, RS | Software-reset status | Set by software reset, cleared by power-on reset or writing zero |
| 4, WD | Watchdog status | Set by a CPU error event; cleared by power-on/software reset or writing zero |
| Others | Reserved | Read zero; writes ignored |

Writing one to RS or WD preserves the existing latch; it cannot set a clear
latch. Reads are non-destructive. The specification labels both status bits
read/clear and explicitly describes zero-clear for WD; Helios applies that
convention to RS as well. QEMU implements only the software-reset write and
retains its status across host resets. Helios distinguishes the reset causes.
Bytes, halfwords, doublewords, atomics and partial masked stores are rejected
before any side effect, consistently with other word-only device apertures.
Adjacent reserved NCR addresses return an SBus error acknowledgement; wider
register mirroring is not modeled. See [fault capture](fault-capture.md).

`ResetControl` only records a request. `Machine::step` and `step_scheduled` apply it after the
requesting store completes, before the next instruction or interrupt. The store
advances its 100 ns quantum plus any PROM wait before reset clears timer state.
In the scheduled path this is a whole-system boundary: both processors stop
at the request, retained RTC/serial state receives the store's completion time,
and then the CPUs, devices and scheduler reset. A request
already pending on the physical bus is handled before CPU execution, even if
the CPU is in error state, and consumes no instruction time. Reset pulse lengths
and oscillator stabilization delays are not modeled.

The outcome is `Step::SoftwareReset`, carrying the store's PC/nPC and instruction
word, or `None` for a preexisting bus request. The CLI prints `reset=software`
trace records and a reset count. A completed reset-triggering store also counts
as an instruction; the reset count is additional information, not another step.
The core never exits the host process to service a reset.

Software reset restarts PROM execution at PC/nPC 0/4, resets the MMU to boot
mode, resets both SCCs, masks/clears interrupts and resets both counters.
RS becomes one and WD clears. RAM, ROM, NVRAM/RTC, the fractional RTC second
and already transmitted bytes survive. The integer register file, PSR.PS/CWP and
TBR.TT survive non-power-up reset, following microSPARC-II sections 3.11.3 and
10.2; other CPU/MMU state uses the existing deterministic reset baseline
described in [memory.md](memory.md). The current FPU baseline clears its registers
and writable status on either reset; the fixed version remains visible, as
documented in [fpu.md](fpu.md). Full preservation of cache/TLB RAM and
physical reset timing are outside this functional model.

`Machine::reset` applies the existing power-on baseline and clears both status
latches, while retaining the same machine memory and nonvolatile state. It also
resets the MBus AER, bus-installed MXCC state and the standalone IOMMU. A guest
software reset likewise resets the standalone IOMMU, so an I/O translation
cached before the restart cannot survive into the next system image.
`Machine::reset_processor_module(processor)` is a narrower module-local reset:
it resets only that CPU/MMU and its MXCC, leaving shared devices, the AER, the
standalone IOMMU, RAM and the other independently constructed module intact.
In a two-CPU machine, whole-machine reset resets both owned CPUs/MXCCs and the
shared devices once; module-local reset does not reset the other CPU or the
scheduler's shared clock.

The NCR controller latches IU_ERROR without resetting SBus devices. Helios
latches WD when a machine step first enters CPU error state. Repeated attempts
to step that stopped CPU do not create new events; unsupported emulator
behavior does not assert WD. The CPU still stops with its precise fault for
inspection. The microSPARC-II's automatic partial watchdog reset sequence is
**not implemented**; the controller latch alone is not watchdog recovery.

Regression tests cover status clearing, reserved bits, invalid widths, mappings,
translated write protection, a guest reset store in a branch delay slot,
retained CPU/memory/RTC state, reset device state, repeated reset traces and the
distinction between CPU error and an unsupported emulator operation.
