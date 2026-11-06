# NCR89C105 auxiliary port

- AuxIO1 base71900000 byte accesses, chip specification figure6-21. Output
  mask0d, density input20, TC bit02 alwaysreads0, reserved d0 alwaysreads0.
  Each TC write emitsone pulse, drained throughdeviceAPI; no FDC connected yet,
  no bus-clock pulse duration modeled. Resetoutputs/events, retainedexternalinput.
- Baseportsimplemented; addressmirrors andconfig/modem pending. AuxIO2at71910000
  retainsbits3:2, withstablepower/noPFDinput andackno-opwhennoevent. Poweroff
  bit0explicitlystopsbeforestatechanges. Externalpowerfailconfig/IRQ pending.
  Do not copy QEMU's unrestricted8bitreadback; actualreservedbits/density/TC
  are independently defined. Wrongwidth/atomic/imageaccess stopsbeforeeffects.
- Previous revision-probe observed caughtfaultat68099587,ffd16754 instruction
  c8092000 read71900000. This is an onboarddevice, not absentSBus.
- tests/auxiliary.rs exhausts256writes withbothdensityinputlevels, validates
  pulseconsumption/reset, rejectsinvalidbusaccessatomically, andexecutessynthetic
  PROMSTB/LDUBusingrealbus. No firmware bytes or reference implementations vendored.
- AuxIO1-only 2B-step run still takes one additional caughtfault, now71910000
  at68099763, and ends ffd16428/ffd1642c,1999982081instructions,17traps,
  17902IRQs,1reset,225serialbytes. AuxIO2 removes that absent-device fault.
  Longtrace shows repeated Forth open-device/path-component processing with
  monotonically allocatedinstances; investigate this loop, not more blindsteps.
- Bothports: originalROM100M CLIbudgetstopsatSTB71400002,PCffd16898,
  nPCffd1689c,worde0292000,libraryattempt80936336. Firmware nowadvances to
  NCR82077DOR; apparentForthloopwasassociatedwithmissingauxports. Serial225B.
  272tests,fmt/clippyandfourdemospass. Source/testexpectationsfromprimaryNCR.
