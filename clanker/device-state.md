# Device state and endpoint boundaries

- Checked 2026-11-06 against the revisions in docs/references.md. Human-facing
  behavior and limits are in docs/devices.md. Next work is interrupt delivery
  and timers, then evidence from firmware; no firmware prompt has been reached.
- Zilog SCC User's Manual 1992 section 2.3.4: one control pointer across both
  channels; direct data access preserves it. QEMU's per-channel pointer differs.
  RR2/WR2 and WR9 are shared. Use the Z8530 legacy read aliases, not ESCC extended
  readback. MAME is useful for comparison but the manual is authoritative.
- Serial is a deterministic asynchronous byte model. RX FIFO is three bytes;
  each completed-output queue is bounded at 4096. Receive returns false before
  consuming a host byte when disabled/full. Unsupported modes and backpressure
  stop before device mutation; CPU/MMU must not translate these into hardware
  faults or suppress them under NF. Walk/probe/RM-error paths propagate them too.
- `SystemBus` routes bytes directly. Do not use a word read to select a serial
  byte, or a read-modify-write to store it. Reject unsupported widths/atomics
  before a destructive read. Bare-metal machines still install only RAM.
- ST M48T08 R/W latches hold visible fields; internal time continues until ST
  stops the oscillator. W falling edge commits staged fields. RTC advancement
  is explicit, independent of host wall time and instruction count. Year origin
  1968 comes from QEMU sun4m wiring; the chip stores only a two-digit year.
- NVRAM raw export is not a full snapshot with a hidden running counter. Import
  validates length/clock but preserves configuration and IDPROM checksum as-is.
  Reset retains NVRAM, RTC latches and already transmitted output, clearing SCC
  RX/pending TX. Source images are never rewritten by the CLI.
- Downloaded comparison files (disposable, outside git):
  `/tmp/helios-qemu-escc.c`, `/tmp/helios-qemu-m48t59.c`,
  `/tmp/helios-qemu-sun_nvram.h`, `/tmp/helios-zilog-scc-1992.pdf` and `.txt`.
  ST PDF was read through the web tool. Local MAME devices are under
  `../mame/src/devices/machine/`. No implementation files were copied into Helios.
- Validate with fmt/clippy/test and all three demos. CLI `boot` and `serial-demo`
  use stdout for TTYA, stderr for diagnostics/trace; `run` keeps its prior output.
