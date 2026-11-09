# Memory and reset

The first memory-system target is microSPARC-II, as used in SPARCstation 5.
Its physical addresses are **31 bits**, virtual addresses are 32 bits, and the
context register is eight bits. The SS-20/Axil profiles select a functional
SuperSPARC-II-style reference-MMU identity: 36-bit physical addresses, boot mode
in PCR bit 13, and the high sun4m address map (see [machine profiles](machines.md)).
This is a functional implementation with synchronous memory access. [Cache
diagnostic storage](cache.md) is implemented; cache hit/fill behavior and timing
remain pending. The installed peripheral subset is described in
[devices.md](devices.md).

The sources are the Sun **microSPARC-II User's Manual**, revision 1.1, July 1994,
chapters 5 and 12, and SPARC V8 appendix H. Links and the pinned QEMU comparison
revision are in [references.md](references.md). Tests use newly written programs
and synthetic page tables, without firmware or an external assembler.

## CPU access and reset

`Cpu` owns `Srmmu`; the MMU calls the physical `Bus`. Ordinary instruction
fetches select ASI 8/9 and ordinary data accesses select ASI A/B according to
PSR.S. Alternate-space instructions require supervisor mode and register
addressing (`i=0`). Privilege/encoding checks happen before memory effects.
Supported instructions are LDA/LDUBA/LDUHA/LDSBA/LDSHA/LDDA,
STA/STBA/STHA/STDA and SWAPA/LDSTUBA.
For the reference profile PSR.DE (bit 15) selects little-endian data accesses:
halfword, word, integer doubleword and FPU loads/stores apply the documented
byte conversions, while instruction fetches and page-table accesses remain
big-endian. The microSPARC-II profile masks PSR.DE.
Alignment is checked before translation and is not suppressed by PCR.NF.
The public MMU load/store methods assume that the CPU has checked privilege.

`Cpu::new` and `Machine::bare_metal` start with translation and boot mode off.
`Machine::sun4m` installs RAM, PROM, serial, NVRAM/RTC, interrupt control and timers,
then resets. `Machine::ss5` provides the same peripherals and reset state with
physical DSIMM banks and empty-bank decode; CLI `boot` uses this constructor.
The `sun4m` constructors remain contiguous-RAM synthetic harnesses. See
[DRAM and TLB diagnostics](dram-tlb.md) for layouts and API differences.
The reset baseline is:

- PC=0, nPC=4; PSR.S=PSR.PS=1, ET=0, CWP=0; WIM=1.
- TBR, Y, general registers and ICC are deterministically zero.
- SS-5: PCR implementation/version=`0x04000000`, BM(bit 14)=1.
- SS-20/Axil: PCR implementation/version=`0x08000000`, BM(bit 13)=1.
- EN=NF=IE=DE=0; context, CTPR, fault registers and the TLB are cleared.

The deterministic values for otherwise unspecified state are emulator choices,
not a claim about power-on RAM/register contents. `Machine::reset` retains RAM
and ROM, NVRAM/RTC (including fractional time) and completed serial output.
It resets CPU/MMU, both SCCs, interrupt control and timers, and clears reset
status. Guest software reset retains the register file, PSR.PS/CWP and TBR.TT,
sets the software-reset status bit and otherwise uses this baseline.
[System reset](reset.md) describes the register and the still-pending automatic
watchdog recovery sequence.

The translation modes follow microSPARC-II table 40:

| Access | Mode | Physical address |
| --- | --- | --- |
| ASI 8/9, BM=1 | SS-5 boot instruction space, regardless of EN | `0x70000000 \| (VA & 0x0fffffff)` |
| ASI 8/9, BM=1 | SS-20/Axil boot instruction space | `0xff0000000 \| (VA & 0x7ffff)` |
| ASI 8/9, BM=0, EN=0 | Instruction pass-through | `VA & 0x7fffffff` |
| ASI A/B, EN=0 | Data pass-through, regardless of BM | `VA & 0x7fffffff` |
| ASI 8/9 with BM=0, or A/B, EN=1 | Page-table translation | PTE and page offset |
| ASI 20 | Physical bypass, regardless of BM/EN | `VA & 0x7fffffff` (SS-5) / full VA (SS-20) |
| ASI 21–2f | Physical bypass, regardless of BM/EN | `(ASI & 0xf) << 32 \| VA` |

`boot FILE` accepts a nonempty raw image of at most 1 MiB. The PROM aperture
starts at physical `0x70000000` for SS-5 and `0xff0000000` for SS-20/Axil;
unused bytes are filled with `0xff`, including an incomplete last word. ROM stores fail. Console serial and NVRAM/RTC are also
installed, together with interrupt control and counter timers. An ELF loader,
QEMU `fw_cfg` and complete firmware boot remain pending.

## Doubleword and atomic accesses

LDD/STD and their alternate forms require eight-byte alignment and an even
register-pair number. Helios takes the permitted illegal-instruction trap for
odd register pairs. The lower memory address supplies the even register; rd=0
still transfers the second word through g1. Sources and effective addresses are
captured before any destination register is written, including overlapping pairs.
An aligned doubleword fits in one MMU page and uses one translation.

The installed serial and NVRAM devices accept plain byte loads/stores only.
Interrupt, timer, reset/status and I/O configuration registers require aligned word loads/stores.
The processor user count at `0x71d00000` also accepts aligned doubleword
loads/stores while configured in user mode. Both halves form one transaction;
see [interrupts-timers.md](interrupts-timers.md). Other widths, partial masked
stores and atomic MMIO operations stop before device effects.
The [I/O control block](io-control.md) configures DMA translation and queues
I/O-only TLB flushes. CPU and DMA use the same 64-entry TLB; I/O mappings
ignore the CPU context and use their own writable bit and address range.

SWAP exchanges a word with rd; LDSTUB returns an unsigned byte and stores `0xff`
into that byte lane. Both operate even when rd=g0. SWAP requires word alignment;
LDSTUB accepts any byte address. The ASI must be an implemented memory space
(8–B or 20); applying a doubleword or atomic operation to MMU registers stops
explicitly before control-register side effects.

Each doubleword access or exchange is a single bus transaction. A rejected
transaction leaves the destination registers and target memory unchanged. This
is stricter than V8's permitted partial effects after some second-cycle machine
errors; Helios does not model those non-resumable errors. MMU fault status and
successful translation's R/M updates remain separate effects. SFSR/SFAR describe
the whole transaction at its effective virtual address; the bus error can also
identify a failing second physical word.

Atomic operations check store permission before touching the target and use the
store access type in SFSR. All writable data permissions also permit a read;
instruction-space exchanges require both execute and write. R and M are set on
successful translation. PCR.NF still records failures, returns zero for failed
loads/exchanges (two zero words for LDD), and suppresses target writes. It never
turns a read-only mapping into a writable one.

## Control spaces

| ASI | Implemented access |
| --- | --- |
| `03` | Word load: probe; word store: TLB flush |
| `04` | Word reads/writes of the registers below |
| `05` / `06` | SS-20/Axil I/D-TLB diagnostics. SEL 0–3 writes install a real reference-profile entry; SEL 5/6 expose a diagnostic PTP2 view. On SS-5, `06` is the microSPARC-II TLB diagnostic space |
| `08` / `09` | User / supervisor instruction space, byte/halfword/word/doubleword and atomics |
| `0a` / `0b` | User / supervisor data space, byte/halfword/word/doubleword and atomics |
| `0c` / `0e` | Word instruction/data cache diagnostic tag loads/stores |
| `0d` | Word instruction-cache diagnostic data loads/stores |
| `0f` | Byte/halfword/word/doubleword data-cache diagnostic data loads/stores |
| `10`–`14` | Word stores: conditionally invalidate one diagnostic tag in each cache |
| `20` | Physical bypass, byte/halfword/word/doubleword and atomics |
| `21`–`2f` | 36-bit physical bypass for SS-20/Axil; applies the ASI low nibble to PA[35:32] |
| `30`–`32`, `36`, `37`, `39` | SuperSPARC counter/store-buffer diagnostics accepted as no-op storage |
| `38`, `49`–`4c` | SuperSPARC MMU breakpoint/counter registers and action storage |
| `02` | SuperSPARC MXCC register subset; unrecognized addresses read zero, writes are ignored |

For ASI 4, VA bits 12:8 select a register and bits 7:0 are ignored after alignment
checking. Bits 31:13 must be zero. CTPR and replacement-control masks widen for
the SS-20/Axil reference profile (`0xffffffc0` and `0xffffffff`).

| Register address | Behavior |
| --- | --- |
| `0000` | Processor Control Register (PCR) |
| `0100` | Context Table Pointer Register, writable mask `0x00ffffc0` (SS-5) / `0xffffffc0` (SS-20/Axil) |
| `0200` | Context register, writable mask `0xff` |
| `0300` | Synchronous Fault Status Register (SFSR), read clears; writes ignored |
| `0400` | Synchronous Fault Address Register (SFAR), read clears; writes ignored |
| `1000` | TLB Replacement Control Register, mask `0x01f1ffff` (SS-5) / `0xffffffff` (SS-20/Axil); functional fill-driven counter |
| `1300` | SFSR diagnostic alias: reads retain state, writes masked by `0x00016fff` |
| `1400` | SFAR diagnostic alias: reads retain state, writes retain all 32 bits |

PCR.EN, NF and BM affect accesses. Cache/performance control bits in the mask
`0x003eff03` are retained; instruction/data memory stays coherent and no parity,
refresh timing or branch-fold timing is simulated. Bits outside that mask read
zero, apart from the fixed version. Enabling software table walking (bit 23) or
watchpoints (bit 22) stops as unsupported before changing the register. The
manual's PCR figure and reserved-bit prose disagree for some controls; broader
PCR diagnostics need hardware confirmation before claiming full compatibility.

Other ASIs, unsupported widths in control spaces, unimplemented registers and
unsupported controls produce an explicit emulator stop. Hardware control-space
fault synthesis is pending. Generic 36-bit bypass ASIs `21`–`2f` are enabled only
for the SS-20/Axil reference profile, not the microSPARC-II SS-5 profile.
Injected PTP walks, diagnostic M-bit rewalks, software table walks and
watchpoints are still pending.

## Page tables and TLB

The root entry address is `(CTPR << 4) | (context << 2)`. Reference-profile
PTEs carry a 36-bit physical page number; microSPARC-II PTEs remain 31-bit. Further indices select
VA bits 31:24, 23:18 and 17:12. Entry types are invalid (0), table pointer (1),
leaf (2) and reserved (3). Level 1/2/3 leaves map 16 MiB/256 KiB/4 KiB regions.
Physical page numbers use PTE bits 26:8. Table pointers can address all 31
physical bits, while the context table base is restricted to the low 256 MiB.
Software must align tables and large-page physical bases to their required sizes.

| ACC | User rights | Supervisor rights |
| --- | --- | --- |
| 0 | Read | Read |
| 1 | Read/write | Read/write |
| 2 | Read/execute | Read/execute |
| 3 | Read/write/execute | Read/write/execute |
| 4 | Execute | Execute |
| 5 | Read | Read/write |
| 6 | None | Read/execute |
| 7 | None | Read/write/execute |

Instruction-space stores require both execute and write permission. A user
access to ACC 6/7 produces a privilege fault. Other permission failures produce
a protection fault, before the denied target or PTE R/M bits are changed.
Successful translations set R; writes also set M, before the target access.
Updates use masked physical stores so unrelated PTE bits are preserved. Failure
to read a descriptor or update R/M is a translation fault at that table level.
A later target bus failure can therefore leave R/M set.

The TLB caches 64 leaf entries with context tags, supervisor-global matching
and decoded tag permissions checked on every access. Normal fills derive these
from ACC; diagnostic writes can edit them independently. ASI `06` shares the
actual entries. TRCR controls wrapping, counter freeze and reserved slots, using
a functional counter advanced on fills. Intermediate PTP caching, injected PTP
walks and detached M-bit rewalks remain pending; see [dram-tlb.md](dram-tlb.md).
Changes to cached page mappings require a flush. Context switches
retain tagged entries; PCR and CTPR writes leave TLB entries intact, matching
the reference MMU rule that a context-table-pointer change invalidates cached
PTPs but does not demap the TLB.

ASI 3 uses address bits 11:8 for the operation type and bits 31:12 for the
virtual page. Page, segment and region flushes (types 0–2) invalidate normal
TLB entries whose virtual page matches the selected extent and diagnostic tags
with matching Level-0 descriptors; context flush (type 3) invalidates entries
tagged with the current context; entire flush (type 4) invalidates all CPU
entries. I/O flush registers invalidate only I/O entries. Types 5–15 do
nothing. Flushes clear tag Valid while retaining diagnostic contents and the
replacement counter. Cache flush ASIs do not invalidate translations.

Probe types 0–3 request level 3/2/1/0 respectively. An earlier leaf returns zero
rather than satisfying a request for a deeper level. These probes invalidate
cached entries conservatively and do not set R. Type 4 returns the leaf from
the TLB or a table walk, setting R and caching it on a miss, without a permission
check. Invalid/reserved paths return zero; table bus failures return zero and
record a translation fault without delivering a CPU exception. Reserved probe
types return zero as the selected value for undefined results.

The microSPARC-II manual explicitly excludes root PTEs from TLB lookup, while
its probe table lists their readout. Root descriptors remain probeable, but a
normal translation ending in a root PTE is an explicit unsupported stop. No
4 GiB page support is claimed.

## Fault delivery

SFSR records FT (invalid, protection, privilege, translation or access bus), AT
(supervisor, instruction/data and read/write), walk level, FAV and overwrite.
SFAR contains the virtual address for data and translation faults. Physical
target errors use level zero because no descriptor access failed. SBus timeout
and slave error set TO/BE, including when preserved through a walk/probe fault.
Parity errors remain pending; see [fault-capture.md](fault-capture.md).

Translation faults have priority over data faults, which have priority over
instruction faults when updating the registers. A higher-priority new fault
replaces the saved status and clears OW; the same class replaces it and sets OW;
a lower-priority new fault leaves saved status/address intact. The CPU still
receives the new exception independently of which status was retained.

MMU failures use trap `0x01` for fetch and `0x09` for data instructions. LDA/STA
faults still use the data trap even when their ASI selects instruction space.
Physical target failures retain trap `0x21`/`0x29`. Misalignment uses `0x07`.
An exception with PSR.ET=0 enters CPU error state; ET does not imply no-fault mode.
Plain byte/halfword/word CPU SBus store errors complete asynchronously instead:
AFSR/AFAR capture the physical address/cause and the NCR module-error latch
requests level 15. These do not update SFSR/SFAR or depend on PCR.NF.

PCR.NF suppresses memory faults except in ASI 8/9, while still updating SFSR/SFAR.
A suppressed failed read returns zero; a failed store does nothing. Permission
checks are not relaxed. Unsupported emulator behavior and alignment faults
remain visible. These choices follow the microSPARC-II NF description rather
than QEMU's generic SPARC shortcut of suppressing faults when ET=0.

Device `Unsupported` and `WouldBlock` errors are emulator stops rather than
hardware access faults. Page walks, R/M updates and probes propagate them
without recording a fabricated translation failure; PCR.NF does not suppress
them. Earlier translation effects can remain, but the rejected device access
and instruction do not commit. The owner can drain output and retry after
backpressure; see [devices.md](devices.md).
