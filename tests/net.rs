use helios::net::packet::{
    arp_packet, checksum, ether_frame, ipv4_packet, udp_datagram, EthernetFrame, MacAddr,
    BROADCAST_MAC, ETHERTYPE_ARP, ETHERTYPE_IPV4, ETHERTYPE_RARP, PROTO_ICMP, PROTO_UDP,
};
use helios::net::{
    NatConfig, NatEngine, BOOTP_CLIENT_PORT, BOOTP_SERVER_PORT, DNS_PORT, TFTP_PORT,
};
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const GUEST_MAC: MacAddr = [0x08, 0x00, 0x20, 0xc0, 0xff, 0xee];
const GUEST_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 2);
const GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 1);
const GATEWAY_MAC: MacAddr = [0x02, 0x00, 0xde, 0xad, 0xbe, 0xef];
const DHCP_MAGIC: [u8; 4] = [99, 130, 83, 99];

fn config() -> NatConfig {
    NatConfig {
        gateway_mac: GATEWAY_MAC,
        gateway_ip: GATEWAY_IP,
        client_ip: GUEST_IP,
        netmask: Ipv4Addr::new(255, 255, 255, 0),
        dns: SocketAddrV4::new(Ipv4Addr::new(8, 8, 8, 8), DNS_PORT),
        tftp_dir: None,
    }
}

fn guest_ip_frame(
    source_ip: Ipv4Addr,
    destination_ip: Ipv4Addr,
    protocol: u8,
    payload: &[u8],
    destination_mac: MacAddr,
) -> Vec<u8> {
    let packet = ipv4_packet(source_ip, destination_ip, protocol, 0x4242, 64, payload);
    ether_frame(destination_mac, GUEST_MAC, ETHERTYPE_IPV4, &packet)
}

fn guest_udp_frame(
    source_ip: Ipv4Addr,
    destination_ip: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    destination_mac: MacAddr,
) -> Vec<u8> {
    let segment = udp_datagram(
        source_ip,
        destination_ip,
        source_port,
        destination_port,
        payload,
    );
    guest_ip_frame(
        source_ip,
        destination_ip,
        PROTO_UDP,
        &segment,
        destination_mac,
    )
}

fn wait_output(net: &mut NatEngine) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        net.poll(Instant::now());
        if let Some(frame) = net.output() {
            return frame;
        }
        assert!(Instant::now() < deadline, "no frame produced");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn option_value(payload: &[u8], wanted: u8) -> Option<Vec<u8>> {
    let mut index = 240;
    while index < payload.len() {
        match payload[index] {
            255 => break,
            0 => index += 1,
            tag => {
                let length = usize::from(*payload.get(index + 1)?);
                if tag == wanted {
                    return Some(payload.get(index + 2..index + 2 + length)?.to_vec());
                }
                index += 2 + length;
            }
        }
    }
    None
}

fn bootp_request(message_type: Option<u8>, file: &str) -> Vec<u8> {
    let mut packet = vec![0u8; 300];
    packet[0] = 1;
    packet[1] = 1;
    packet[2] = 6;
    packet[4..8].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    packet[28..34].copy_from_slice(&GUEST_MAC);
    packet[108..108 + file.len()].copy_from_slice(file.as_bytes());
    if let Some(message_type) = message_type {
        packet[236..240].copy_from_slice(&DHCP_MAGIC);
        packet[240..243].copy_from_slice(&[53, 1, message_type]);
        packet[243] = 255;
    }
    packet
}

#[test]
fn arp_requests_for_the_gateway_are_answered() {
    let now = Instant::now();
    let mut net = NatEngine::new(config());
    let request = arp_packet(1, GUEST_MAC, GUEST_IP, [0; 6], GATEWAY_IP);
    net.input(
        &ether_frame(BROADCAST_MAC, GUEST_MAC, ETHERTYPE_ARP, &request),
        now,
    );

    let frame = net.output().expect("ARP reply");
    let ethernet = EthernetFrame::parse(&frame).unwrap();
    assert_eq!(ethernet.ethertype(), ETHERTYPE_ARP);
    assert_eq!(ethernet.destination(), GUEST_MAC);
    let arp = ethernet.arp().unwrap();
    assert_eq!(arp.operation, 2);
    assert_eq!(arp.sender_mac, GATEWAY_MAC);
    assert_eq!(arp.sender_ip, GATEWAY_IP);
    assert_eq!(arp.target_mac, GUEST_MAC);
    assert_eq!(arp.target_ip, GUEST_IP);
    assert_eq!(net.stats().arp_requests, 1);
}

#[test]
fn rarp_requests_receive_the_configured_client_address() {
    let now = Instant::now();
    let mut net = NatEngine::new(config());
    let request = arp_packet(
        3,
        GUEST_MAC,
        Ipv4Addr::BROADCAST,
        GUEST_MAC,
        Ipv4Addr::UNSPECIFIED,
    );
    net.input(
        &ether_frame(BROADCAST_MAC, GUEST_MAC, ETHERTYPE_RARP, &request),
        now,
    );

    let frame = net.output().expect("RARP reply");
    let ethernet = EthernetFrame::parse(&frame).unwrap();
    assert_eq!(ethernet.ethertype(), ETHERTYPE_RARP);
    let arp = ethernet.arp().unwrap();
    assert_eq!(arp.operation, 4);
    assert_eq!(arp.sender_mac, GATEWAY_MAC);
    assert_eq!(arp.sender_ip, GATEWAY_IP);
    assert_eq!(arp.target_mac, GUEST_MAC);
    assert_eq!(arp.target_ip, GUEST_IP);
    assert_eq!(net.stats().rarp_requests, 1);
}

#[test]
fn dhcp_discover_is_offered_and_request_is_acknowledged() {
    let now = Instant::now();
    let mut net = NatEngine::new(config());

    let discover = bootp_request(Some(1), "");
    let frame = guest_udp_frame(
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        68,
        BOOTP_SERVER_PORT,
        &discover,
        BROADCAST_MAC,
    );
    net.input(&frame, now);

    let reply = net.output().expect("DHCP offer");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let ip = ethernet.ipv4().unwrap();
    assert!(ip.destination.is_broadcast());
    let udp = ip.udp().unwrap();
    assert_eq!(udp.source_port, BOOTP_SERVER_PORT);
    assert_eq!(udp.destination_port, BOOTP_CLIENT_PORT);
    let payload = udp.payload();
    assert_eq!(payload[0], 2);
    assert_eq!(&payload[16..20], &GUEST_IP.octets());
    assert_eq!(option_value(payload, 53), Some(vec![2]));
    assert_eq!(
        option_value(payload, 1),
        Some(Ipv4Addr::new(255, 255, 255, 0).octets().to_vec())
    );
    assert_eq!(option_value(payload, 3), Some(GATEWAY_IP.octets().to_vec()));
    assert_eq!(option_value(payload, 6), Some(GATEWAY_IP.octets().to_vec()));
    assert_eq!(
        option_value(payload, 54),
        Some(GATEWAY_IP.octets().to_vec())
    );

    let mut request = bootp_request(Some(3), "");
    request[12..16].copy_from_slice(&GUEST_IP.octets());
    let frame = guest_udp_frame(
        GUEST_IP,
        Ipv4Addr::BROADCAST,
        68,
        BOOTP_SERVER_PORT,
        &request,
        BROADCAST_MAC,
    );
    net.input(&frame, now);
    let reply = net.output().expect("DHCP ack");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let ip = ethernet.ipv4().unwrap();
    assert_eq!(ip.destination, GUEST_IP);
    let udp = ip.udp().unwrap();
    assert_eq!(option_value(udp.payload(), 53), Some(vec![5]));
    assert_eq!(net.stats().bootp_requests, 2);
}

#[test]
fn bootp_names_a_tftp_boot_file_only_when_it_exists() {
    let root = temp_dir("bootp");
    fs::write(root.join("unix"), b"kernel").unwrap();
    let mut config = config();
    config.tftp_dir = Some(root.clone());
    let now = Instant::now();

    let mut net = NatEngine::new(config);
    let request = bootp_request(None, "unix");
    let frame = guest_udp_frame(
        GUEST_IP,
        GATEWAY_IP,
        68,
        BOOTP_SERVER_PORT,
        &request,
        GATEWAY_MAC,
    );
    net.input(&frame, now);
    let reply = net.output().expect("BOOTP reply");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let udp = ethernet.ipv4().unwrap().udp().unwrap();
    let payload = udp.payload();
    assert_eq!(&payload[108..112], b"unix");
    assert!(payload[44..108].starts_with(b"helios"));

    let request = bootp_request(None, "missing");
    let frame = guest_udp_frame(
        GUEST_IP,
        GATEWAY_IP,
        68,
        BOOTP_SERVER_PORT,
        &request,
        GATEWAY_MAC,
    );
    net.input(&frame, now);
    let reply = net.output().expect("BOOTP reply");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let udp = ethernet.ipv4().unwrap().udp().unwrap();
    assert_eq!(udp.payload()[108], 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn icmp_echo_to_the_gateway_is_answered() {
    let now = Instant::now();
    let mut net = NatEngine::new(config());
    let mut echo = vec![8, 0, 0, 0, 0xab, 0xcd, 0, 1, b'h', b'i'];
    let sum = checksum(&echo);
    echo[2..4].copy_from_slice(&sum.to_be_bytes());
    let frame = guest_ip_frame(GUEST_IP, GATEWAY_IP, PROTO_ICMP, &echo, GATEWAY_MAC);
    net.input(&frame, now);

    let reply = net.output().expect("ICMP reply");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let ip = ethernet.ipv4().unwrap();
    assert_eq!(ip.source, GATEWAY_IP);
    assert_eq!(ip.destination, GUEST_IP);
    assert_eq!(ip.protocol, PROTO_ICMP);
    assert_eq!(ip.payload()[0], 0);
    assert_eq!(&ip.payload()[4..], &echo[4..]);
    assert_eq!(net.stats().icmp_echoes, 1);
}

#[test]
fn udp_flows_are_natted_to_host_sockets_and_answers_come_back() {
    let server = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let port = server.local_addr().unwrap().port();
    let mut net = NatEngine::new(config());
    let frame = guest_udp_frame(
        GUEST_IP,
        Ipv4Addr::LOCALHOST,
        40000,
        port,
        b"ping",
        GATEWAY_MAC,
    );
    net.input(&frame, Instant::now());

    let mut buffer = [0u8; 64];
    let (length, peer) = server.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..length], b"ping");
    server.send_to(b"pong", peer).unwrap();

    let reply = wait_output(&mut net);
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let ip = ethernet.ipv4().unwrap();
    assert_eq!(ip.source, Ipv4Addr::LOCALHOST);
    assert_eq!(ip.destination, GUEST_IP);
    let udp = ip.udp().unwrap();
    assert_eq!(udp.source_port, port);
    assert_eq!(udp.destination_port, 40000);
    assert_eq!(udp.payload(), b"pong");
    assert_eq!(net.stats().udp_replies, 1);
}

#[test]
fn dns_queries_are_relayed_and_answers_use_the_queried_address() {
    let server = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let port = server.local_addr().unwrap().port();
    let mut config = config();
    config.dns = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let mut net = NatEngine::new(config);

    let queried = Ipv4Addr::new(8, 8, 8, 8);
    let frame = guest_udp_frame(GUEST_IP, queried, 41000, DNS_PORT, b"query", GATEWAY_MAC);
    net.input(&frame, Instant::now());
    let mut buffer = [0u8; 64];
    let (length, peer) = server.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..length], b"query");
    server.send_to(b"answer", peer).unwrap();

    let reply = wait_output(&mut net);
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let ip = ethernet.ipv4().unwrap();
    assert_eq!(ip.source, queried);
    assert_eq!(ip.destination, GUEST_IP);
    let udp = ip.udp().unwrap();
    assert_eq!(udp.source_port, DNS_PORT);
    assert_eq!(udp.destination_port, 41000);
    assert_eq!(udp.payload(), b"answer");
    assert_eq!(net.stats().dns_queries, 1);
}

#[test]
fn tftp_serves_blocks_in_lock_step_and_refuses_traversal() {
    let root = temp_dir("tftp");
    let contents: Vec<u8> = (0..600).map(|index| (index % 251) as u8).collect();
    fs::write(root.join("bootfile"), &contents).unwrap();
    let mut config = config();
    config.tftp_dir = Some(root.clone());
    let now = Instant::now();
    let mut net = NatEngine::new(config);

    let request = {
        let mut packet = vec![0, 1];
        packet.extend_from_slice(b"bootfile\0octet\0");
        packet
    };
    let frame = guest_udp_frame(GUEST_IP, GATEWAY_IP, 5000, TFTP_PORT, &request, GATEWAY_MAC);
    net.input(&frame, now);
    let reply = net.output().expect("TFTP data");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let udp = ethernet.ipv4().unwrap().udp().unwrap();
    assert_eq!(udp.source_port, TFTP_PORT);
    assert_eq!(udp.destination_port, 5000);
    assert_eq!(udp.payload()[..4], [0, 3, 0, 1]);
    assert_eq!(&udp.payload()[4..], &contents[..512]);

    let ack = [0u8, 4, 0, 1];
    let frame = guest_udp_frame(GUEST_IP, GATEWAY_IP, 5000, TFTP_PORT, &ack, GATEWAY_MAC);
    net.input(&frame, now);
    let reply = net.output().expect("TFTP second block");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let udp = ethernet.ipv4().unwrap().udp().unwrap();
    assert_eq!(udp.payload()[..4], [0, 3, 0, 2]);
    assert_eq!(&udp.payload()[4..], &contents[512..]);

    let ack = [0u8, 4, 0, 2];
    let frame = guest_udp_frame(GUEST_IP, GATEWAY_IP, 5000, TFTP_PORT, &ack, GATEWAY_MAC);
    net.input(&frame, now);
    assert!(net.output().is_none());

    let request = {
        let mut packet = vec![0, 1];
        packet.extend_from_slice(b"../secret\0octet\0");
        packet
    };
    let frame = guest_udp_frame(GUEST_IP, GATEWAY_IP, 5001, TFTP_PORT, &request, GATEWAY_MAC);
    net.input(&frame, now);
    let reply = net.output().expect("TFTP error");
    let ethernet = EthernetFrame::parse(&reply).unwrap();
    let udp = ethernet.ipv4().unwrap().udp().unwrap();
    assert_eq!(udp.payload()[0..2], [0, 5]);
    assert_eq!(udp.payload()[2..4], [0, 1]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tftp_retransmits_an_unacknowledged_block() {
    let root = temp_dir("retransmit");
    fs::write(root.join("file"), vec![0x5a; 600]).unwrap();
    let mut config = config();
    config.tftp_dir = Some(root.clone());
    let now = Instant::now();
    let mut net = NatEngine::new(config);

    let request = {
        let mut packet = vec![0, 1];
        packet.extend_from_slice(b"file\0octet\0");
        packet
    };
    let frame = guest_udp_frame(GUEST_IP, GATEWAY_IP, 5002, TFTP_PORT, &request, GATEWAY_MAC);
    net.input(&frame, now);
    let first_frame = net.output().expect("first block");
    net.poll(now + Duration::from_millis(1100));
    let resent_frame = net.output().expect("retransmitted block");
    let first = EthernetFrame::parse(&first_frame).unwrap();
    let resent = EthernetFrame::parse(&resent_frame).unwrap();
    assert_eq!(
        first.ipv4().unwrap().udp().unwrap().payload(),
        resent.ipv4().unwrap().udp().unwrap().payload()
    );

    let _ = fs::remove_dir_all(root);
}

fn temp_dir(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("helios-net-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}
