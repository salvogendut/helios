# SS5 Ethernet

The NCR89C100 combines a DMA2 interface with an NCR92C990 MAC, compatible with
the original LANCE register and descriptor format. Helios implements the
registers, initialization DMA, single-buffer transmit/receive and internal
loopback. Original OBP's internal loopback test completes and network boot
transmits RARP requests. This is not an OS or network-boot compatibility claim.

## Registers and interrupts

| Physical address | Access | Function |
| --- | --- | --- |
| `0x78400010` | Word | E_CSR: identity, reset, control and error/IRQ status |
| `0x78400014` | Word | E_TST_CSR: cache diagnostics, unsupported |
| `0x78400018` | Word | E_VLD: cache diagnostics, unsupported |
| `0x7840001c` | Word | Upper DVMA address byte, reset `0xff000000` |
| `0x78c00000` | Halfword | MAC register data port |
| `0x78c00002` | Halfword | Register address port, low two bits |

Wrong transfer sizes return SBus error acknowledge and set E_SLAVE_ERR, cleared
by writing one. Unsupported atomics, masked stores and diagnostic registers
stop before device mutation. E_RESET is held until cleared and resets the MAC;
while held, MAC reads return zero and writes transfer no data. E_DEV_ID is `a`.
The MAC's interrupt and DMA memory error are separately visible in E_CSR and
gated by E_INT_EN onto NCR89C105 source 16 / CPU level 6. E_INVALIDATE clears
the memory error, but cannot acknowledge the MAC interrupt.

CSR0 implements STOP, INIT, START, demand polling, interrupt enable, read-only
TX/RX state and write-one-to-clear causes. INIT completes only after all twelve
initialization halfwords have actually been read through the IOMMU. CSR1/2 hold
the 24-bit block address and require STOP for access. CSR3's low three bits
control the bus interface. NCR's upper programmable-IPG fields are retained;
unlike PCnet, they are not interrupt masks. IPG fields can be programmed while
waiting for initialization; changing active DMA byte-lane configuration stops
explicitly. IPG timing and external PHY timing are not modeled.

## DMA and packet endpoint

`Machine::step` services at most one pending Ethernet transfer before sampling
interrupts. `Machine::service_ethernet` exposes the same operation to headless
callers without executing a CPU instruction or advancing time. DMA uses onboard
master 5, the shared CPU/I/O TLB and real IOPTE permissions. Arbiter denial
retains the request for retry. A hardware DMA error sets MAC MERR, turns off
TX/RX and latches E_ERR_PEND. Unsupported targets or modes remain host stops;
CPU PC/nPC and time stay unchanged. Partial completed DMA writes remain in RAM.

The MAC's 24-bit address is combined with E_BASE_ADDR. Metadata is always read
as big-endian halfword bit patterns on the SS5 wiring. SWAP only changes packet
byte lanes: set means natural byte order; clear exchanges the lanes within
each halfword, including odd-address transfers. MAC address pairs in the init
block place the first octet in the low eight bits of each halfword.

Supported rings contain 1–128 eight-byte descriptors. Each packet currently
occupies one owned descriptor. Transmission reads the payload before clearing
OWN and raising TINT. Receive stores the payload and calculated Ethernet FCS,
then the message length, then clears OWN and raises RINT. Ring indices wrap.
Broadcast, physical address, multicast hash and promiscuous filtering apply.
A matching packet without an owned receive descriptor sets MISS.

`Ethernet::take_frame()` returns a transmitted frame without preamble or FCS.
`receive_frame(&bytes)` accepts complete 60–1514-byte frames without FCS, with
a bounded 64-frame input queue. A full queue or invalid length returns false;
frames arriving while the receiver is disabled or in internal loopback are
discarded. The 64-frame output queue applies backpressure before another TX
descriptor is fetched. Reset retains already transmitted output. Internal
loopback accepts 8–32-byte test packets, delivers them through the real receive
ring and never exposes them to the host network endpoint.

The frontend has no network backend or RARP/TFTP server yet. The library packet
endpoint is the boundary for future isolated or host networking. No network
traffic is sent to the host's interfaces by the current CLI.

## Limits and reference differences

Descriptor chaining, external loopback, forced collisions, CRC-disable/test
modes, wire errors, runt/oversized normal transmission, retransmission timing
and the DMA2 cache diagnostics remain explicit unsupported operations. There
is no dirty writeback cache: functional DMA transfers are coherent and complete
one at a time, so drain/invalidate pulses complete immediately. Burst modes
are configuration only; reserved encodings stop. Transfer deadlines and MAC
FIFO contention are not cycle-accurate. Idle transmit polling uses 1.6 ms;
TDMD requests polling immediately.

The NCR manual labels E_BASE_ADDR as bits 7:0, although Linux's sunlance driver
and pinned QEMU use bits 31:24. Helios implements the shifted high byte and
records that disagreement rather than accepting both encodings. NCR's CSR3
IPG extension also overrides the standalone description that restricts all
CSR3 access to STOP. The NCR89C100 integration notes explicitly direct software
to program IPG while waiting for IDON.

Sources: NCR89C100 chapter 3 pages 3-18–3-22 and chapter 4's NCR92C990 mode,
CSR and descriptor definitions; integration notes in chapter 2; pinned QEMU
`hw/dma/sparc32_dma.c` and `hw/net/pcnet.c`; pinned MAME `am79c90.cpp`; Linux
v6.12 `sunlance.c`. See [references.md](references.md) for links. Regressions use
independent descriptor fixtures and CRC values, including both byte-lane modes,
ownership ordering, partial DMA failure, IRQ acknowledgement and loopback.
