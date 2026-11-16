//! Linux echo relay. Each outstanding request owns an unprivileged ping socket,
//! so the kernel's identifier allocation separates guests and destinations.
//! No raw sockets, subprocesses or changes to host network policy are needed.
//! See docs/networking.md for the Linux/RFC sources and explicit limits.

use super::packet::{checksum, MacAddr};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

const LIMIT: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

pub(super) struct Reply {
    pub mac: MacAddr,
    pub guest: Ipv4Addr,
    pub remote: Ipv4Addr,
    pub payload: Vec<u8>,
}

struct Pending {
    socket: UdpSocket,
    mac: MacAddr,
    guest: Ipv4Addr,
    remote: Ipv4Addr,
    host_id: u16,
    request: Vec<u8>,
    expires: Instant,
}

impl Pending {
    fn reply(&self, source: IpAddr, packet: &[u8]) -> Option<Reply> {
        if source != self.remote
            || packet.len() != self.request.len()
            || packet.len() < 8
            || packet[..2] != [0, 0]
            || checksum(packet) != 0
            || packet[4..6] != self.host_id.to_be_bytes()
            || packet[6..] != self.request[6..]
        {
            return None;
        }
        let mut payload = packet.to_vec();
        payload[4..6].copy_from_slice(&self.request[4..6]);
        payload[2..4].fill(0);
        let sum = checksum(&payload);
        payload[2..4].copy_from_slice(&sum.to_be_bytes());
        Some(Reply {
            mac: self.mac,
            guest: self.guest,
            remote: self.remote,
            payload,
        })
    }
}

#[derive(Default)]
pub(super) struct Poll {
    pub replies: Vec<Reply>,
    pub errors: u64,
    pub expired: u64,
}

#[derive(Default)]
pub(super) struct Relay {
    pending: Vec<Pending>,
    next_poll: Option<Instant>,
    disabled: bool,
    warned: bool,
    warning: Option<String>,
}

impl Relay {
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    fn warn(&mut self, error: &io::Error) {
        if !self.warned {
            self.warning = Some(format!("External ICMP echo unavailable: {error}"));
            self.warned = true;
        }
    }

    pub fn send(
        &mut self,
        mac: MacAddr,
        guest: Ipv4Addr,
        remote: Ipv4Addr,
        ttl: u8,
        request: &[u8],
        now: Instant,
    ) -> bool {
        if self.disabled || self.pending.len() >= LIMIT {
            return false;
        }
        let socket = match echo_socket() {
            Ok(socket) => socket,
            Err(error) => {
                self.disabled = matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
                );
                self.warn(&error);
                return false;
            }
        };
        let sent = (|| {
            socket.connect(SocketAddrV4::new(remote, 0))?;
            socket.set_ttl(u32::from(ttl))?;
            let host_id = socket.local_addr()?.port();
            if socket.send(request)? != request.len() {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "short ICMP send"));
            }
            Ok(host_id)
        })();
        match sent {
            Ok(host_id) => {
                self.pending.push(Pending {
                    socket,
                    mac,
                    guest,
                    remote,
                    host_id,
                    request: request.to_vec(),
                    expires: now + TIMEOUT,
                });
                // Poll immediately for a newly sent request, then at most once
                // per millisecond while waiting; never recv on every CPU step.
                self.next_poll = Some(now);
                true
            }
            Err(error) => {
                self.warn(&error);
                false
            }
        }
    }

    pub fn poll(&mut self, now: Instant, buffer: &mut [u8]) -> Poll {
        let mut result = Poll::default();
        if self.pending.is_empty() || self.next_poll.is_some_and(|next| now < next) {
            return result;
        }
        self.next_poll = Some(now + POLL_INTERVAL);
        self.pending.retain_mut(|pending| {
            if now >= pending.expires {
                result.expired += 1;
                return false;
            }
            // Invalid/unrelated input cannot keep the frontend in an unbounded
            // receive loop. A valid reply consumes the request exactly once.
            for _ in 0..4 {
                match pending.socket.recv_from(buffer) {
                    Ok((length, source)) => {
                        if let Some(reply) = pending.reply(source.ip(), &buffer[..length]) {
                            result.replies.push(reply);
                            return false;
                        }
                        result.errors += 1;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        result.errors += 1;
                        return false;
                    }
                }
            }
            true
        });
        result
    }
}

#[cfg(all(feature = "host-icmp", target_os = "linux"))]
fn echo_socket() -> io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::ICMPV4))?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into())?;
    // Safe owning conversion: std's datagram I/O also works for a Linux ping
    // socket. recv_from retains the real sender address for reply validation.
    Ok(socket.into())
}

#[cfg(not(all(feature = "host-icmp", target_os = "linux")))]
fn echo_socket() -> io::Result<UdpSocket> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "requires a Linux host and the host-icmp Cargo feature",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(ty: u8, id: u16) -> Vec<u8> {
        let mut p = vec![ty, 0, 0, 0, 0, 0, 0xff, 0xfe, 0, 0xff, 0x41];
        p[4..6].copy_from_slice(&id.to_be_bytes());
        fix_sum(&mut p);
        p
    }

    fn fix_sum(p: &mut [u8]) {
        p[2..4].fill(0);
        let sum = checksum(p);
        p[2..4].copy_from_slice(&sum.to_be_bytes());
    }

    fn pending(now: Instant) -> Pending {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket.set_nonblocking(true).unwrap();
        Pending {
            socket,
            mac: [2, 3, 4, 5, 6, 7],
            guest: Ipv4Addr::new(192, 168, 0, 2),
            remote: Ipv4Addr::LOCALHOST,
            host_id: 0x1357,
            request: packet(8, 0x2468),
            expires: now + TIMEOUT,
        }
    }

    #[test]
    fn reply_requires_the_real_peer_identifier_sequence_data_and_checksum() {
        let p = pending(Instant::now());
        let original = p.request.clone();
        let good = packet(0, p.host_id);
        let source = IpAddr::V4(p.remote);
        assert!(p.reply(Ipv4Addr::new(127, 0, 0, 2).into(), &good).is_none());
        assert!(p.reply(source, &good[..7]).is_none());
        for index in [0, 1, 4, 6, 8] {
            let mut wrong = good.clone();
            wrong[index] ^= 1;
            fix_sum(&mut wrong);
            assert!(p.reply(source, &wrong).is_none(), "offset {index}");
        }
        let mut corrupt = good.clone();
        corrupt[2] ^= 1;
        assert!(p.reply(source, &corrupt).is_none());
        let mut extra = good.clone();
        extra.push(0);
        fix_sum(&mut extra);
        assert!(p.reply(source, &extra).is_none());
        assert_eq!(p.request, original);
        let reply = p.reply(source, &good).unwrap();
        assert_eq!(reply.mac, p.mac);
        assert_eq!((reply.remote, reply.guest), (p.remote, p.guest));
        assert_eq!(reply.payload, packet(0, 0x2468));
        assert_eq!(checksum(&reply.payload), 0);
    }

    #[test]
    fn polling_is_throttled_and_a_reply_consumes_the_pending_socket_once() {
        let now = Instant::now();
        let p = pending(now);
        let destination = p.socket.local_addr().unwrap();
        let mut relay = Relay {
            pending: vec![p],
            ..Relay::default()
        };
        let mut buffer = [0; 128];
        assert!(relay.poll(now, &mut buffer).replies.is_empty());
        let host = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        host.send_to(&packet(0, 0x1357), destination).unwrap();
        assert!(relay.poll(now, &mut buffer).replies.is_empty());
        let result = relay.poll(now + POLL_INTERVAL, &mut buffer);
        assert_eq!(result.replies.len(), 1);
        assert_eq!((result.expired, result.errors, relay.len()), (0, 0, 0));
        assert!(relay
            .poll(now + POLL_INTERVAL * 2, &mut buffer)
            .replies
            .is_empty());
    }

    #[test]
    fn request_budget_and_deadline_release_resources_without_false_replies() {
        let now = Instant::now();
        let mut relay = Relay {
            pending: (0..LIMIT).map(|_| pending(now)).collect(),
            ..Relay::default()
        };
        assert!(!relay.send(
            [2; 6],
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::LOCALHOST,
            63,
            &packet(8, 1),
            now
        ));
        assert_eq!(relay.len(), LIMIT);
        // Even a queued answer is not accepted beyond the request's lifetime.
        let host = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        host.send_to(
            &packet(0, 0x1357),
            relay.pending[0].socket.local_addr().unwrap(),
        )
        .unwrap();
        let result = relay.poll(now + TIMEOUT, &mut [0; 128]);
        assert!(result.replies.is_empty());
        assert_eq!(result.expired, LIMIT as u64);
        assert_eq!(relay.len(), 0);
    }
}
