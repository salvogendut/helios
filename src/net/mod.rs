//! Host-side user-mode NAT for the SS5 Ethernet endpoint.
//!
//! The engine owns no emulation state and runs on the frontend's thread: the
//! frontend hands it guest frames, calls `poll` with host time, and injects the
//! returned frames through `Ethernet::receive_frame`. Stage one implements ARP
//! and RARP, BOOTP/DHCP, gateway ICMP echo, UDP NAT with DNS relaying, and a
//! read-only TFTP server. TCP NAT and host port forwards are explicit later
//! stages; TCP packets are counted and dropped.

pub mod packet;
mod tftp;

use crate::net::packet::{
    arp_packet, ether_frame, icmp_echo_reply, icmp_protocol_unreachable, ipv4_packet,
    ipv4_udp_fragments, EthernetFrame, Ipv4, MacAddr, Udp, BROADCAST_MAC, ETHERTYPE_ARP,
    ETHERTYPE_IPV4, ETHERTYPE_RARP, PROTO_ICMP, PROTO_TCP, PROTO_UDP,
};
use std::collections::HashMap;
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tftp::TftpServer;

pub const DEFAULT_GATEWAY_MAC: MacAddr = [0x02, 0x00, 0xde, 0xad, 0xbe, 0xef];
pub const DEFAULT_GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 1);
pub const DEFAULT_CLIENT_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 2);
pub const DEFAULT_NETMASK: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
pub const DNS_PORT: u16 = 53;
pub const BOOTP_SERVER_PORT: u16 = 67;
pub const BOOTP_CLIENT_PORT: u16 = 68;
pub const TFTP_PORT: u16 = 69;

const DHCP_MAGIC: [u8; 4] = [99, 130, 83, 99];
const BOOTP_REQUEST: u8 = 1;
const UDP_FLOW_LIMIT: usize = 128;
const DNS_FLOW_LIMIT: usize = 32;
const UDP_IDLE: Duration = Duration::from_secs(120);
const DNS_IDLE: Duration = Duration::from_secs(30);
const REPLY_TTL: u8 = 64;
const DNS_FALLBACK: Ipv4Addr = Ipv4Addr::new(8, 8, 8, 8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NatConfig {
    pub gateway_mac: MacAddr,
    pub gateway_ip: Ipv4Addr,
    pub client_ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    /// Host resolver that guest DNS queries are relayed to.
    pub dns: SocketAddrV4,
    /// Read-only TFTP root; also enables naming a boot file in BOOTP replies.
    pub tftp_dir: Option<PathBuf>,
}

impl Default for NatConfig {
    fn default() -> Self {
        Self {
            gateway_mac: DEFAULT_GATEWAY_MAC,
            gateway_ip: DEFAULT_GATEWAY_IP,
            client_ip: DEFAULT_CLIENT_IP,
            netmask: DEFAULT_NETMASK,
            dns: SocketAddrV4::new(system_dns().unwrap_or(DNS_FALLBACK), DNS_PORT),
            tftp_dir: None,
        }
    }
}

/// The host's first configured IPv4 nameserver, if any.
pub fn system_dns() -> Option<Ipv4Addr> {
    let text = fs::read_to_string("/etc/resolv.conf").ok()?;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() == Some("nameserver") {
            if let Some(Ok(address)) = fields.next().map(str::parse) {
                return Some(address);
            }
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NatStats {
    pub guest_frames: u64,
    /// Host frames produced, before the Ethernet receive queue accepts them.
    pub host_frames: u64,
    pub arp_requests: u64,
    pub rarp_requests: u64,
    pub bootp_requests: u64,
    pub icmp_echoes: u64,
    pub udp_requests: u64,
    pub udp_replies: u64,
    pub dns_queries: u64,
    pub tftp_datagrams: u64,
    pub dropped: u64,
    /// IPv4 TCP packets: TCP NAT is a later stage.
    pub unsupported_tcp: u64,
    /// Fragmented guest IPv4 packets: reassembly is not implemented.
    pub unsupported_fragments: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct UdpKey {
    guest_port: u16,
    remote: SocketAddrV4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct DnsKey {
    guest_port: u16,
    requested: Ipv4Addr,
}

struct UdpFlow {
    socket: UdpSocket,
    guest_ip: Ipv4Addr,
    guest_port: u16,
    remote: SocketAddrV4,
    last_used: Instant,
}

struct DnsFlow {
    socket: UdpSocket,
    guest_ip: Ipv4Addr,
    guest_port: u16,
    requested: Ipv4Addr,
    last_used: Instant,
}

/// Single-owner NAT engine driven by a frontend. Host time is supplied by the
/// caller so emulator device time stays independent of host scheduling.
pub struct NatEngine {
    config: NatConfig,
    guest_mac: Option<MacAddr>,
    udp: HashMap<UdpKey, UdpFlow>,
    dns: HashMap<DnsKey, DnsFlow>,
    tftp: Option<TftpServer>,
    output: std::collections::VecDeque<Vec<u8>>,
    ip_id: u16,
    stats: NatStats,
    /// Reused host-socket receive buffer; per-step allocation was measurable.
    buffer: Vec<u8>,
}

impl NatEngine {
    pub fn new(config: NatConfig) -> Self {
        Self {
            tftp: config.tftp_dir.clone().map(TftpServer::new),
            config,
            guest_mac: None,
            udp: HashMap::new(),
            dns: HashMap::new(),
            output: std::collections::VecDeque::new(),
            ip_id: 0,
            stats: NatStats::default(),
            buffer: vec![0u8; 65536],
        }
    }

    pub fn config(&self) -> &NatConfig {
        &self.config
    }

    pub fn stats(&self) -> NatStats {
        self.stats
    }

    pub fn connections(&self) -> (usize, usize) {
        (self.udp.len(), self.dns.len())
    }

    /// Handle one guest Ethernet frame (without FCS), producing zero or more
    /// frames toward the guest via `output`.
    pub fn input(&mut self, frame: &[u8], now: Instant) {
        let Some(ethernet) = EthernetFrame::parse(frame) else {
            self.stats.dropped += 1;
            return;
        };
        self.stats.guest_frames += 1;
        let source = ethernet.source();
        if source != [0; 6] && source[0] & 1 == 0 {
            self.guest_mac = Some(source);
        }
        match ethernet.ethertype() {
            ETHERTYPE_ARP => self.handle_arp(&ethernet),
            ETHERTYPE_RARP => self.handle_rarp(&ethernet),
            ETHERTYPE_IPV4 => self.handle_ipv4(&ethernet, now),
            _ => {}
        }
    }

    /// Poll host sockets and retransmit timers. `now` is host time.
    pub fn poll(&mut self, now: Instant) {
        self.poll_udp(now);
        self.poll_dns(now);
        let due = match self.tftp.as_mut() {
            Some(tftp) => tftp.poll(now),
            None => Vec::new(),
        };
        for ((ip, port), packet) in due {
            self.send_udp(
                self.guest_destination(),
                self.config.gateway_ip,
                ip,
                TFTP_PORT,
                port,
                &packet,
            );
            self.stats.tftp_datagrams += 1;
        }
        self.udp
            .retain(|_, flow| now.duration_since(flow.last_used) < UDP_IDLE);
        self.dns
            .retain(|_, flow| now.duration_since(flow.last_used) < DNS_IDLE);
    }

    /// Next frame for the guest, in order.
    pub fn output(&mut self) -> Option<Vec<u8>> {
        self.output.pop_front()
    }

    fn guest_destination(&self) -> MacAddr {
        self.guest_mac.unwrap_or(BROADCAST_MAC)
    }

    fn next_ip_id(&mut self) -> u16 {
        self.ip_id = self.ip_id.wrapping_add(1);
        self.ip_id
    }

    fn emit(&mut self, frame: Vec<u8>) {
        self.stats.host_frames += 1;
        self.output.push_back(frame);
    }

    fn handle_arp(&mut self, ethernet: &EthernetFrame<'_>) {
        let Some(arp) = ethernet.arp() else {
            self.stats.dropped += 1;
            return;
        };
        if arp.operation != 1 || arp.target_ip != self.config.gateway_ip {
            return;
        }
        self.stats.arp_requests += 1;
        let reply = arp_packet(
            2,
            self.config.gateway_mac,
            self.config.gateway_ip,
            arp.sender_mac,
            arp.sender_ip,
        );
        self.emit(ether_frame(
            arp.sender_mac,
            self.config.gateway_mac,
            ETHERTYPE_ARP,
            &reply,
        ));
    }

    fn handle_rarp(&mut self, ethernet: &EthernetFrame<'_>) {
        let Some(arp) = ethernet.arp() else {
            self.stats.dropped += 1;
            return;
        };
        if arp.operation != 3 {
            return;
        }
        self.stats.rarp_requests += 1;
        // The reply's sender is the server (gateway address) and the target is
        // the requester (assigned address), as RFC 903 describes.
        let client = if arp.sender_mac != [0; 6] {
            arp.sender_mac
        } else {
            arp.target_mac
        };
        let reply = arp_packet(
            4,
            self.config.gateway_mac,
            self.config.gateway_ip,
            client,
            self.config.client_ip,
        );
        self.emit(ether_frame(
            client,
            self.config.gateway_mac,
            ETHERTYPE_RARP,
            &reply,
        ));
    }

    fn handle_ipv4(&mut self, ethernet: &EthernetFrame<'_>, now: Instant) {
        let Some(ip) = ethernet.ipv4() else {
            self.stats.dropped += 1;
            return;
        };
        if !ip.header_checksum_valid() {
            self.stats.dropped += 1;
            return;
        }
        if ip.fragment_offset != 0 || ip.more_fragments {
            self.stats.unsupported_fragments += 1;
            return;
        }
        match ip.protocol {
            PROTO_UDP => {
                let Some(udp) = ip.udp() else {
                    self.stats.dropped += 1;
                    return;
                };
                if !udp.checksum_valid(ip.source, ip.destination) {
                    self.stats.dropped += 1;
                    return;
                }
                self.handle_udp(ethernet, &ip, &udp, now);
            }
            PROTO_ICMP => self.handle_icmp(ethernet, &ip),
            PROTO_TCP => {
                self.stats.unsupported_tcp += 1;
                let unreachable = icmp_protocol_unreachable(ethernet.payload());
                self.reply_ipv4(
                    ethernet.source(),
                    self.config.gateway_ip,
                    ip.source,
                    PROTO_ICMP,
                    &unreachable,
                );
            }
            _ => {}
        }
    }

    fn handle_udp(
        &mut self,
        ethernet: &EthernetFrame<'_>,
        ip: &Ipv4<'_>,
        udp: &Udp<'_>,
        now: Instant,
    ) {
        self.stats.udp_requests += 1;
        let client = (ip.source, udp.source_port);
        // BOOTP clients may broadcast, send to 0.0.0.0, or unicast to the
        // gateway; the server port identifies the service.
        if udp.destination_port == BOOTP_SERVER_PORT {
            self.handle_bootp(ethernet, udp);
            return;
        }
        if ip.destination == self.config.gateway_ip && udp.destination_port == TFTP_PORT {
            let reply = match self.tftp.as_mut() {
                Some(tftp) => tftp.handle(client, udp.payload(), now),
                None => None,
            };
            self.stats.tftp_datagrams += 1;
            if let Some(reply) = reply {
                self.send_udp(
                    ethernet.source(),
                    self.config.gateway_ip,
                    ip.source,
                    TFTP_PORT,
                    udp.source_port,
                    &reply,
                );
            }
            return;
        }
        if udp.destination_port == DNS_PORT {
            self.dns_query(
                ip.source,
                udp.source_port,
                ip.destination,
                udp.payload(),
                now,
            );
            return;
        }
        self.nat_udp(
            ip.source,
            udp.source_port,
            ip.destination,
            udp.destination_port,
            udp.payload(),
            now,
        );
    }

    fn handle_icmp(&mut self, ethernet: &EthernetFrame<'_>, ip: &Ipv4<'_>) {
        if ip.destination != self.config.gateway_ip {
            return;
        }
        let Some(reply) = icmp_echo_reply(ip.payload()) else {
            return;
        };
        self.stats.icmp_echoes += 1;
        self.reply_ipv4(
            ethernet.source(),
            self.config.gateway_ip,
            ip.source,
            PROTO_ICMP,
            &reply,
        );
    }

    fn handle_bootp(&mut self, ethernet: &EthernetFrame<'_>, udp: &Udp<'_>) {
        let request = udp.payload();
        if request.len() < 236 || request[0] != BOOTP_REQUEST {
            self.stats.dropped += 1;
            return;
        }
        self.stats.bootp_requests += 1;
        let is_dhcp = request.len() >= 240 && request[236..240] == DHCP_MAGIC;
        let message_type = if is_dhcp {
            dhcp_message_type(&request[240..])
        } else {
            None
        };
        let reply_type = match message_type {
            Some(1) => Some(2), // DISCOVER -> OFFER
            Some(3) => Some(5), // REQUEST -> ACK
            Some(8) => Some(5), // INFORM -> ACK
            None => None,
            _ => return,
        };

        let mut reply = vec![0u8; 300];
        reply[0] = 2; // BOOTREPLY
        reply[1] = 1; // Ethernet
        reply[2] = 6; // hardware address length
        reply[4..8].copy_from_slice(&request[4..8]); // xid
        reply[16..20].copy_from_slice(&self.config.client_ip.octets()); // yiaddr
        reply[20..24].copy_from_slice(&self.config.gateway_ip.octets()); // siaddr
        reply[28..34].copy_from_slice(&request[28..34]); // chaddr
        self.bootp_boot_file(request, &mut reply);

        if is_dhcp {
            let mut options = Vec::new();
            options.extend_from_slice(&DHCP_MAGIC);
            let message_type = reply_type.expect("DHCP request has a reply type");
            options.extend_from_slice(&[53, 1, message_type]);
            let broadcast = broadcast_address(self.config.gateway_ip, self.config.netmask);
            for (tag, value) in [
                (1u8, self.config.netmask.octets()),
                (3, self.config.gateway_ip.octets()),
                (6, self.config.gateway_ip.octets()),
                (28, broadcast.octets()),
                (51, 86400u32.to_be_bytes()),
                (54, self.config.gateway_ip.octets()),
            ] {
                options.extend_from_slice(&[tag, 4]);
                options.extend_from_slice(&value);
            }
            options.push(255);
            reply[236..236 + options.len()].copy_from_slice(&options);
        }

        let ciaddr = Ipv4Addr::new(request[12], request[13], request[14], request[15]);
        let destination_ip = if !ciaddr.is_unspecified() && !ciaddr.is_broadcast() {
            ciaddr
        } else if is_dhcp {
            Ipv4Addr::BROADCAST
        } else {
            self.config.client_ip
        };
        let destination_mac = if destination_ip.is_broadcast() || destination_ip.is_multicast() {
            BROADCAST_MAC
        } else {
            ethernet.source()
        };
        self.send_udp(
            destination_mac,
            self.config.gateway_ip,
            destination_ip,
            BOOTP_SERVER_PORT,
            BOOTP_CLIENT_PORT,
            &reply,
        );
    }

    fn bootp_boot_file(&self, request: &[u8], reply: &mut [u8]) {
        let file = bootp_string(&request[108..236]);
        let Some(tftp) = &self.tftp else {
            return;
        };
        if file.is_empty() || !tftp.file_exists(&file) {
            return;
        }
        let server = bootp_string(&request[44..108]);
        let server = if server.is_empty() { "helios" } else { &server };
        let length = server.len().min(63);
        reply[44..44 + length].copy_from_slice(&server.as_bytes()[..length]);
        reply[108..108 + file.len()].copy_from_slice(file.as_bytes());
    }

    fn nat_udp(
        &mut self,
        guest_ip: Ipv4Addr,
        guest_port: u16,
        remote_ip: Ipv4Addr,
        remote_port: u16,
        payload: &[u8],
        now: Instant,
    ) {
        let remote = SocketAddrV4::new(remote_ip, remote_port);
        let key = UdpKey { guest_port, remote };
        if !self.udp.contains_key(&key) {
            if self.udp.len() >= UDP_FLOW_LIMIT {
                if let Some(oldest) = self
                    .udp
                    .iter()
                    .min_by_key(|(_, flow)| flow.last_used)
                    .map(|(key, _)| *key)
                {
                    self.udp.remove(&oldest);
                }
            }
            match host_udp_socket(remote) {
                Ok(socket) => {
                    self.udp.insert(
                        key,
                        UdpFlow {
                            socket,
                            guest_ip,
                            guest_port,
                            remote,
                            last_used: now,
                        },
                    );
                }
                Err(_) => {
                    self.stats.dropped += 1;
                    return;
                }
            }
        }
        if let Some(flow) = self.udp.get_mut(&key) {
            flow.guest_ip = guest_ip;
            flow.last_used = now;
            if flow.socket.send(payload).is_err() {
                self.stats.dropped += 1;
            }
        }
    }

    fn dns_query(
        &mut self,
        guest_ip: Ipv4Addr,
        guest_port: u16,
        requested: Ipv4Addr,
        payload: &[u8],
        now: Instant,
    ) {
        self.stats.dns_queries += 1;
        let key = DnsKey {
            guest_port,
            requested,
        };
        if !self.dns.contains_key(&key) {
            if self.dns.len() >= DNS_FLOW_LIMIT {
                if let Some(oldest) = self
                    .dns
                    .iter()
                    .min_by_key(|(_, flow)| flow.last_used)
                    .map(|(key, _)| *key)
                {
                    self.dns.remove(&oldest);
                }
            }
            match host_udp_socket(self.config.dns) {
                Ok(socket) => {
                    self.dns.insert(
                        key,
                        DnsFlow {
                            socket,
                            guest_ip,
                            guest_port,
                            requested,
                            last_used: now,
                        },
                    );
                }
                Err(_) => {
                    self.stats.dropped += 1;
                    return;
                }
            }
        }
        if let Some(flow) = self.dns.get_mut(&key) {
            flow.guest_ip = guest_ip;
            flow.last_used = now;
            if flow.socket.send(payload).is_err() {
                self.stats.dropped += 1;
            }
        }
    }

    fn poll_udp(&mut self, now: Instant) {
        if self.udp.is_empty() {
            return;
        }
        let mut replies = Vec::new();
        let mut dead = Vec::new();
        let Self { udp, buffer, .. } = self;
        for (key, flow) in udp.iter_mut() {
            loop {
                match flow.socket.recv(buffer) {
                    Ok(length) => {
                        flow.last_used = now;
                        replies.push((
                            flow.remote,
                            flow.guest_ip,
                            flow.guest_port,
                            buffer[..length].to_vec(),
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        dead.push(*key);
                        break;
                    }
                }
            }
        }
        for key in dead {
            self.udp.remove(&key);
        }
        for (remote, guest_ip, guest_port, payload) in replies {
            self.send_udp(
                self.guest_destination(),
                remote.ip().to_owned(),
                guest_ip,
                remote.port(),
                guest_port,
                &payload,
            );
            self.stats.udp_replies += 1;
        }
    }

    fn poll_dns(&mut self, now: Instant) {
        if self.dns.is_empty() {
            return;
        }
        let mut replies = Vec::new();
        let mut dead = Vec::new();
        let Self { dns, buffer, .. } = self;
        for (key, flow) in dns.iter_mut() {
            loop {
                match flow.socket.recv(buffer) {
                    Ok(length) => {
                        flow.last_used = now;
                        replies.push((
                            flow.requested,
                            flow.guest_ip,
                            flow.guest_port,
                            buffer[..length].to_vec(),
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        dead.push(*key);
                        break;
                    }
                }
            }
        }
        for key in dead {
            self.dns.remove(&key);
        }
        for (requested, guest_ip, guest_port, payload) in replies {
            self.send_udp(
                self.guest_destination(),
                requested,
                guest_ip,
                DNS_PORT,
                guest_port,
                &payload,
            );
            self.stats.udp_replies += 1;
        }
    }

    fn reply_ipv4(
        &mut self,
        destination_mac: MacAddr,
        source: Ipv4Addr,
        destination: Ipv4Addr,
        protocol: u8,
        payload: &[u8],
    ) {
        let id = self.next_ip_id();
        let packet = ipv4_packet(source, destination, protocol, id, REPLY_TTL, payload);
        self.emit(ether_frame(
            destination_mac,
            self.config.gateway_mac,
            ETHERTYPE_IPV4,
            &packet,
        ));
    }

    fn send_udp(
        &mut self,
        destination_mac: MacAddr,
        source: Ipv4Addr,
        destination: Ipv4Addr,
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
    ) {
        let id = self.next_ip_id();
        for packet in ipv4_udp_fragments(
            source,
            destination,
            source_port,
            destination_port,
            payload,
            id,
            REPLY_TTL,
        ) {
            self.emit(ether_frame(
                destination_mac,
                self.config.gateway_mac,
                ETHERTYPE_IPV4,
                &packet,
            ));
        }
    }
}

fn host_udp_socket(remote: SocketAddrV4) -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.set_nonblocking(true)?;
    socket.connect(remote)?;
    Ok(socket)
}

fn dhcp_message_type(options: &[u8]) -> Option<u8> {
    let mut index = 0;
    while index < options.len() {
        match options[index] {
            255 => break,
            0 => index += 1,
            tag => {
                let length = usize::from(*options.get(index + 1)?);
                if tag == 53 && length >= 1 {
                    return options.get(index + 2).copied();
                }
                index += 2 + length;
            }
        }
    }
    None
}

fn bootp_string(field: &[u8]) -> String {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn broadcast_address(address: Ipv4Addr, netmask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(address) | !u32::from(netmask))
}
