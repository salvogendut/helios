# microSPARC-II mask revision and comparison configuration

- Implemented physical 10003018 and 1000301c from microSPARC-II manual
  sections 5.7.17–18, figure 5.30 (visually inspected), table 38. Mask ID=30,
  revision 3.0 is a documented model choice, not a verified SS5/85 die revision.
  ID occupies31:24, reserved23:11, writable10:0; compare is full32bits.
  Writable values reset0; mask identity persists. Trigger/breakpoint controls
  remain explicit unsupported. Section5.14.2 requires external scan setup
  and stops clocks; do not invent architectural watchpoint traps.
- Pinned QEMU sun4m_iommu.c resets mask ID to turboSPARC23 and ORs writes
  into low24bits; neither behavior is copied into this microSPARC model.
- Original unmodified ROM, 128MiB, defaultNVRAM/noinput,100ns step. CLI200M
  budget reaches ffd16298/ffd162a0,199998745instructions,17traps,1238IRQs,1reset.
  TTYA prints timer warning, NVRAM checksum/default messages, Probing CPU
  FMI,MB86904. Subsequent initialization is still in progress, no prompt yet.
  A read at71900000 generates caught guest data-access trap at68099587;
  this is the missing NCR auxiliary LED/floppy register, not empty SBus.
- PROM timing experiment saved /tmp/helios-prom-timing-experiment.patch
  and /tmp/helios-prom-timing-experiment-tests.rs, NOT applied. Charging250ns
  per PROM byte lets free-running system timer interrupt much earlier in POST,
  producing Async Trap TT1a; it is not a validated fix. No primary source yet
  for precise SS5 PROM timing. Keep original100ns convention until coherent
  timing is established; do not alter firmware, zero-limit timer semantics or
  particular PC ranges to force diagnostic success.
