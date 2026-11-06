# Original OBP serial monitor reached

- User stop condition is now met: original Sun SS5 OBP emits a responsive `ok`
  on SCC A / TTYA. Stop hardware bring-up here; do not continue into SCSI or
  timer changes without a new user instruction.
- Final controller subset: src/devices/floppy.rs, NCR82077 DOR/MSR/DSR/FIFO/
  DIR/CCR, SPECIFY/SENSE DRIVE/SENSE INTERRUPT/VERSION/CONFIGURE/DUMPREG/
  PERPENDICULAR/LOCK/SEEK/RECALIBRATE. No floppy drive or media engine attached.
  Missing commands/offsets, powerdown, atomics and unsupported widths remain
  explicit. Wider ordinary FDC accesses still produce NCR SBus error.
- Sources: /tmp/helios-82077.pdf/.txt Intel290166, /tmp/helios-ncr89c105.txt;
  pinned /tmp/helios-qemu-fdc.c for comparison. NCR overrides PC DOR bit wiring.
  Primary command description says79 recalibration steps, ST0 table says80:
  use79, document discrepancy; absent TRK0 yields70|unit and PCN0. Timed SEEK
  reports20|unit with requested PCN; no media position feedback exists.
  Delays derive from SRT/rate; individual pulse edges not modeled. PCN is zeroed
  at recalibration issue, seek PCN committed on completion. Simultaneous batch
  completions ordered by unit. Busy persists until SENSE INTERRUPT.
- Polling convention250us at1Mbps, scaled by datarate, derives from documented
  CONFIGURE suppression window rather than NCR measured timing. First command
  byte pauses polling, so slow parameter writes can still disable it. Four
  reset sense results c0..c3/PCN0, one interrupt; DORgate connects toNCR22/IRQ11.
- Initial FDC subset reached RECALIBRATE opcode07 at81415449, PCffd16898,
  bytewrite71400005, dor1c,rate0,specifyc2/33,config58,pretrack0. Timed
  disconnected-drive recalibration let OBP continue to SBus probing and monitor.
- Unmodified ROMSHA256e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e;
  Machine::ss5,128MiB,four32MiBDSIMMs,defaultNVRAM,noinitialinput,100ns/step.
  No CPU/ROM patch or diagnostic bypass. 16 earlier intentional/diagnostic traps;
  the seventeenth is guest data-access fault during later device probing:
  step93294238,PCffd265f8,nPCffd266ac,wordc88c0400,ASI20 byte read6e000000,
  SBusTimeout,TT29. Serial mentions espdma/esp before Data Access Error. Do not
  equate this exact fault address with the on-board ESP address78800000;
  full probe failure cause remains unexplored at user-requested stopping point.
- Firstprompt93481624:93481339instructions,17traps,268IRQs,1software reset,
  344serialbytes. PCffd27424/nPCffd27428, CPUerrorfalse. Instructioncountincludes
  resetting store. Early timerwarningexpected0000001e remains; reset28539752.
- Afterfirstprompt, send bytes "1 2 + .\r" only as Serial::receive accepts them.
  Reply echoescommand, prints3 and anotherok. Secondprompt93671422:
  93671135instructions,17traps,270IRQs,1reset,360serialbytes,samePC/nPC.
  Verification stops here. /tmp/helios-prompt-probe.rs/.log and-console.log;
  retained local/probes/obp-ok-ss5 withsource,metadata,transcript,NVRAMexport.
  A separate CLI100M run has344serialbytes,99999650instructions,17traps,
  333IRQs,1reset,PCffd273c0/nPCffd273c4,budgetexit0.
- This is an error-recovery monitor, not completed POST, normal firmware boot or
  OS compatibility. CLI outputsserial but is not interactive; library verifies
  bidirectionalconsole. SCSI/Ethernet/CGsix, timerdiagnostic and other documented
  hardware gaps remain. OpenBIOS still not proved working.
- Validation:278tests,fmt/clippy andallfourbuiltindemospass. FPUunchanged;
  no needrepeat208464casearithmeticoracleforthisdevice-onlymilestone.
