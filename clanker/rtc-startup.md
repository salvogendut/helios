# RTC startup date (2026-11-16)

Developed on `fix/rtc-startup` after the SX console merge `c960604`.
The matching Heliacal branch starts from `d81d21c`.

## Cause and policy

The user's screenshot shows NetBSD rejecting TOD time, then displaying
2026-11-02 from the filesystem. The saved SS-20 NVRAM actually contains
`00 53 01 00 07 01 01 32` at 0x1ff8: 2000-01-01 00:01:53 with Sun's
1968 year origin. The existing device advances only with emulated time and
neither frontend previously seeded current time. This is separate from the
optional 85/75 MHz CPU timebase and the older timecounter-backwards warning.

NetBSD `mkclock.c` 1.21 and `kern_todr.c` 1.52 at local reference tree
`bc1ab70174b4c8f3c69457e0ab39d4cf3cf77147` confirm the 1968 origin,
2021 minimum and filesystem fallback. Those files are unmodified. References
are linked in `docs/references.md`; M48T08 latch/calendar rules are unchanged.

`Nvram::initialize_sun_clock` accepts caller-supplied UTC Unix seconds in
1970..2067. It replaces the eight RTC registers, releases transfer latches
and oscillator stop, and preserves configuration SRAM and IDPROM. Invalid
range is atomic. The device does not read host time. Library defaults,
image import, machine reset and guest software reset retain their prior
deterministic behavior.

CLI `--rtc host|saved|UNIX_SECONDS` exposes host seeding or fixed dates;
saved remains the CLI default. Heliacal defaults `rtc_host_time` to true for
new and old flat/named records. Its Tinker-gated Advanced `RTC at startup`
row offers Host UTC / Saved guest time. Save changes the draft/config only;
the next boot or confirmed F5 construction applies it to a private NVRAM
copy. Failed replacement cannot change the old machine's clock. The option
changes hardware equality without changing identity or NVRAM paths.

There is no continuous wall-time synchronization, pause catch-up, timezone
conversion or CPU-speed change. A guest may set its own time after startup;
Saved guest time preserves that date at subsequent cold constructions.

## Checks and evidence

- Four device regressions independently specify Unix epoch, leap-day, current
  November-16 and last representable date; SRAM/identity preservation,
  latch/stop replacement, range atomicity and core reset retention.
- Three CLI regressions cover fixed/saved/host modes, unchanged input files,
  all profiles, malformed/duplicate/range errors and no output on rejection.
- Three frontend regressions cover legacy/named-record defaults and roundtrip,
  transactional/gated menu behavior and actual worker construction on every
  profile with host or saved time, including invalid host dates.
- 665 unique default core tests, 683 with CHD/perf-counters; formatting,
  both Clippy configurations, all four demos and release CLI build pass.
- Heliacal: 78 tests, formatting, Clippy, rebuilt release and SDL smoke pass.
  Isolated SS-20/SX/256 MiB firmware display smoke passes after 53,727,232 steps
  at PC ffef0004. That smoke alone is not an OS acceptance claim.
- NetBSD 11.1 SS-20/SX/256 MiB displays `Mon Nov 16 08:26:29 UTC 2026`
  at boot, with no preposterous-TOD or filesystem-time fallback warning.
  This bounded date check stops during SSH key generation after its configured
  ntpdate attempt times out without a network endpoint. It is not a new
  login/clean-halt checkpoint; the earlier SX acceptance remains separate.
- Solaris 2.6 on a disposable SS-5/128 MiB disk reaches root and reports
  `Mon Nov 16 08:27:33 GMT 2026` from `date -u`, with no clock warning.
  `uname -a`, `HELIOS-RTC-OK`, sync and halt pass; OBP returns after
  1,311,550,000 steps. Timing under concurrent work is not a benchmark.

Ignored evidence: `local/rtc-startup-2026-11-16/`. Test disks are reflink
snapshots with SCSI ID 3 in their names. The NetBSD source was running in the
user's Heliacal; its disposable snapshot required normal fsck repairs. Those
repairs occur only on the copy, which also has no network endpoint and therefore
waits for its configured ntpdate request to fail. The original image and the
user's process are not attached or modified by these runners. Captures, logs,
source hashes, compact harness source and validation records are retained.
