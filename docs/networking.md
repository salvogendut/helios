# Host networking

Helios can attach a user-mode NAT to each machine's NCR92C990 Ethernet endpoint.
The engine is host code outside the emulation core: the frontend drains guest
frames with `Ethernet::take_frame`, calls `NatEngine::input`, polls host sockets
with `NatEngine::poll`, and injects returned frames with
`Ethernet::receive_frame`. The CLI does this after every completed CPU step.
Host time is supplied by the caller; emulator device time does not depend on
host scheduling or network activity.

This is the first stage of IRIS-style host networking. It does not bridge onto
a host interface and it does not expose an inbound listener.

## Defaults and CLI

```sh
cargo run --release -- boot local/firmware/sun-ss5.bin --steps 1100000000 \
  --net --tftp-dir local/tftp
```

| Setting | Default |
| --- | --- |
| Guest address | `192.168.0.2` |
| Gateway | `192.168.0.1` |
| Netmask | `255.255.255.0` |
| Gateway MAC | `02:00:de:ad:be:ef` |
| DNS upstream | first IPv4 nameserver in `/etc/resolv.conf`, else `8.8.8.8` |

`--net` enables ARP, RARP, BOOTP/DHCP, ICMP echo, UDP NAT and DNS
relaying. `--net-gateway`, `--net-client`, `--net-mask` and `--net-dns`
override the addresses; `--tftp-dir` additionally enables the read-only TFTP
server and implies `--net`. The chosen addresses and TFTP root are printed to
stderr, and a frame/request summary is printed when the step budget ends the
run. `--serial-in` and telnet serial consoles are independent.

Gateway ping works in every build. External IPv4 echo also needs a Linux host
and the optional `host-icmp` feature (enabled by Heliacal):

```sh
cargo run --release --features host-icmp -- boot local/firmware/sun-ss5.bin \
  --net --steps 1100000000
# Exercise the NAT without booting firmware or an OS:
cargo run --release --features host-icmp --example net_ping -- www.google.com
```

The Linux `net.ipv4.ping_group_range` setting must permit one of the process's
groups to open an unprivileged ping socket. Check `id -G` and
`sysctl net.ipv4.ping_group_range` if the host reports permission denied.
Helios logs a diagnostic once when echo forwarding is unavailable. It does not
change host policy, request root, use raw sockets, or launch a ping subprocess.

## Guest checks

First check `192.168.0.1`, then an external IPv4 address, then a hostname. For
example, Solaris uses `ping -s www.google.com 56 3`; NetBSD uses
`ping -n -c 3 www.google.com`. Both were tested through Heliacal using disposable
Solaris 2.6 SS-5 and NetBSD 11.1 SS-20 disks: gateway, numeric Google address and
Google hostname each returned three replies with zero loss.

With a static address, configure `le0` as `192.168.0.2/24`, set the default
route to `192.168.0.1`, and use `nameserver 192.168.0.1` in the guest's
`/etc/resolv.conf`. On NetBSD, wait until the address no longer appears as
`TENTATIVE` in `ifconfig le0 inet` before checking connectivity.

Solaris also needs a `hosts: files dns` entry in `/etc/nsswitch.conf`; an empty
file or a files-only entry can produce `unknown host` without sending a DNS
query. Preserve the other service entries when editing it. Sun's
[name-service setup guide](https://docs.oracle.com/cd/E19455-01/806-1386/6jam5ahkd/index.html)
describes the setting and reboot requirement for cached lookup configuration.
These are guest configuration steps; Heliacal does not rewrite guest files.

## Implemented behavior

- ARP replies advertise the gateway MAC for the gateway address. The guest MAC
  is learned from frames. RARP replies carry the gateway as sender and the
  configured guest address as target, so the PROM learns both the boot server
  and its own address.
- BOOTP and DHCP replies carry the configured address, server identifier,
  netmask, router, broadcast address, lease and DNS option. DHCP DISCOVER is
  offered, REQUEST and INFORM are acknowledged, and replies are broadcast for
  DHCP or unicast to `ciaddr`/the offered address for plain BOOTP. Option 6
  advertises the gateway, whose DNS traffic is relayed.
- Valid ICMP echo requests to the gateway are answered locally. On Linux with
  `host-icmp`, external requests use nonblocking datagram ICMP sockets, one per
  outstanding request. Linux assigns the host identifier; the relay restores
  the guest identifier and checksum after validating the sender, type/code,
  checksum, sequence, payload and length. Replies retain the requesting guest's
  IP and MAC even when another guest frame has since arrived. Requests expire
  after five host seconds, with at most 64 outstanding. Pending sockets are
  polled at most once per millisecond, except immediately after a new send.
  Error, unavailable and timeout cases never produce synthetic echo replies.
- Guest UDP flows are mapped to nonblocking host UDP sockets. Replies are
  re-addressed to the guest; the map is bounded (128 flows) and idle flows
  expire after 120 s.
- DNS queries to any destination port 53 are relayed to the configured host
  resolver, and the answer's source address is rewritten to the address the
  guest queried. The relay map is bounded (32 flows) and expires after 30 s.
- TFTP is read-only, serves octet and netascii, uses 512-byte blocks with
  one-second retransmission up to five tries, and confines names to the
  configured root (`/name` is relative to the root, `..` and symlink escapes
  are rejected). BOOTP names a requested boot file only if it exists under that
  root.
- Replies larger than one Ethernet MTU are fragmented at 8-byte boundaries.
  Ethernet frames are padded to the 60-byte minimum.

## Explicit limits

- TCP NAT and host-to-guest port forwards are later stages. TCP packets are
  counted, dropped, and answered with ICMP protocol-unreachable so guest
  connection attempts fail instead of hanging.
- External ICMP support is IPv4 echo only. ICMP error translation, traceroute,
  IPv6 and guest fragment reassembly are not implemented. Echo data is limited
  to 1472 bytes (the 1500-byte Ethernet MTU minus IP/ICMP headers). External
  requests with exhausted TTL are dropped; forwarded TTL is decremented once.
  Reply IP headers use the NAT's TTL of 64, not the remote system's observed TTL.
  Non-Linux hosts and builds without `host-icmp` retain gateway ping only.
- There is no PCAP bridge, NFS server, BOOTP lease database, DHCP NAK, IPv6,
  ARP cache, or host firewall integration.
- Only one guest identity is configured. TFTP transfer memory is bounded by a
  32 MiB file limit; a slow or stalled transfer expires after five unanswered
  retransmissions.
- NAT is functional, not a security boundary. It opens host UDP and, when
  enabled, ICMP sockets to destinations the guest requests.

## Validation

`tests/net.rs` covers ARP, RARP, DHCP offer/ack, BOOTP boot-file naming, ICMP
echo, UDP NAT against a local socket, DNS relay source-address rewriting, TFTP
block/ACK lock-step, path traversal rejection and retransmission. The engine
also has frame-level tests for header checksum acceptance. Firmware network
boot through the NAT remains to be recorded.

Relay unit tests cover reply matching, deadlines, request limits and bounded
polling. The explicit Linux test exercises real kernel echo with concurrent
guest identities, duplicate identifiers/sequences and payload sizes through
1472 bytes; it needs ping-socket permission:

```sh
cargo test --features host-icmp --test net host_icmp_loopback -- --ignored
```

## Sources

- [RFC 792](https://www.rfc-editor.org/rfc/rfc792.html): echo request/reply fields,
  payload preservation and checksums.
- [Linux IP sysctls](https://docs.kernel.org/networking/ip-sysctl.html):
  `ping_group_range` and group-based access to datagram ICMP sockets.
- [Linux v6.12 ping.c](https://github.com/torvalds/linux/blob/v6.12/net/ipv4/ping.c):
  socket identifier allocation/replacement and received ICMP packet layout.
- IRIS `src/net/nat_icmp.rs` was inspected as a sibling host-relay reference;
  Helios uses a bounded per-request implementation and validates the real sender.
