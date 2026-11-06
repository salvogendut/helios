# Timer, Ethernet and serial abort checkpoint

User resumed timer/Ethernet work after the previous prompt checkpoint. No
subagents. User then chose Sun IDPROM MAC 08:00:20:c0:ff:ee / hostid 80c0ffee
after supplying https://lib.ru/TXT/faqsunnvram.txt. This supersedes the briefly
suggested de:ad:c0:ff:ee:00; do not restore that address. Imported NVRAM is
never rewritten automatically. New IDPROM date=0, checksum=a9.

Timer diagnosis:
- Sun-4M 950-1373-01 July 19 1991 revision 50, section 5.3.2 page 24, explicitly
  says zero-limit free-run has no IRQ and overflow reloads 500 ns (skip zero).
  Figure shows pending/L in BOTH counter and limit. Limit read samples L then
  acknowledges it. This supersedes the old NCR zero-limit-overflow IRQ choice.
- NCR89C105 table 6-10 is 6 SBus clocks per EPROM byte; 8-bit acknowledge forces
  word reads into four byte cycles. SPARCengine5 table2-1: SS5/85 SBus21.25MHz.
  Physical PROM instruction/data reads add this service time to base100ns,
  with integer fractional carry. Boot aliases count too; direct debugging reads
  do not. Host stops consume no PROM phase/time. No instruction-PC special case.
- Timer diagnostic now passes. Normal CPU/cache timing is still functional.

Ethernet:
- DMA2 78400010 word CSR, 7840001c base high byte; MAC78c00000/+2 half RDP/RAP.
  Wrong size latches slave error; POST deliberately tests this. MAC metadata
  uses word-transparent halfwords; SWAP affects packet byte lanes only.
- NCR89C100 is IOMMU master5; service one DMA transfer per boundary. Invalid
  PTE/permissions produce MAC MERR+DMA E_ERR, not a synthetic CPU trap. Host
  unsupported errors stay pending and preserve PC/time. Packet OWN clears last.
- Init IDON requires all12halfwords. MACaddress is learned from guest block.
  CSR3 upper IPG extension differs from AMD PCnet. Primary E_BASE field label
  7:0 conflicts with Linux/QEMU high-byte placement; docs record the choice.
- Normal single-descriptor TX/RX, CRC/filter/ringwrap, bounded64frame queues;
  no host network backend. Chaining, external loopback/test/CRC-disable modes,
  cache diagnostics and cycle timing remain explicit gaps.
- OBP starts with mode44 internal loopback (not corrupt metadata). Need real
  TX->RX DMA, 8–32-byte packets and FCS. Normal mode later transmits64byteRARP.
- POST also writes CFG1=55,aa,00 on ESP. Accept readback and retain internal
  chip-test state until chip reset; test-register execution still unsupported.

Cancellation:
- Ctrl-C byte does not abort OBP's RARP loop. Decoded SCC BREAK does.
- Serial::set_break controls RR0 bit7 with WR15 latch and WR0externalreset;
  falling edge enqueues one null. FullFIFO release retries, oddparity stops.
  No serial IRQ or bit timing/CLI interactive BREAK implementation is claimed.

Proof: local/probes/timer-ethernet-ss5, original ROM unchanged (e7f40845...45e),
128MiB, 2B budget, fresh user-chosen NVRAM. FullRAMtest, no timerwarning.
RARP at995183053 and1057759536. BREAK onsecondpacket,off1058759536.
Prompt1058977191; command 1 2 + . CR returns3 andsecondprompt1059249163.
Final1059239040instructions/21sync/10102IRQ/1reset,4036bytes,PCffd1689c,
NPCffd168a0,noCPUerrororhoststop. Repeated on final implementation samecounts.
311tests,fmt/clippy/all4demos pass; CI/commit recorded in ignoredconfiguration.

Remaining POST error is parallel DMA CSR7c800000, PC700044f8 step8711303;
OBP handles it and continues. Thus cleanPOST remains false. The user's serial
ok stopping point is reached again; do not launch the parallel-port milestone
without a new continuation. SCSI data, storage, CGsix and OSboot remain pending.
