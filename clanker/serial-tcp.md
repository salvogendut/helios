# Loopback telnet serial consoles

Implemented 2026-11-07 at the user's request for an IRIS-style local serial
line. `src/serial_host.rs` owns a nonblocking `TcpListener` bound only to
`127.0.0.1`; frontends poll it between CPU steps, so no device worker, thread or
lock enters the emulation core. `TcpSerial::send` escapes `0xff` and queues
output per client (64 KiB cap, oldest bytes dropped); `receive` accepts clients,
filters telnet IAC negotiation, answers option changes without looping and
returns one guest-bound byte.

CLI `boot` gains `--serial-port-a PORT` and `--serial-port-b PORT`. Port zero
binds a free port and the actual port is printed to stderr. TTYA output still
goes to stdout; `--serial-in FILE`, NVRAM saving, trace separation and step
budget behavior are unchanged. `run` rejects the flags because its bare-metal
harness installs no SCC devices. IRIS's equivalent is
`serial_port_a`/`serial_port_b` on defaults 8880/8881; Helios keeps listeners
opt-in so existing tests and CI never bind host ports.

Initial telnet offer: WILL ECHO/SGA/BINARY and DO BINARY. Client-initiated
negotiation is consumed; subnegotiation payloads are discarded. The first
implementation kept the state machine in `Iac` while returning a doubled IAC
data byte, which swallowed the following character; `tests/serial_host.rs`
catches that regression.

Validation: five integration tests cover loopback bind/port reporting,
handshake, broadcast plus escaping, negotiation filtering, disconnect removal
and bind failure. A manual release run of original OBP with `--serial-port-a
18882` delivered the OBP trap/NVRAM transcript to a raw socket client. Full
`cargo test`, Clippy and demos were run after the change. No remote bind option
or telnet server-side line editing is claimed.
