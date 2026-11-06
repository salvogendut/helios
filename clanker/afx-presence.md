# AFX presence / resumed bring-up

- User explicitly resumed after the serial recovery prompt; previous stop note
  no longer blocks continued development. Next target is firmware device probing.
- Before trace: original ROM afx-slot-empty? atffd3a600 readsconstant6e000000
  viaASI20byte, testsbit0 and compareszero. No AFX card forCGsix profile ->0.
  Return inputbyte0, no genericfour-byteRAM, unsupportedwrites/widerwidths.
  Portalwaysboardpresent; nofakeTCXcard. Primaryms2appendixA PRES-L staticpin;
  primarySPARCengine5 OEM3.2.7.4/3.2.9 describesAFXvsSBuscards. Boarddecodeand
  bit0 contract inferredfrom originalROM, not schematicverified. QEMUpin agrees
  onaddress, butmodels4bytesRAM whichwedonotcopy.
- Newfaultat93375626,PCffd16718,nPCffef0000,wordc8012000,ordinarywordread
  physical78400000 (MACIOSCSIDMACSR). AFXreadnowpasses; serialerrorlooksidentical
  becauseOBP againentersrecoveryprompt. Don'tmistakeidenticalserialfornoeffect.
  CLI200M:199998649instructions,17traps,1334IRQs,1reset,PCffd273cc/nPCffd273c0.
- 280tests,fmt/clippy,fourdemospass; recordslocal/probes/afx-presence-ss5.
- Nextreference /tmp/helios-ncr89c100.txt andpinnedqemu-dma.c/esp.c.
  WordtransparentMACIOregisters, masks/reset/IRQs first; no fabricated DMA
  completion while no controller engine exists. ExistingMachineDMAAPIavailable.
