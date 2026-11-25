use super::*;
use std::net::TcpListener;

const MAC: MacAddr = [8, 0, 32, 1, 2, 3];
const GUEST: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 2);
const REMOTE: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 42);
const GW: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 1);

fn datagram(dst: Ipv4Addr, flags: u8, seq: u32, ack: u32, data: &[u8]) -> Vec<u8> {
    let mut seg = vec![0; 20 + data.len()];
    seg[0..2].copy_from_slice(&1222u16.to_be_bytes());
    seg[2..4].copy_from_slice(&80u16.to_be_bytes());
    seg[4..8].copy_from_slice(&seq.to_be_bytes());
    seg[8..12].copy_from_slice(&ack.to_be_bytes());
    seg[12] = 0x50;
    seg[13] = flags;
    seg[14..16].copy_from_slice(&8192u16.to_be_bytes());
    seg[20..].copy_from_slice(data);
    let sum = pseudo_checksum(GUEST, dst, PROTO_TCP, &seg);
    seg[16..18].copy_from_slice(&sum.to_be_bytes());
    ipv4_packet(GUEST, dst, PROTO_TCP, 0, 64, &seg)
}
fn feed(relay: &mut Relay, raw: &[u8], now: Instant, replies: &mut Replies) -> bool {
    relay.input(MAC, &Ipv4::parse(raw).unwrap(), raw, GW, now, replies)
}
fn decoded(reply: &(MacAddr, Vec<u8>)) -> TcpPacket<&[u8]> {
    assert_eq!(reply.0, MAC);
    let ip = Ipv4::parse(&reply.1).unwrap();
    assert_eq!(ip.destination, GUEST);
    assert!(ip.header_checksum_valid());
    assert_eq!(
        pseudo_checksum(ip.source, ip.destination, PROTO_TCP, ip.payload()),
        0
    );
    TcpPacket::new_checked(ip.payload()).unwrap()
}
#[test]
fn closed_port_reset_rules_and_invalid_checksums() {
    let now = Instant::now();
    let mut relay = Relay::default();
    let mut replies = vec![];
    assert!(feed(
        &mut relay,
        &datagram(GW, 2, u32::MAX, 0, &[]),
        now,
        &mut replies
    ));
    let rst = decoded(&replies[0]);
    assert!(rst.ack() && rst.rst());
    assert_eq!(rst.ack_number().0, 0);
    replies.clear();
    assert!(feed(
        &mut relay,
        &datagram(REMOTE, 16, 500, 77, &[]),
        now,
        &mut replies
    ));
    let rst = decoded(&replies[0]);
    assert!(rst.rst() && !rst.ack());
    assert_eq!(rst.seq_number().0, 77);
    replies.clear();
    feed(
        &mut relay,
        &datagram(REMOTE, 4, 2, 0, &[]),
        now,
        &mut replies,
    );
    let mut bad = datagram(REMOTE, 2, 0, 0, &[]);
    bad[36] ^= 2;
    assert!(!feed(&mut relay, &bad, now, &mut replies));
    assert!(replies.is_empty() && relay.pending.is_empty() && relay.flows.is_empty());
}

#[test]
fn duplicate_syn_data_and_half_close_preserve_host_stream() {
    let now = Instant::now();
    let mut relay = Relay::default();
    let mut replies = vec![];
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let std::net::SocketAddr::V4(remote) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    // Use a private loopback listener for the real host socket and an ordinary
    // routed guest destination. Exercise the production connect-completion path.
    let host = nonblocking_connect(remote).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let key = Key {
        guest: SocketAddrV4::new(GUEST, 1222),
        remote: SocketAddrV4::new(REMOTE, 80),
        mac: MAC,
    };
    let syn = datagram(REMOTE, 2, 100, 0, &[]);
    relay.pending.insert(
        key,
        Opening {
            host,
            syn: syn.clone(),
            deadline: now + CONNECT_TIMEOUT,
        },
    );
    feed(&mut relay, &syn, now, &mut replies);
    assert_eq!(relay.pending.len(), 1);
    relay.poll(now, &mut replies);
    let synack = decoded(replies.last().unwrap());
    assert!(synack.syn() && synack.ack());
    assert_eq!(synack.ack_number().0, 101);
    let ack = (synack.seq_number().0 as u32).wrapping_add(1);
    replies.clear();
    let request = datagram(REMOTE, 0x18, 101, ack, b"hello");
    feed(&mut relay, &request, now, &mut replies);
    feed(&mut relay, &request, now, &mut replies);
    feed(
        &mut relay,
        &datagram(REMOTE, 0x11, 106, ack, &[]),
        now,
        &mut replies,
    );
    relay.poll(now + POLL_INTERVAL, &mut replies);
    let mut got = Vec::new();
    server.read_to_end(&mut got).unwrap();
    assert_eq!(got, b"hello");
    server.write_all(b"reply after FIN").unwrap();
    server.shutdown(Shutdown::Write).unwrap();
    replies.clear();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !replies.iter().any(|r| decoded(r).fin()) {
        assert!(Instant::now() < deadline, "no FIN after the server closed");
        std::thread::sleep(POLL_INTERVAL);
        relay.poll(Instant::now(), &mut replies);
    }
    assert!(replies
        .iter()
        .any(|r| decoded(r).payload() == b"reply after FIN"));
    assert!(replies.iter().any(|r| decoded(r).fin()));
    relay.poll(now + IDLE_TIMEOUT + Duration::from_secs(1), &mut replies);
    assert!(relay.flows.is_empty());
}

#[test]
fn pending_timeout_refuses_the_guest_without_a_syn_ack() {
    let now = Instant::now();
    let mut relay = Relay::default();
    let key = Key {
        mac: MAC,
        guest: SocketAddrV4::new(GUEST, 1222),
        remote: SocketAddrV4::new(REMOTE, 80),
    };
    let host = HostSocket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
        .unwrap()
        .into();
    relay.pending.insert(
        key,
        Opening {
            host,
            syn: datagram(REMOTE, 2, 42, 0, &[]),
            deadline: now,
        },
    );
    let mut replies = Vec::new();
    relay.poll(now, &mut replies);
    assert_eq!(replies.len(), 1);
    let packet = decoded(&replies[0]);
    assert!(packet.rst() && packet.ack() && !packet.syn());
    assert_eq!(packet.ack_number().0, 43);
    assert!(relay.pending.is_empty() && relay.flows.is_empty());
}
