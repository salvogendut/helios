# Ultra 5 OpenBoot fast MMU miss handling

Original 3.31.0 reaches `fprs!` with `%g4=4`. Its first missing mapping
occurs with TL 0, TBA `0xf0000000`, PSTATE `0x14` and primary context 0.
All four observed traps are fast data misses (TT `0x68`). The guest handler
begins at VA `0xf0000d00` with the MMU globals selected.

| Fault PC | D-MMU Tag Access | Operation |
| --- | --- | --- |
| `0xf0009018` | `0xffffffffff000000` | LDX, retried through D-TLB slot 5 at PA `0x3ff6cf00` |
| `0xf0009238` | `0xfffffffc003fe000` | STX |
| `0xf0009238` | `0xfffffffc003c0000` | STX |
| `0xf0009238` | `0x00000000fffc6000` | STX targeting the PCI host bridge |

OBP installs the mapping through D-MMU Data In at `0xf0000f70`, then executes
`membar #Sync` and `retry`. The first retried load returns zero from RAM.
The code also uses explicit I- and D-side page demaps. The core models
Uclear at replacement boundaries and commits demaps synchronously.
Protection faults, RED-state entry, window traps and interrupts remain open.

| NVRAM seed | Retired at first PCI control store | Traps |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,466,988 | 4 |
| Cold, diagnostics off | 53,456,505 | 4 |
| Soft, diagnostics on | 22,915,896 | 4 |
| Soft, diagnostics off | 22,905,789 | 4 |

The serial transcripts, first-miss trace and refill trace are in
`local/ultra5-fprs-2026-11-22/`. The next store targets PA `0x01fe00002000`
with `0x000000140020010f`, an eight-byte access to PCI Control/Status.
[Reproduction](../docs/ultra5.md#openboot-handles-its-first-mmu-misses)

## PCI register initialization

The next 5,766 retired instructions include the PCI Control/Status store
and 74 zero stores across the clear and partial mapping register banks.
The local-ROM regression checks each physical store and its value, starting
with one populated PCI and onboard interrupt state and one enabled mapping
in each bank. It also checks that the fifth guest trap is a fast D-TLB miss.

| NVRAM seed | Retired before PCI target address programming | Traps |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,472,754 | 5 |
| Cold, diagnostics off | 53,462,271 | 5 |
| Soft, diagnostics on | 22,921,662 | 5 |
| Soft, diagnostics off | 22,911,555 | 5 |

All four stop at the store of `0x40` to PA `0x01fe00002028`. The raw serial
transcripts and stop records are in `local/ultra5-pci-init-2026-11-22/`.
[PCI interrupt initialization](../docs/ultra5.md#pci-interrupt-initialization)

## First access to the Simba command register

After target selection the guest retires the diagnostic, host PCI command
and latency writes. Counting from immediately before the target store adds
69,533 retired instructions on each seed. The local-ROM test checks the
little-endian command readback (`0x0146`), latency (`0x28`) and the
operands of the next `stha %l1,[%l0] %asi`.

| Seed | Instructions retired | Trap entries |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,542,287 | 5 |
| Cold, diagnostics off | 53,531,804 | 5 |
| Soft, diagnostics on | 22,991,195 | 5 |
| Soft, diagnostics off | 22,981,088 | 5 |

The instruction at VA `0xf00266a0` attempts a zero write to PA
`0x01fe01000804` through ASI `0x15`. This is Simba function 0's primary
command, separate from the CPU's PCI host command at `0x01fe01000004`.
Diagnostic runs end in `ttya initialized\r\n` and quiet runs emit no bytes.
Serial transcripts and stop records are under `local/ultra5-pci-target-2026-11-22/`.
[Reproduction](../docs/ultra5.md#pci-host-bridge-configuration)

## Downstream bridge setup and first IOMMU read

All four seeds retire another 6,059 instructions. The ROM fixture checks
both enable/disable sequences, bus numbers, address maps, control and timer
readbacks. The next access is an LDX from VA `0xfffc4200` at PC
`0xf0009018`, nPC `0xf000901c`, targeting PA `0x01fe00000200`. Start
the IOMMU work with IIi §19.3.2.1 and this mapped read.

| Seed | Retired before IOMMU control read | Trap entries |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,548,346 | 5 |
| Cold, diagnostics off | 53,537,863 | 5 |
| Soft, diagnostics on | 22,997,254 | 5 |
| Soft, diagnostics off | 22,987,147 | 5 |

The per-seed serial transcripts match the preceding checkpoint byte for
byte. Stop records and validation logs are in
`local/ultra5-simba-2026-11-22/`.
[Reproduction](../docs/ultra5.md#simba-bridge-configuration)

## IOMMU setup and CPU TICK boundary

Each seed retires another 134,007 instructions and takes one additional
fast data miss. The guest reaches its Forth `tick!` primitive with
instruction `0x89912000` (`wrpr %g4, 0, %tick`), `%g4=0`, PC/nPC
`0xf00235f0/0xf00235f4`. IIi §14.1.8 defines TICK.NPT and the
63-bit cycle counter. The existing APB tick is a separate PCI clock.

| Seed | Retired before TICK write | Trap entries |
| --- | ---: | ---: |
| Cold, diagnostics on | 53,682,353 | 6 |
| Cold, diagnostics off | 53,671,870 | 6 |
| Soft, diagnostics on | 23,131,261 | 6 |
| Soft, diagnostics off | 23,121,154 | 6 |

The local-ROM regression checks every IOMMU store and readback, starting
with 16 valid cacheable entries. The unmodified-ROM serial transcripts are
byte-identical to the Simba checkpoint. Per-seed results and validation
logs are in `local/ultra5-iommu-2026-11-22/`.
[Reproduction](../docs/ultra5.md#iommu-initialization)

## First CPU timer interrupt

All four original-ROM seeds took six fast data misses and one level-14
interrupt before reaching UDIVX at `0xf00564b4`. Cold boot retired
54,078,142 instructions with diagnostics on and 54,067,659 with them off.
Soft boot retired 23,527,050 and 23,516,943 respectively. The soft diagnostic
trace records PC/nPC `0xf0056484/0xf0056480` at interrupt entry. The guest
rewrote the trap return PCs to leave its spin loop. Serial output matched the
IOMMU checkpoint byte for byte. The transcripts, traced timer registers and
validation logs are in `local/ultra5-tick-2026-11-22/`.
[Reproduction](../docs/ultra5.md#cpu-timer-and-clock-calibration)

## Clock calibration and PCIO command boundary

After UDIVX and the mapped BAR1 read, OBP reaches its first unsupported
PCIO command write. All four seeds stop at PC/nPC
`0xf0042ff4/0xf0042ff8` with eight MMU misses and five timer interrupts.
Cold diagnostic on/off retired 54,406,979/54,396,496 instructions. Soft
diagnostic on/off retired 23,855,887/23,845,780. Per-seed transcripts and
stop records are in `local/ultra5-calibration-2026-11-22/`. For the next
step, distinguish the PCIO function's PCI command bits from the IIi host
bridge and Simba command registers already implemented.
[Reproduction](../docs/ultra5.md#onboard-ebus-configuration)

## PCIO command, RTC and reset report

All four OBP 3.31.0 seeds now stop at the Reset Control store on the same PC,
`0xf0009238`, after 12 synchronous trap entries and 277 timer interrupts.
Soft reset retired 51,120,398 (diagnostics on) and 51,110,291 (off).
Cold reset retired 81,671,490 (on) and 81,661,007 (off). The configurations
were intentionally uninitialized apart from byte 1 and the reset marker.
The soft marker steers the firmware path while both fixtures start the board
with POR asserted.

The original-ROM regression checks two retired EBus command writes, distinct
guest reads of latched RTC seconds, the serial-A checksum/default/reset
messages, and the translated address and value of the next store. The guest
uses its own timekeeping and console routines. The four serial transcripts,
stop records and validation logs are under
`local/ultra5-pcio-timekeeper-2026-11-22/`.
[Reproduction and remaining limits](../docs/ultra5.md#timekeeper-and-reset-report)

## IIi acknowledgements and memory-bank probe

The original ROM retires the Reset Control acknowledgement, clears the two
CPU data-buffer error ports and clears PCI fault status at PBM offsets
`2010`, `0030` and `0040`. The diagnostic ROM test checks both pairs of
SDB clears, the three PCI clears, a mapped word read of `OBMD`, the module
and bank messages and the later guest store to Mem_Control0.

A bounded soft-marker trace shows the Forth routine computing
`0x900014d6` from `0x9000143d`. Its thread at VA `0xf0059494` contains
`0 mc-control0! mc-control0!`. The first store retires at PC `0xf0009238`.
The following fetch at VA `0xf000923c` still maps to PA `0x3ff8923c`
with LSU_CONTROL `0xf`. A diagnostic read of the backing xword in the
old geometry returns `0xe0700004c859e008`; in the new geometry it returns
zero. The second register write has not retired. Establish the hardware
cache behavior for code, Forth thread and stack accesses across this interval
before changing DRAM decoding. The trace does not identify the supplying
cache on physical silicon.

The four TTYA transcripts, stop records and soft-marker memory trace are in
`local/ultra5-reset-cause-2026-11-22/`.
[Reproduction and exact budgets](../docs/ultra5.md#module-and-memory-bank-report)

## Refresh and PCI enumeration

The original-ROM regression checks the zero raw backing xword during the
geometry change, the cached instruction that still retires, the guest’s
second Mem_Control0 write and readback (`0x900014d6`), and the subsequent
little-endian ATI device-ID load. At PA `0x01fe01011002` the latter returns
`0x4754`. Both the bridge and display identities match the published
440 MHz Ultra 5 configuration. The normal external-cache data path and
display MMIO remain open.

With all four seeds the next unserved operation is a byte read from
Simba A at PA `0x01fe0100083d` (PCI Interrupt Pin). The serial-console continuation covers this register and subsequent PCI enumeration. The latest serial
transcripts and exact stop records are in `local/ultra5-refresh-cache-2026-11-22/`.
[Reproduction and budgets](../docs/ultra5.md#dram-refresh-and-pci-identification)

## Interactive serial A monitor

The normal OBP banner is followed by an interactive prompt with the saved
`auto-boot? false` configuration. Factory recovery defaults select net;
sending SAB BREAK after the banner enters the same monitor. The host waited
for each prompt before delivering the bytes of `1 2 + .\r`. Both transcripts
end with the echoed command, `3`, and a new prompt. Evidence and the 8 KiB
console seed are in `local/ultra5-serial-console-2026-11-22/`.

An earlier recovery prompt followed an instruction MMU miss during the first
onboard PCI probe. A master-abort load had already retired when the deferred
trap arrived. DONE skipped the next unretired Forth instruction. Preserving
that instruction as the continuation lets the firmware finish enumeration.
The recovery capture is retained separately from the normal-banner evidence.
The SAB driver masks external interrupts and polls ISR0 with IPC.VIS set.
A short command below its 32-byte threshold needs the four-frame TIME event
and subsequent RFRD to expose the bytes.
[Console instructions](../docs/ultra5.md#interactive-serial-console)
