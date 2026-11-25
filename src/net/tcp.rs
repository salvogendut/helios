//! Outbound IPv4 TCP proxy. smoltcp handles the guest half (windows, sequence
//! space, retransmission and FIN); the OS handles the independent host half.
mod device;
use super::packet::{ipv4_packet, pseudo_checksum, Ipv4, MacAddr, PROTO_TCP};
use device::Port;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::ChecksumCapabilities;
use smoltcp::socket::tcp::{Socket, SocketBuffer, State};
use smoltcp::time::Instant as StackTime;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, TcpControl, TcpPacket, TcpRepr};
use socket2::{Domain, Protocol, Socket as HostSocket, Type};
use std::collections::{hash_map::RandomState, HashMap};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

const MAX_FLOWS: usize = 32;
const BUFFER_SIZE: usize = 16 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
struct Key {
    mac: MacAddr,
    guest: SocketAddrV4,
    remote: SocketAddrV4,
}

struct Opening {
    host: TcpStream,
    syn: Vec<u8>,
    deadline: Instant,
}

struct Connected {
    host: TcpStream,
    write_closed: bool,
    port: Port,
    interface: Interface,
    sockets: SocketSet<'static>,
    handle: SocketHandle,
    last_activity: Instant,
}

type Replies = Vec<(MacAddr, Vec<u8>)>;

#[derive(Default)]
pub(super) struct Relay {
    pending: HashMap<Key, Opening>,
    flows: HashMap<Key, Connected>,
    origin: Option<Instant>,
    next_poll: Option<Instant>,
}

fn stack_time(origin: Instant, now: Instant) -> StackTime {
    StackTime::from_millis(now.saturating_duration_since(origin).as_millis() as i64)
}

fn nonblocking_connect(remote: SocketAddrV4) -> io::Result<TcpStream> {
    let host = HostSocket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    host.set_nonblocking(true)?;
    match host.connect(&remote.into()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
        #[cfg(unix)]
        Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => {}
        Err(e) => return Err(e),
    }
    Ok(host.into())
}

impl Relay {
    pub(super) fn input(
        &mut self,
        mac: MacAddr,
        ip: &Ipv4<'_>,
        raw: &[u8],
        gateway: Ipv4Addr,
        now: Instant,
        replies: &mut Replies,
    ) -> bool {
        let Ok(packet) = TcpPacket::new_checked(ip.payload()) else {
            return false;
        };
        let src = IpAddress::Ipv4(ip.source.into());
        let dst = IpAddress::Ipv4(ip.destination.into());
        let Ok(segment) = TcpRepr::parse(&packet, &src, &dst, &ChecksumCapabilities::default())
        else {
            return false;
        };
        if segment.src_port == 0
            || segment.dst_port == 0
            || !src.is_unicast()
            || !dst.is_unicast()
            || raw.len() > 1500
        {
            return false;
        }
        let key = Key {
            mac,
            guest: SocketAddrV4::new(ip.source, segment.src_port),
            remote: SocketAddrV4::new(ip.destination, segment.dst_port),
        };
        let tick = stack_time(*self.origin.get_or_insert(now), now);
        if let Some(flow) = self.flows.get_mut(&key) {
            flow.port.input = Some(raw.to_vec());
            flow.last_activity = now;
            flow.dispatch(tick, mac, replies);
            return true;
        }
        if self.pending.contains_key(&key) {
            if packet.rst() {
                self.pending.remove(&key);
            }
            return true;
        }
        if segment.control == TcpControl::Syn
            && segment.ack_number.is_none()
            && !ip.destination.is_loopback()
            && ip.destination != gateway
            && self.pending.len() + self.flows.len() < MAX_FLOWS
        {
            if let Ok(host) = nonblocking_connect(key.remote) {
                self.pending.insert(
                    key,
                    Opening {
                        host,
                        syn: raw.to_vec(),
                        deadline: now + CONNECT_TIMEOUT,
                    },
                );
                self.next_poll = None;
                return true;
            }
        }
        reset(key, &packet, replies);
        true
    }

    pub(super) fn poll(&mut self, now: Instant, replies: &mut Replies) {
        if (self.pending.is_empty() && self.flows.is_empty())
            || self.next_poll.is_some_and(|due| now < due)
        {
            return;
        }
        self.next_poll = Some(now + POLL_INTERVAL);
        let tick = stack_time(*self.origin.get_or_insert(now), now);
        // Remove completed connects only. A retransmitted SYN reuses the host socket.
        let mut finished = Vec::new();
        for (key, open) in &self.pending {
            if now >= open.deadline || !matches!(open.host.take_error(), Ok(None)) {
                finished.push((*key, false));
            } else if open.host.peer_addr().is_ok() {
                finished.push((*key, true));
            }
        }
        for (key, established) in finished {
            let open = self.pending.remove(&key).unwrap();
            if established {
                let mut flow = Connected::new(key, open.host, now, tick);
                flow.port.input = Some(open.syn);
                flow.dispatch(tick, key.mac, replies);
                self.flows.insert(key, flow);
            } else {
                let ip = Ipv4::parse(&open.syn).unwrap();
                reset(key, &TcpPacket::new_unchecked(ip.payload()), replies);
            }
        }
        self.flows.retain(|key, flow| {
            let socket = flow.sockets.get_mut::<Socket>(flow.handle);
            if now.saturating_duration_since(flow.last_activity) >= IDLE_TIMEOUT {
                socket.abort();
            } else {
                match pump(socket, &mut flow.host, &mut flow.write_closed) {
                    Ok(true) => flow.last_activity = now,
                    Err(_) => socket.abort(),
                    Ok(false) => {}
                }
            }
            flow.dispatch(tick, key.mac, replies);
            !matches!(
                flow.sockets.get::<Socket>(flow.handle).state(),
                State::Closed | State::Listen
            )
        });
    }
}

// RFC 9293 §3.10.7.1 (CLOSED). In particular, never answer a reset with a reset.
fn reset(key: Key, packet: &TcpPacket<&[u8]>, replies: &mut Replies) {
    if packet.rst() {
        return;
    }
    let mut out = [0u8; 20];
    let mut tcp = TcpPacket::new_unchecked(&mut out[..]);
    tcp.set_header_len(20);
    tcp.set_src_port(key.remote.port());
    tcp.set_dst_port(key.guest.port());
    tcp.set_rst(true);
    if packet.ack() {
        tcp.set_seq_number(packet.ack_number());
    } else {
        tcp.set_ack(true);
        tcp.set_ack_number(packet.seq_number() + packet.segment_len());
    }
    let sum = pseudo_checksum(*key.remote.ip(), *key.guest.ip(), PROTO_TCP, &out);
    out[16..18].copy_from_slice(&sum.to_be_bytes());
    replies.push((
        key.mac,
        ipv4_packet(*key.remote.ip(), *key.guest.ip(), PROTO_TCP, 0, 64, &out),
    ));
}

impl Connected {
    fn new(key: Key, host: TcpStream, now: Instant, tick: StackTime) -> Self {
        let mut port = Port::default();
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = RandomState::new().build_hasher().finish();
        let mut interface = Interface::new(config, &mut port, tick);
        let remote = IpAddress::Ipv4((*key.remote.ip()).into());
        // This IP-only interface represents one remote endpoint to the guest.
        interface.update_ip_addrs(|addrs| addrs.push(IpCidr::new(remote, 0)).unwrap());
        let mut socket = Socket::new(
            SocketBuffer::new(vec![0; BUFFER_SIZE]),
            SocketBuffer::new(vec![0; BUFFER_SIZE]),
        );
        socket.listen((remote, key.remote.port())).unwrap();
        socket.set_ack_delay(None);
        let mut sockets = SocketSet::new(vec![]);
        let handle = sockets.add(socket);
        Self {
            host,
            write_closed: false,
            port,
            interface,
            sockets,
            handle,
            last_activity: now,
        }
    }
    fn dispatch(&mut self, tick: StackTime, mac: MacAddr, replies: &mut Replies) {
        self.interface.poll(tick, &mut self.port, &mut self.sockets);
        replies.extend(self.port.output.drain(..).map(|p| (mac, p)));
    }
}

fn transfer(result: io::Result<usize>) -> (usize, io::Result<Option<usize>>) {
    match result {
        Ok(n) => (n, Ok(Some(n))),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            (0, Ok(None))
        }
        Err(e) => (0, Err(e)),
    }
}

fn pump(
    socket: &mut Socket<'_>,
    host: &mut TcpStream,
    write_closed: &mut bool,
) -> io::Result<bool> {
    let mut progress = false;
    if socket.can_recv() {
        // Dequeue exactly the bytes accepted by the host, including short writes.
        match socket.recv(|bytes| transfer(host.write(bytes))).unwrap()? {
            Some(0) => return Err(io::ErrorKind::WriteZero.into()),
            Some(_) => progress = true,
            None => {}
        }
    }
    if !*write_closed
        && !socket.may_recv()
        && matches!(
            socket.state(),
            State::CloseWait | State::Closing | State::LastAck | State::TimeWait
        )
    {
        host.shutdown(Shutdown::Write)?;
        *write_closed = true;
    }
    if socket.can_send() {
        match socket.send(|space| transfer(host.read(space))).unwrap()? {
            Some(0) => socket.close(),
            Some(_) => progress = true,
            None => {}
        }
    }
    Ok(progress)
}

#[cfg(test)]
mod tests;
