# User-mode NAT stage one

Implemented 2026-11-07 at the user's request for the same kind of host backend
as IRIS. IRIS's `src/net/` is about 12k lines (ARP, DHCP, DNS, ICMP, UDP/TCP
NAT, TFTP/NFS, port forwards, optional PCAP). Helios starts with the subset
needed for guest address assignment and datagram networking.

Ownership choice: the engine owns no guest state and runs on the frontend
thread. `NatEngine::input` consumes guest frames, `poll(now)` services host
sockets and timers, and `output()` returns frames for
`Ethernet::receive_frame`. The CLI pumps it after each completed CPU step in
`src/main.rs`; the core does not call host networking. Host time is explicit so
device time stays deterministic and tests can force retransmission deadlines.
No worker thread, lock or channel is involved.

Packet code (`src/net/packet.rs`) is the host byte boundary: parsing and
building swap there only; the emulated NIC still moves raw byte frames. Checks
reject bad IPv4 header checksums and nonzero bad UDP checksums. Guest fragments
are dropped rather than reassembled; host replies larger than the MTU are
fragmented on 8-byte boundaries.

Stage-one services:

- ARP reply for gateway IP; MAC learned from any unicast source.
- RARP reply follows the captured original-OBP request
  (`local/probes/timer-ethernet-ss5/rarp-1.bin`): the reply's sender is the
  gateway (`spa=192.168.0.1`) and the target is the client (`tpa`). An early
  version put the client address in both fields; OBP then sent TFTP to its own
  address. The corrected reply makes it TFTP from the gateway.
- BOOTP/DHCP: DISCOVER->OFFER, REQUEST/INFORM->ACK; DHCP replies broadcast,
  plain BOOTP replies follow `ciaddr`/offered address. Option 6 advertises the
  gateway; DNS is relayed to the host resolver so a loopback system resolver is
  never handed to the guest.
- ICMP echo only for the gateway; TCP is counted and answered with ICMP
  protocol-unreachable instead of pretending a connection exists.
- UDP flows use connected nonblocking host sockets, bounded and expiring.
- DNS intercepts port 53 to any queried IP and rewrites the reply source to
  that IP, matching IRIS's resolver-transparency reasoning.
- TFTP is a pure datagram state machine in `src/net/tftp.rs`: octet/netascii,
  512-byte blocks, one-second retransmit, five tries, canonicalized read-only
  root, `/name` relative to the root, `..`/symlink escapes rejected. BOOTP only
  echoes a boot file that exists there.

Not implemented yet: TCP NAT, port forwards, external ICMP relay, fragment
reassembly, NFS, PCAP bridging, DHCP NAK/leases, IPv6. `docs/networking.md`
states these limits to users.

Validation: a release run of original OBP with `--net --tftp-dir` completed
RARP, requested `C0A80002.SUN4M`, fetched four TFTP blocks with ACKs, reported
server `192.168.0.1` / client `192.168.0.2`, and returned to `ok` after rejecting
the zero-filled file as non-executable. `tests/net.rs` has nine frame/socket
tests (ARP, RARP, DHCP offer/ack, BOOTP file naming, ICMP, UDP NAT, DNS source
rewrite, TFTP lock-step and traversal, TFTP retransmit). Full suite, Clippy,
demos and UI checks were run after integration. OBP reached its `ok` prompt;
no executable kernel was supplied, so no network OS boot is claimed.
