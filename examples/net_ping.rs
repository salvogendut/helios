//! Exercise the NAT's real ICMP path without firmware or a guest disk.
//! cargo run --release --features host-icmp --example net_ping -- www.google.com
use helios::net::packet::{
    checksum, ether_frame, ipv4_packet, EthernetFrame, ETHERTYPE_IPV4, PROTO_ICMP,
};
use helios::net::{NatConfig, NatEngine};
use std::net::{IpAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1".into());
    let remote = (name.as_str(), 0)
        .to_socket_addrs()?
        .find_map(|address| match address.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .ok_or("no IPv4 address")?;
    let config = NatConfig::default();
    let mut net = NatEngine::new(config.clone());
    for sequence in 0u16..3 {
        let mut request = vec![8, 0, 0, 0, 0x12, 0x34, 0, 0];
        request[6..8].copy_from_slice(&sequence.to_be_bytes());
        request.extend_from_slice(b"HELIOS actual remote echo\0\xff");
        let sum = checksum(&request);
        request[2..4].copy_from_slice(&sum.to_be_bytes());
        let ip = ipv4_packet(config.client_ip, remote, PROTO_ICMP, sequence, 64, &request);
        let frame = ether_frame(config.gateway_mac, [2, 0, 0, 0, 0, 2], ETHERTYPE_IPV4, &ip);
        let start = Instant::now();
        net.input(&frame, start);
        loop {
            net.poll(Instant::now());
            if let Some(warning) = net.take_network_warning() {
                return Err(warning.into());
            }
            if let Some(reply) = net.output() {
                let ip = EthernetFrame::parse(&reply)
                    .and_then(|frame| frame.ipv4())
                    .ok_or("invalid reply")?;
                if ip.source != remote
                    || ip.destination != config.client_ip
                    || ip.protocol != PROTO_ICMP
                    || !ip.header_checksum_valid()
                    || ip.payload().len() != request.len()
                    || ip.payload()[..2] != [0, 0]
                    || ip.payload()[4..] != request[4..]
                    || checksum(ip.payload()) != 0
                {
                    return Err("echo identity/data mismatch".into());
                }
                println!("{remote}: sequence={sequence} identifier=0x1234 data={} bytes wall_rtt={:.3} ms", request.len()-8, start.elapsed().as_secs_f64()*1000.0);
                break;
            }
            if start.elapsed() >= Duration::from_secs(3) {
                return Err(format!("no remote reply from {remote}; {:?}", net.stats()).into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    println!("NAT counters: {:?}", net.stats());
    Ok(())
}
