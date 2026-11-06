# microSPARC-II bus faults and capture registers

Helios distinguishes an unanswered SBus cycle from an error acknowledgement by
a selected slave. CPU reads report synchronous SFSR/SFAR state; failed plain
byte, halfword and word CPU stores complete and report an asynchronous module
error through AFSR/AFAR and the NCR89C105 interrupt controller. Memory requests
also update MFSR/MFAR metadata. Parity generation/checking remains unimplemented.

The sources are the microSPARC-II User's Manual revision 1.1, sections 5.3,
5.6.4–5.6.5, 5.7.5–5.7.9, 9.1 and chapter 11 table 50; the NCR89C105 address
map, SBus feature subset and figures 6-9/6-11; and SBus transfer-size encoding.
[References](references.md) records these primary documents and emulator comparisons.

## Synchronous read errors

With the sun4m controller installed, unmapped physical accesses in
`0x30000000`–`0x7fffffff` receive `BusErrorKind::SBusTimeout`: these are the five
SBus slave-select apertures. Mapped memory/devices retain their own behavior.
RAM, control and local-graphics holes remain distinct unmapped accesses. A bare
bus without the controller does not synthesize SBus errors.

The NCR's documented reserved windows return `SBusError`. The floppy aperture
at `0x71400000`–`0x714fffff` rejects halfword/word/doubleword transfers with an
error acknowledgement, as specified for this byte-only range. Its byte
operations remain explicit emulator stops because the floppy controller is not
implemented. Register mirrors, full onboard decoding and remaining device-width
rules are separate work; this is not a complete NCR address decoder.

SFSR.TO (bit 11) distinguishes timeout from SFSR.BE (bit 10). FT, AT, level,
FAV, overwrite and fault priority retain their existing rules. SFAR captures the
CPU virtual address. Table walks and probes retain the physical error cause
through `MmuFault::bus_error`, along with the walk level. Unsupported operations
and backpressure do not become hardware faults. PCR.NF still suppresses eligible
synchronous faults while recording status; it does not hide emulator limits.

Physical CPU read failures use the existing access-error traps, or CPU error
state with ET=0. A timeout is completed immediately in the functional model;
the real controller's 256-SBus-clock delay is not simulated.

## Asynchronous CPU stores

A plain byte/halfword/word store that receives an SBus timeout or error
acknowledgement completes its instruction and captures the physical address in
AFAR. It does not write CPU SFSR/SFAR, take a synchronous trap, or set watchdog
status. The module interrupt can be taken at the next machine step, subject to
ET and NCR masks. Level 15 bypasses PIL, as with other supported level-15 sources.
PCR.NF does not suppress an asynchronous error.

| Address | Register | Behavior |
| --- | --- | --- |
| `0x10001000` | AFSR | Writable mask `0xff0e0000`; bits 23:20 always read `1000`; read unlocks capture |
| `0x10001004` | AFAR | Physical address bits 30:0; reading does not unlock capture |
| `0x10001050` | MFSR | Writable mask `0x8188f8f0`; read unlocks status capture |
| `0x10001054` | MFAR | Physical address bits 30:0; read unlocks address capture |

Fault registers require aligned word accesses. Rejected widths/atomics do not
perform destructive reads. Status/address values remain readable after unlock;
the next eligible capture replaces them. Reset clears writable contents and
locks; the fixed AFSR nibble remains visible.

AFSR records ERR, TO or BE, S, FAV and SBus SIZE: byte=1, halfword=2, word=0.
RD is clear for these store errors. While locked, subsequent errors set ME and
retain the first address/cause. Reading AFSR unlocks the pair. Software can write
implemented status/address fields for diagnostics; an ERR status write establishes
the capture lock and a zero ERR clears it. Such writes do not generate interrupts.
This treatment of diagnostic lock writes is a functional choice; the manual
specifies writable fields and hardware-error locking but not their interaction.

The NCR latches system pending bit 30 (module error), mapped to CPU level 15.
CPU clear-pending bit 15 (IC) acknowledges it independently of software level 15
(bit 31). Disabling the module-error source through its mask-set bit also clears
the latch, following the documented mask acknowledgement path. Mask All only
gates delivery. Reading or clearing AFSR does not acknowledge the NCR latch.
Another hardware error can assert a new interrupt even while capture is locked.

SBus doubleword store fault completion, split transactions, late-error signalling
and write-buffer timing remain explicit gaps. Doubleword store bus errors stop
before capture rather than choosing an undocumented split/burst outcome.
Successful RAM doublewords retain their existing indivisible behavior.

## Memory request capture

MFSR/MFAR update on CPU and DMA requests to the main-memory aperture, including
page-table reads and CPU R/M updates. MFAR records the physical request address.
MFSR captures S, CP, BM, C and request Type:

| Request | Type |
| --- | --- |
| Current uncached read path, including byte/halfword/word/doubleword loads | `1` (DRAM RD64) |
| Byte / halfword / word / doubleword store | `9` / `a` / `b` / `c` |

The CPU drives S from actual PSR.S, independently of alternate-space permissions.
CP distinguishes CPU requests from I/O requests. BM reflects PCR.BM; C reflects
the CPU PTE's cacheable bit for translated target accesses and is zero for bypass,
walks and DMA. Ordinary coherent accesses do not perform cache-line fills; their
capture type follows the current uncached transfer path. Atomic RAM operations
record their read followed by their completed write.

`Bus::set_cpu_supervisor` and `observe_memory_request` carry this information
without making the physical memory decoder infer CPU state. CPU stepping and
Machine DMA calls supply it. Direct MMU harnesses can set the pin state explicitly;
SystemBus starts in supervisor mode. Raw host bus/image accesses do not impersonate
CPU requests or overwrite capture state.

MFSR and MFAR have independent read-unlock behavior, following their individual
register descriptions. A diagnostic MFSR.ERR write locks both; subsequent requests
preserve each locked part until its read or a clearing status write. No parity
error, ATO event or module interrupt is invented from ordinary empty-bank reads.
Real memory parity storage/checking, error-event generation and its interrupt
routing remain pending. The writable fields and locks alone do not establish a
working parity diagnostic.

## Validation

Focused tests cover slot timeout versus slave acknowledgement, virtual versus
physical fault addresses, status priority/NF, table walks/probes, precise read
failures, completed asynchronous stores, sizes, first/multiple error capture,
masking, IC acknowledgement, interrupt entry, register masks and read-unlock,
CPU/DMA metadata, actual CPU mode, unsupported operations and resets.

The unchanged original OBP now passes both bus-error diagnostic checks without
the earlier warning/reset and stops at `LDDF` in POST. See [firmware.md](firmware.md)
for the bounded run and remaining FPU boundary. No firmware prompt or successful
POST is claimed.
