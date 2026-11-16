# External ICMP echo

The reported `ping www.google.com` failure exposed a missing NAT feature:
gateway echo was implemented, but external ICMP requests were ignored.

## Implementation

`src/net/icmp.rs` owns a bounded Linux datagram-ICMP relay behind `host-icmp`.
Each request has its own unprivileged ping socket, five-second deadline and
original guest MAC/IP/identifier/sequence/data. A reply must match the actual
peer, kernel-assigned identifier, length, sequence, payload and checksum before
the guest identifier/checksum are restored. No synthetic success, raw sockets,
ping helper process, host sysctl mutation or unsafe code was added.

The 64-request cap and one-millisecond polling interval bound resources and
avoid a receive syscall per CPU step. Gateway requests now also validate ICMP
code/checksum. Counters distinguish gateway replies, external sends/replies,
errors and timeouts. CLI and Heliacal drain a once-only host-error diagnostic.
Heliacal enables the optional feature; core default builds remain dependency
free. `socket2` 0.6.5 supports the core's Rust 1.74 baseline; CI tests stable and
1.74 with real loopback ping sockets. The disposable CI runner's ping-group
policy is explicitly configured; the emulator never changes users' host policy.

Sources and limits are in `docs/networking.md`. TCP, IPv6, traceroute/error
translation and guest fragment reassembly remain unsupported. ICMP data is
limited to 1472 bytes. Reply IP TTL is the NAT's 64, not the remote observed TTL.

## Verification

Evidence: ignored `local/icmp-2026-11-16/`.

- Normal suite: 671 unique passing tests (672 lines including the isolated MXCC
  child); `chd,perf-counters,host-icmp`: 688 unique passing tests (689 lines).
- Real Linux echo regression, explicitly run with `--ignored`: four concurrent
  requests, multiple guest IP/MAC identities and destinations, repeated IDs and
  sequences, zero/odd/full-MTU payloads. All four returned correctly.
- Relay fixtures cover invalid peer/identifier/sequence/data/type/code/length/
  checksum, one-shot reply consumption, poll throttling, deadlines and capacity.
  NAT fixtures cover malformed input, exhausted TTL, non-unicast/oversized
  requests, and feature-off diagnostics without false replies.
- Formatting, default and combined-feature Clippy, four demos and the separate
  `helios-ui` locked check passed. Heliacal: 78 tests, Clippy, formatting, release
  rebuild and SDL dummy/software smoke passed.
- `examples/net_ping.rs` received three actual replies from Google through the
  relay, preserving identifier `0x1234` and 27 binary payload bytes. This is a
  host/NAT probe, separate from the OS tests below.
- Actual Heliacal SS-20/NetBSD 11.1 single-user boot on a disposable CHD clone:
  gateway, numeric Google address and `www.google.com` each returned 3/3 with
  zero loss, followed by clean guest halt and frontend Quit=0. This run used
  the final release binary with packet instrumentation removed.
- Actual Heliacal SS-5/Solaris 2.6 boot on a disposable raw clone: gateway,
  numeric Google address and `www.google.com` each returned 3/3 with zero loss
  using the final release. Google hostname RTT was 38–39 guest ms. Guest
  hostname lookup initially failed before emitting any DNS query because
  `/etc/nsswitch.conf` was empty. Adding `hosts: files dns` enabled lookup via
  `nameserver 192.168.0.1` in `/etc/resolv.conf`; no DNS-relay code change was
  needed. The guest was cleanly halted afterward.

NetBSD's first attempt ran while its IPv4 address was `TENTATIVE`; after
duplicate-address detection completed, no Ethernet/device fix was necessary.
The final harness waits for address readiness. A diagnostic rerun also exposed
a stale test PTY alias after forced cleanup; the harness now removes only its
own aliases and waits for visible X windows. Temporary packet logging was
removed from the frontend. No firmware, CPU, MMU or Ethernet register behavior
changed.

The user's `local/disks/netbsd11-ss20-scsi3.chd` and source Solaris image are
preserved. Tests use separate configs, PTYs, Xvfb instances and reflink clones;
they do not change the live machine catalog or user's running Heliacal.
Both final guests halted and their frontends exited with status zero. The test
disk clones, temporary NVRAM and trace-source backup were removed; compact
logs, reproduction scripts, configurations and screenshots remain.
