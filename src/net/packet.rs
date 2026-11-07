//! Ethernet, ARP/RARP, IPv4, ICMP and UDP wire formats for the host network
//! backend. These are the host byte boundary: multi-byte fields are converted
//! here, never inside CPU or device code.

use std::net::Ipv4Addr;

pub type MacAddr = [u8; 6];
pub const BROADCAST_MAC: MacAddr = [0xff; 6];

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_RARP: u16 = 0x8035;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub const ETH_HDR: usize = 14;
pub const ETH_MIN: usize = 60;
pub const IPV4_HDR: usize = 20;
pub const UDP_HDR: usize = 8;
pub const ICMP_HDR: usize = 8;
pub const MTU: usize = 1500;
const IPV4_MAX_PAYLOAD: usize = MTU - IPV4_HDR;

#[inline]
fn be16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

#[inline]
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn mac_at(bytes: &[u8], offset: usize) -> MacAddr {
    bytes[offset..offset + 6].try_into().expect("six-byte MAC")
}

fn ip_at(bytes: &[u8], offset: usize) -> Ipv4Addr {
    Ipv4Addr::new(
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    )
}

fn sum16(mut accumulator: u32, data: &[u8]) -> u32 {
    let mut chunks = data.chunks_exact(2);
    for chunk in &mut chunks {
        accumulator += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    if let [last] = chunks.remainder() {
        accumulator += u32::from(*last) << 8;
    }
    accumulator
}

fn fold(mut accumulator: u32) -> u16 {
    while accumulator >> 16 != 0 {
        accumulator = (accumulator & 0xffff) + (accumulator >> 16);
    }
    !(accumulator as u16)
}

/// RFC 1071 Internet checksum.
pub fn checksum(data: &[u8]) -> u16 {
    fold(sum16(0, data))
}

/// TCP/UDP checksum over the IPv4 pseudo-header and segment. The segment's
/// checksum field must already be zero.
pub fn pseudo_checksum(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, segment: &[u8]) -> u16 {
    let mut accumulator = sum16(0, &src.octets());
    accumulator = sum16(accumulator, &dst.octets());
    accumulator += u32::from(protocol);
    accumulator += segment.len() as u32;
    fold(sum16(accumulator, segment))
}

#[derive(Clone, Copy)]
pub struct EthernetFrame<'a> {
    bytes: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        (bytes.len() >= ETH_HDR).then_some(Self { bytes })
    }

    pub fn destination(&self) -> MacAddr {
        mac_at(self.bytes, 0)
    }

    pub fn source(&self) -> MacAddr {
        mac_at(self.bytes, 6)
    }

    pub fn ethertype(&self) -> u16 {
        be16(self.bytes, 12)
    }

    pub fn payload(&self) -> &'a [u8] {
        &self.bytes[ETH_HDR..]
    }

    pub fn arp(&self) -> Option<Arp<'a>> {
        (self.ethertype() == ETHERTYPE_ARP || self.ethertype() == ETHERTYPE_RARP)
            .then(|| Arp::parse(self.payload()))
            .flatten()
    }

    pub fn ipv4(&self) -> Option<Ipv4<'a>> {
        (self.ethertype() == ETHERTYPE_IPV4)
            .then(|| Ipv4::parse(self.payload()))
            .flatten()
    }
}

/// An Ethernet frame padded to the 60-byte minimum.
pub fn ether_frame(
    destination: MacAddr,
    source: MacAddr,
    ethertype: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = vec![0; ETH_MIN.max(ETH_HDR + payload.len())];
    frame[..6].copy_from_slice(&destination);
    frame[6..12].copy_from_slice(&source);
    put16(&mut frame, 12, ethertype);
    frame[ETH_HDR..ETH_HDR + payload.len()].copy_from_slice(payload);
    frame
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arp<'a> {
    pub operation: u16,
    pub sender_mac: MacAddr,
    pub sender_ip: Ipv4Addr,
    pub target_mac: MacAddr,
    pub target_ip: Ipv4Addr,
    bytes: &'a [u8],
}

impl<'a> Arp<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 28
            || be16(bytes, 0) != 1
            || be16(bytes, 2) != ETHERTYPE_IPV4
            || bytes[4] != 6
            || bytes[5] != 4
        {
            return None;
        }
        Some(Self {
            operation: be16(bytes, 6),
            sender_mac: mac_at(bytes, 8),
            sender_ip: ip_at(bytes, 14),
            target_mac: mac_at(bytes, 18),
            target_ip: ip_at(bytes, 24),
            bytes,
        })
    }

    /// Rewrite a request into a reply/answer with the given sender identity.
    pub fn reply(self, sender_mac: MacAddr, sender_ip: Ipv4Addr, target_ip: Ipv4Addr) -> Vec<u8> {
        let mut bytes = self.bytes[..28].to_vec();
        put16(&mut bytes, 6, if self.operation == 3 { 4 } else { 2 });
        bytes[8..14].copy_from_slice(&sender_mac);
        bytes[14..18].copy_from_slice(&sender_ip.octets());
        bytes[18..24].copy_from_slice(&self.sender_mac);
        bytes[24..28].copy_from_slice(&target_ip.octets());
        bytes
    }
}

/// A signed ARP packet (used for RARP replies, whose sender is the client).
pub fn arp_packet(
    operation: u16,
    sender_mac: MacAddr,
    sender_ip: Ipv4Addr,
    target_mac: MacAddr,
    target_ip: Ipv4Addr,
) -> Vec<u8> {
    let mut bytes = vec![0u8; 28];
    put16(&mut bytes, 0, 1);
    put16(&mut bytes, 2, ETHERTYPE_IPV4);
    bytes[4] = 6;
    bytes[5] = 4;
    put16(&mut bytes, 6, operation);
    bytes[8..14].copy_from_slice(&sender_mac);
    bytes[14..18].copy_from_slice(&sender_ip.octets());
    bytes[18..24].copy_from_slice(&target_mac);
    bytes[24..28].copy_from_slice(&target_ip.octets());
    bytes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4<'a> {
    pub protocol: u8,
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub id: u16,
    pub ttl: u8,
    /// Fragment offset in bytes.
    pub fragment_offset: usize,
    pub more_fragments: bool,
    header: &'a [u8],
    payload: &'a [u8],
}

impl<'a> Ipv4<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < IPV4_HDR || bytes[0] >> 4 != 4 {
            return None;
        }
        let header = usize::from(bytes[0] & 0x0f) * 4;
        if header < IPV4_HDR || bytes.len() < header {
            return None;
        }
        let total = usize::from(be16(bytes, 2));
        if total < header || bytes.len() < total {
            return None;
        }
        let flags_offset = be16(bytes, 6);
        Some(Self {
            protocol: bytes[9],
            source: ip_at(bytes, 12),
            destination: ip_at(bytes, 16),
            id: be16(bytes, 4),
            ttl: bytes[8],
            fragment_offset: usize::from(flags_offset & 0x1fff) * 8,
            more_fragments: flags_offset & 0x2000 != 0,
            header: &bytes[..header],
            payload: &bytes[header..total],
        })
    }

    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    pub fn udp(&self) -> Option<Udp<'a>> {
        (self.protocol == PROTO_UDP && !self.more_fragments && self.fragment_offset == 0)
            .then(|| Udp::parse(self.payload))
            .flatten()
    }

    /// True if a receive-side checksum check accepts this header.
    pub fn header_checksum_valid(&self) -> bool {
        checksum(self.header) == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Udp<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    payload: &'a [u8],
    segment: &'a [u8],
}

impl<'a> Udp<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < UDP_HDR {
            return None;
        }
        let length = usize::from(be16(bytes, 4));
        if length < UDP_HDR || bytes.len() < length {
            return None;
        }
        Some(Self {
            source_port: be16(bytes, 0),
            destination_port: be16(bytes, 2),
            payload: &bytes[UDP_HDR..length],
            segment: &bytes[..length],
        })
    }

    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    pub fn checksum_valid(&self, src: Ipv4Addr, dst: Ipv4Addr) -> bool {
        be16(self.segment, 6) == 0 || pseudo_checksum(src, dst, PROTO_UDP, self.segment) == 0
    }
}

/// Build a UDP datagram (header plus payload) with its checksum filled in.
pub fn udp_datagram(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut segment = vec![0u8; UDP_HDR + payload.len()];
    put16(&mut segment, 0, source_port);
    put16(&mut segment, 2, destination_port);
    let length = segment.len() as u16;
    put16(&mut segment, 4, length);
    segment[UDP_HDR..].copy_from_slice(payload);
    // A computed zero must be transmitted as all ones (RFC 768).
    let checksum = match pseudo_checksum(src, dst, PROTO_UDP, &segment) {
        0 => 0xffff,
        checksum => checksum,
    };
    put16(&mut segment, 6, checksum);
    segment
}

/// One unfragmented IPv4 packet around `payload`.
pub fn ipv4_packet(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    protocol: u8,
    id: u16,
    ttl: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut packet = vec![0u8; IPV4_HDR + payload.len()];
    packet[0] = 0x45;
    let length = packet.len() as u16;
    put16(&mut packet, 2, length);
    put16(&mut packet, 4, id);
    put16(&mut packet, 6, 0x4000); // Don't fragment: replies are already sized.
    packet[8] = ttl;
    packet[9] = protocol;
    packet[12..16].copy_from_slice(&src.octets());
    packet[16..20].copy_from_slice(&dst.octets());
    packet[IPV4_HDR..].copy_from_slice(payload);
    let checksum = checksum(&packet[..IPV4_HDR]);
    put16(&mut packet, 10, checksum);
    packet
}

/// IPv4 packets for one UDP datagram, splitting at 8-byte boundaries when the
/// payload exceeds the Ethernet MTU.
pub fn ipv4_udp_fragments(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    id: u16,
    ttl: u8,
) -> Vec<Vec<u8>> {
    let segment = udp_datagram(src, dst, source_port, destination_port, payload);
    if segment.len() <= IPV4_MAX_PAYLOAD {
        return vec![ipv4_packet(src, dst, PROTO_UDP, id, ttl, &segment)];
    }
    let mut packets = Vec::new();
    let mut offset = 0;
    while offset < segment.len() {
        let remaining = segment.len() - offset;
        let mut length = remaining.min(IPV4_MAX_PAYLOAD);
        // Every fragment except the last must end on an 8-byte boundary.
        if length < remaining {
            length &= !7;
        }
        let mut packet = vec![0u8; IPV4_HDR + length];
        packet[0] = 0x45;
        let total = packet.len() as u16;
        put16(&mut packet, 2, total);
        put16(&mut packet, 4, id);
        let fragment = (offset / 8) as u16 | if length < remaining { 0x2000 } else { 0 };
        put16(&mut packet, 6, fragment);
        packet[8] = ttl;
        packet[9] = PROTO_UDP;
        packet[12..16].copy_from_slice(&src.octets());
        packet[16..20].copy_from_slice(&dst.octets());
        packet[IPV4_HDR..].copy_from_slice(&segment[offset..offset + length]);
        let checksum = checksum(&packet[..IPV4_HDR]);
        put16(&mut packet, 10, checksum);
        packets.push(packet);
        offset += length;
    }
    packets
}

/// ICMP echo reply for an echo request's payload (header included).
pub fn icmp_echo_reply(request: &[u8]) -> Option<Vec<u8>> {
    if request.len() < ICMP_HDR || request[0] != 8 {
        return None;
    }
    let mut reply = request.to_vec();
    reply[0] = 0;
    reply[1] = 0;
    put16(&mut reply, 2, 0);
    let checksum = checksum(&reply);
    put16(&mut reply, 2, checksum);
    Some(reply)
}

/// Build an ICMP destination-unreachable/protocol-unreachable message quoting
/// the start of the offending IPv4 packet.
pub fn icmp_protocol_unreachable(ip_packet: &[u8]) -> Vec<u8> {
    let quote = &ip_packet[..ip_packet.len().min(IPV4_HDR + 8)];
    let mut message = vec![0u8; ICMP_HDR + quote.len()];
    message[0] = 3;
    message[1] = 2;
    message[ICMP_HDR..].copy_from_slice(quote);
    let checksum = checksum(&message);
    put16(&mut message, 2, checksum);
    message
}
