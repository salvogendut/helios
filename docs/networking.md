# Host networking

Helios can attach a user-mode NAT to the SS5's NCR92C990 Ethernet endpoint.
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

`--net` enables ARP, RARP, BOOTP/DHCP, gateway ICMP echo, UDP NAT and DNS
relaying. `--net-gateway`, `--net-client`, `--net-mask` and `--net-dns`
override the addresses; `--tftp-dir` additionally enables the read-only TFTP
server and implies `--net`. The chosen addresses and TFTP root are printed to
stderr, and a frame/request summary is printed when the step budget ends the
run. `--serial-in` and telnet serial consoles are independent.

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
- ICMP echo requests to the gateway are answered locally. External ICMP is not
  relayed.
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
- External ICMP echo is not relayed, and guest IP fragments are dropped rather
  than reassembled.
- There is no PCAP bridge, NFS server, BOOTP lease database, DHCP NAK, IPv6,
  ARP cache, or host firewall integration.
- Only one guest identity is configured. TFTP transfer memory is bounded by a
  32 MiB file limit; a slow or stalled transfer expires after five unanswered
  retransmissions.
- NAT is functional, not a security boundary. It opens host UDP sockets to the
  destinations the guest requests.

## Validation

`tests/net.rs` covers ARP, RARP, DHCP offer/ack, BOOTP boot-file naming, ICMP
echo, UDP NAT against a local socket, DNS relay source-address rewriting, TFTP
block/ACK lock-step, path traversal rejection and retransmission. The engine
also has frame-level tests for header checksum acceptance. Firmware network
boot through the NAT remains to be recorded.
