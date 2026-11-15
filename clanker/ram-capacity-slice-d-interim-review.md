# Configurable RAM: slice D interim review

Superseded by [the completion review](ram-capacity-completion.md).

Reviewed completed evidence on 2026-11-15 while the implementation agent was
still running slice D. This is an interim review, not acceptance of its
unfinished work. No production source, active-run disk/configuration or source
media was changed. Slices A/B/C remain accepted at their previous stopping points.

## Completed evidence that can be reused

The serial logs under `local/ram-capacity-2026-11-15/runs/` show maximum-size
Solaris root sessions and clean halts for SS-5 256 MiB, SS-20/SX 448 MiB,
SS-20/CGsix 512 MiB and Axil 512 MiB. The four 16 MiB files have matching
`cksum` results after reboot: `4215202376 16777216`. The SS-20/Axil runs include
processor 0/2 online reports. These are observations from completed logs, not
fresh full OS reruns by this review.

Independently decoded the four `*-reg.serial` captures and checked exact physical
`reg` tuples: eight 32 MiB SS-5 banks, seven 64 MiB SS-20/SX banks ending before
bank 7, and eight 64 MiB banks for SS-20/CGsix and Axil. The producer's awk also
matches `virtual-memory`; its second `MEMREG` record describes a 4 GiB virtual
address space and must not be counted as installed DRAM.

The saved SS-20/SX 448 MiB frontend capture shows a root console, typed commands,
the correct Solaris RAM report, and CPUs 0/2 online. Other frontend captures
must be assessed individually; a filename such as `console-root` or `halted`
does not establish that checkpoint.

## Finding 1: Axil mixed-module discovery disagrees with the allocation

The existing `obp-axil-416.serial` reports 448 MiB, and `obp-axil-480.serial`
reports 512 MiB. To exclude a mislabeled run, this review invoked the existing
acceptance probe with separately recorded arguments, private copied NVRAM and
a private 512-byte empty target; no OS disk was attached:

| Explicit constructor input | Firmware banner | Result |
| --- | --- | --- |
| Axil / CGsix / 1 CPU / 416 MiB | 448 MiB | Mismatch reproduced |
| Axil / CGsix / 1 CPU / 80 MiB | 80 MiB | Control passes |
| SS-20 / CGsix / 1 CPU / 416 MiB | 416 MiB | Control passes |

The current largest-module-first layout represents 416 MiB as six 64 MiB
modules plus one 32 MiB module. The pattern points toward the Axil ROM's
32 MiB detection path or the assumed module geometry; it does not establish
which sizes the physical Axil supports. The 80 MiB control uses 64+16 MiB.

Resolve this before advertising every currently enumerated Axil size as usable.
Trace the 32 MiB probe and capture the physical memory property with a command
supported by this ROM. The review's attempted `dev /memory .properties
device-end` returned `dev ?`, so these repeat probes establish the banner
disagreement only, not the mixed-size `reg` entries. Preserve this distinction.
Determine whether the emulated mapping needs correction or the Axil layout and
valid-total enumeration need a separately justified module set. Do not patch
the ROM or conceal the disagreement in the displayed RAM label. Keep the
existing documented primary-source evidence gap explicit.

## Finding 2: SS-20/CGsix 512 MiB frontend boot panics with one or two CPUs

The two-CPU `gui-ss20-cgsix512/console-root.png` capture contains
`panic: Data fault` followed by reboot, rather than a root console.

The one-CPU control subsequently completed. Both `console-1cpu.png` and
`halted-1cpu.png` show the same kind of panic/reboot. The visible fault context
includes:

```text
pte addr = 0xf5984000, level = 1
pid=0, pc=0xf00f15d4, sp=0xf02436f8, psr=0x404000c2, context=0
panic: Data fault
```

This is not confined to SMP. Whether RAM capacity introduced it is still
unknown. The next useful comparison is the same graphics-console setup with
128 MiB and one CPU, keeping disk/NVRAM/ROM identical. Capture the first failing
guest access and its CPU, PC/nPC, SFSR/SFAR, virtual/physical address and device
decode. Symbolize against the exact guest kernel and loaded modules; the local
`genunix.elf` is relocatable, so guessing a load base is insufficient.

The serial-console 512 MiB passes remain valid. They do not establish that the
graphics-console driver initialization works. The host process remaining alive
and the harness printing `done` likewise do not establish guest command success
or a clean guest halt. Replace repeated fixed-delay retries with checkpoints
that inspect guest progress and stop on the first panic.

## Still missing from the inspected evidence

- The maximum-size command files exercise disk I/O with a 64 KiB buffer and
  16 MiB file. That is not the planned bounded guest allocation/touch workload.
  Add an allocation test that writes and verifies its pages; do not claim this
  alone proves physical addresses above 256 MiB were used.
- Small/mixed-module physical `reg` comparisons are incomplete. The saved mixed
  runs mostly issue `banner`; their prompt/banner success is narrower evidence.
- The saved Axil maximum-size `console-root.png` was blank when reviewed;
  interactive acceptance is not established by that capture. The SS-5 maximum
  frontend and full F9/named-machine/reset acceptance also need their own evidence.
- The complete final validation, documentation reconciliation and cleanup remain
  with the implementation agent. Do not remove its active clones or scratch work
  as part of this interim review.

## Evidence and handover

`local/review-ram-slice-d-2026-11-15/` retains the independent probe commands,
binary hash, serial output, logs, physical-memory-map comparisons and copies of
the panic captures. This review's empty target and NVRAM copies were removed.
No new build or full operating-system clone was created.

The one-CPU GUI control ended with the host Quit action, making this a useful
handover point if requested. Preserve the completed runs and source hashes;
focus further work on the CGsix first-fault trace and Axil 32 MiB detection
before repeating the full acceptance matrix. Issue #3 is not ready to merge or
close on the inspected evidence.
