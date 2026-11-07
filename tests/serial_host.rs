use helios::serial_host::{TcpSerial, TelnetFilter};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::Duration;

fn connect(host: &mut TcpSerial) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", host.port())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // accept() runs inside receive(); the handshake is queued and flushed.
    host.receive();
    stream
}

fn read_handshake(stream: &mut TcpStream) {
    let mut handshake = [0u8; 12];
    stream.read_exact(&mut handshake).unwrap();
    assert_eq!(handshake, TelnetFilter::initial_handshake());
}

#[test]
fn tcp_serial_binds_loopback_and_offers_telnet_character_mode() {
    let mut host = TcpSerial::bind(0).unwrap();
    assert_ne!(host.port(), 0);
    let mut client = connect(&mut host);
    assert_eq!(host.connections(), 1);
    read_handshake(&mut client);
}

#[test]
fn guest_output_is_telnet_escaped_and_broadcast_to_every_client() {
    let mut host = TcpSerial::bind(0).unwrap();
    let mut first = connect(&mut host);
    let mut second = connect(&mut host);
    read_handshake(&mut first);
    read_handshake(&mut second);

    host.send(&[b'A', 0xff, b'B']);
    for client in [&mut first, &mut second] {
        let mut bytes = [0u8; 4];
        client.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [b'A', 0xff, 0xff, b'B']);
    }
}

#[test]
fn telnet_negotiation_is_consumed_and_real_input_reaches_the_guest() {
    let mut host = TcpSerial::bind(0).unwrap();
    let mut client = connect(&mut host);
    read_handshake(&mut client);

    client
        .write_all(&[0xff, 0xff, b'a', 0xff, 0xfb, 0x03, b'b'])
        .unwrap();
    let mut received = Vec::new();
    for _ in 0..16 {
        if let Some(byte) = host.receive() {
            received.push(byte);
            if received.len() == 3 {
                break;
            }
        }
    }
    assert_eq!(received, [0xff, b'a', b'b']);
}

#[test]
fn closing_one_client_keeps_other_clients_connected() {
    let mut host = TcpSerial::bind(0).unwrap();
    let mut first = connect(&mut host);
    let second = connect(&mut host);
    read_handshake(&mut first);
    drop(second);

    for _ in 0..16 {
        host.receive();
        if host.connections() == 1 {
            break;
        }
    }
    assert_eq!(host.connections(), 1);

    host.send(b"x");
    let mut byte = [0u8; 1];
    first.read_exact(&mut byte).unwrap();
    assert_eq!(byte, [b'x']);
    first.shutdown(Shutdown::Both).unwrap();
}

#[test]
fn an_occupied_port_is_reported_instead_of_panicking() {
    let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = held.local_addr().unwrap().port();
    assert!(TcpSerial::bind(port).is_err());
}
