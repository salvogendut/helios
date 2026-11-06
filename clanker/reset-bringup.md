# Reset/status and OBP serial initialization

Checked 2026-11-06. Human contract is docs/reset.md; pinned primary/reference
sources are in docs/references.md. No firmware prompt or differential run.

- NCR89C105 figure 6-6: physical 0x71f00000, word access only. SR bit 0
  requests system reset and reads zero; RS bit 1 and WD bit 4 read/clear.
  Zero clears status, one preserves it. RS zero-clear follows the read/clear
  label and WD's explicit convention; QEMU only handles SR writes.
- ResetControl records a sticky request. Machine services a guest store after
  successful execution/time advancement, before the next instruction or IRQ.
  Step::SoftwareReset includes the original PC/nPC and Some(instruction).
  A host bus write is serviced before CPU execution with None and no clock tick.
  CLI counts the store as an instruction and also reports the reset.
- Software reset resets CPU/MMU and I/O, sets RS, clears WD, preserves RAM,
  NVRAM/RTC and its phase, and completed endpoint output. CPU preserves GPRs,
  windows, PS, CWP and TT per microSPARC-II 3.11.3/10.2. Other state still uses
  the existing deterministic reset baseline (not a complete physical reset model).
- CPU error-state transition latches WD without resetting peripherals. Repeated
  stopped steps do not relatch; emulator Unsupported/WouldBlock do not assert it.
  Automatic microSPARC-II partial watchdog recovery is still pending. Do not
  silently restart on faults and hide the current inspectable error condition.
- OBP next hit WR14=0x82 on TTYA. Zilog 1992 5.2.16 explicitly says reset
  disables DPLL, source/encoding commands configure it, Enter Search Mode enables
  it. Accept configuration while disabled; still reject 0x20 search, auto-echo,
  DTR/request and other unsupported modes before mutating state/pointer.
- OBP initializes keyboard/mouse SCC too. Installed a second independent Serial
  at 0x71000000, A=keyboard, B=mouse, sharing the existing byte register behavior.
  Source: NCR table 6-7 and QEMU SS5 ms_kb_base. Input protocols/host frontend and
  both SCC IRQ sources remain pending; CLI routes only console SCC channel A.
- Final bounded OBP probe: 128 MiB/default NVRAM/no input/100 ns per successful
  step/10000-step limit. 304 instructions, no traps/IRQs/resets/output, exit 1:
  PC=0xbbb4, nPC=0xbbb8, instruction=0xd2a20400; ASI20 word store of 1 to
  physical 0x71d10010. Unsupported 64-bit processor user timer, no CPU error.
  This checkpoint is superseded by the user-timer implementation; see
  user-timer.md for the current SBus-control fault and TLB diagnostic stop.
- Original image unchanged at SHA256
  e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e.
  local/probes/reset-ss5 holds the configuration/trace/console record, separate
  from initial-ss5. OpenBIOS compatibility remains deferred.
- /tmp/helios-qemu-slavio-misc.c contains the pinned source; other manuals and
  NCR/QEMU timer source paths are listed in interrupts-timers.md.
