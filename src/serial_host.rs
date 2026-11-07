//! Host-side TCP serial consoles on loopback, in the spirit of IRIS's serial
//! backends. One owner polls the listener; the emulator's device time does not
//! depend on host networking. Telnet clients get character-at-a-time
//! negotiation, and every connected client sees all output and may type.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};

/// Per-client output is bounded so a stalled telnet client cannot consume
/// unbounded host memory. The oldest bytes are dropped first, keeping the
/// most recent console output.
const PENDING_CAPACITY: usize = 64 * 1024;

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TelnetState {
    #[default]
    Data,
    Iac,
    Will,
    Wont,
    Do,
    Dont,
    Subnegotiation,
    SubnegotiationIac,
}

/// Minimal RFC 854/855/857/858 negotiation filter. The serial console wants
/// character-at-a-time input with no client echo and no end-of-line
/// translation, so the server offers WILL ECHO/SGA/BINARY and DO BINARY on
/// connection. The last reply for each option is remembered to avoid
/// negotiation loops. Subnegotiation payloads are discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelnetFilter {
    state: TelnetState,
    /// 0 = nothing sent, 1 = WILL, 2 = WONT.
    we_will: [u8; 256],
    /// 0 = nothing sent, 1 = DO, 2 = DONT.
    we_do: [u8; 256],
}

impl Default for TelnetFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl TelnetFilter {
    pub fn new() -> Self {
        let mut filter = Self {
            state: TelnetState::Data,
            we_will: [0; 256],
            we_do: [0; 256],
        };
        filter.we_will[usize::from(OPT_ECHO)] = 1;
        filter.we_will[usize::from(OPT_SGA)] = 1;
        filter.we_will[usize::from(OPT_BINARY)] = 1;
        filter.we_do[usize::from(OPT_BINARY)] = 1;
        filter
    }

    /// Character-at-a-time handshake written immediately after accept.
    pub const fn initial_handshake() -> [u8; 12] {
        [
            IAC, WILL, OPT_ECHO, IAC, WILL, OPT_SGA, IAC, WILL, OPT_BINARY, IAC, DO, OPT_BINARY,
        ]
    }

    /// Feed one inbound byte. Returns a real data byte when one falls out of
    /// the state machine; negotiation replies are appended to `reply`.
    pub fn feed(&mut self, byte: u8, reply: &mut Vec<u8>) -> Option<u8> {
        match self.state {
            TelnetState::Data => {
                if byte == IAC {
                    self.state = TelnetState::Iac;
                    None
                } else {
                    Some(byte)
                }
            }
            TelnetState::Iac => {
                if byte == IAC {
                    self.state = TelnetState::Data;
                    return Some(IAC);
                }
                self.state = match byte {
                    WILL => TelnetState::Will,
                    WONT => TelnetState::Wont,
                    DO => TelnetState::Do,
                    DONT => TelnetState::Dont,
                    SB => TelnetState::Subnegotiation,
                    _ => TelnetState::Data,
                };
                None
            }
            TelnetState::Will => {
                self.reply_do(byte, byte == OPT_BINARY, reply);
                self.state = TelnetState::Data;
                None
            }
            TelnetState::Wont => {
                self.reply_do(byte, false, reply);
                self.state = TelnetState::Data;
                None
            }
            TelnetState::Do => {
                self.reply_will(byte, matches!(byte, OPT_ECHO | OPT_SGA | OPT_BINARY), reply);
                self.state = TelnetState::Data;
                None
            }
            TelnetState::Dont => {
                self.reply_will(byte, false, reply);
                self.state = TelnetState::Data;
                None
            }
            TelnetState::Subnegotiation => {
                if byte == IAC {
                    self.state = TelnetState::SubnegotiationIac;
                }
                None
            }
            TelnetState::SubnegotiationIac => {
                self.state = if byte == SE {
                    TelnetState::Data
                } else {
                    TelnetState::Subnegotiation
                };
                None
            }
        }
    }

    fn reply_will(&mut self, option: u8, desired: bool, reply: &mut Vec<u8>) {
        let entry = usize::from(option);
        let state = if desired { 1 } else { 2 };
        if self.we_will[entry] != state {
            self.we_will[entry] = state;
            reply.extend_from_slice(&[IAC, if desired { WILL } else { WONT }, option]);
        }
    }

    fn reply_do(&mut self, option: u8, desired: bool, reply: &mut Vec<u8>) {
        let entry = usize::from(option);
        let state = if desired { 1 } else { 2 };
        if self.we_do[entry] != state {
            self.we_do[entry] = state;
            reply.extend_from_slice(&[IAC, if desired { DO } else { DONT }, option]);
        }
    }
}

/// Escape one outbound byte: IAC must be doubled so a telnet client does not
/// mistake guest data for the start of a command.
fn escape_byte(byte: u8, out: &mut Vec<u8>) {
    if byte == IAC {
        out.push(IAC);
    }
    out.push(byte);
}

#[derive(Debug)]
struct Client {
    stream: TcpStream,
    filter: TelnetFilter,
    pending: VecDeque<u8>,
}

impl Client {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            filter: TelnetFilter::new(),
            pending: VecDeque::new(),
        }
    }

    fn queue(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.pending.len() >= PENDING_CAPACITY {
                self.pending.pop_front();
            }
            self.pending.push_back(byte);
        }
    }

    /// Flush queued output without blocking. Returns false if the client is
    /// gone or unusable.
    fn flush(&mut self) -> bool {
        loop {
            let buffer = self.pending.make_contiguous();
            if buffer.is_empty() {
                return true;
            }
            match self.stream.write(buffer) {
                Ok(0) => return false,
                Ok(written) => {
                    self.pending.drain(..written);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
    }
}

/// A loopback TCP serial console. `bind` only listens on 127.0.0.1. Call
/// `receive` between CPU steps to accept clients and fetch input, and `send`
/// with device output; both are nonblocking.
#[derive(Debug)]
pub struct TcpSerial {
    listener: TcpListener,
    port: u16,
    clients: Vec<Client>,
}

impl TcpSerial {
    /// Bind `127.0.0.1:port`. Port zero asks the host for a free port; read it
    /// back with `port`.
    pub fn bind(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            port,
            clients: Vec::new(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn connections(&self) -> usize {
        self.clients.len()
    }

    fn accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(true).is_err() {
                        continue;
                    }
                    // The console is latency-sensitive; Nagle would hold a
                    // keystroke or line until the previous segment is acked.
                    let _ = stream.set_nodelay(true);
                    let mut client = Client::new(stream);
                    client.queue(&TelnetFilter::initial_handshake());
                    client.flush();
                    self.clients.push(client);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    fn flush(&mut self) {
        self.clients.retain_mut(Client::flush);
    }

    /// Queue guest output for every connected telnet client. Bytes are escaped
    /// for telnet and queued, so this never blocks the caller.
    pub fn send(&mut self, bytes: &[u8]) {
        self.accept();
        if self.clients.is_empty() {
            return;
        }
        let mut escaped = Vec::with_capacity(bytes.len() + bytes.len() / 64);
        for &byte in bytes {
            escape_byte(byte, &mut escaped);
        }
        for client in &mut self.clients {
            client.queue(&escaped);
        }
        self.flush();
    }

    /// Accept waiting clients and return the next guest-bound byte, if any.
    /// Telnet negotiation is consumed and answered; disconnected clients are
    /// removed.
    pub fn receive(&mut self) -> Option<u8> {
        self.accept();
        let mut index = 0;
        while index < self.clients.len() {
            let mut gone = false;
            let mut data = None;
            loop {
                let client = &mut self.clients[index];
                if !client.flush() {
                    gone = true;
                    break;
                }
                let mut byte = [0u8; 1];
                match client.stream.read(&mut byte) {
                    Ok(1) => {
                        let mut reply = Vec::new();
                        let input = client.filter.feed(byte[0], &mut reply);
                        if !reply.is_empty() {
                            client.queue(&reply);
                            if !client.flush() {
                                gone = true;
                                break;
                            }
                        }
                        if let Some(input) = input {
                            data = Some(input);
                            break;
                        }
                    }
                    Ok(_) => {
                        gone = true;
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        gone = true;
                        break;
                    }
                }
            }
            if gone {
                self.clients.remove(index);
            } else if let Some(data) = data {
                return Some(data);
            } else {
                index += 1;
            }
        }
        None
    }
}
