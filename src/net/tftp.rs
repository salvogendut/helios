//! Read-only RFC 1350 TFTP server for PROM network boot. Pure protocol:
//! datagrams in, datagrams out, no sockets. The NAT engine supplies
//! addressing and retransmission timing. Names are relative to the root even
//! when they start with `/`, as `tftpd -s` behaves.

use std::collections::HashMap;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub const BLOCK_SIZE: usize = 512;
pub const MAX_RETRIES: u8 = 5;
pub const RETRANSMIT_TIMEOUT: Duration = Duration::from_millis(1000);
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024 - 1;

const OP_RRQ: u16 = 1;
const OP_WRQ: u16 = 2;
const OP_DATA: u16 = 3;
const OP_ACK: u16 = 4;
const OP_ERROR: u16 = 5;

const ERR_NOT_DEFINED: u16 = 0;
const ERR_FILE_NOT_FOUND: u16 = 1;
const ERR_ACCESS_VIOLATION: u16 = 2;
const ERR_ILLEGAL_OPERATION: u16 = 4;

/// A client transfer identifier: address and source port.
pub type ClientId = (Ipv4Addr, u16);

struct Transfer {
    data: Vec<u8>,
    block: u16,
    offset: usize,
    block_len: usize,
    final_block: bool,
    last_packet: Vec<u8>,
    retries: u8,
    last_sent: Instant,
}

pub struct TftpServer {
    root: PathBuf,
    transfers: HashMap<ClientId, Transfer>,
}

impl TftpServer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            transfers: HashMap::new(),
        }
    }

    /// Whether a boot-file request names a regular file under the TFTP root.
    pub fn file_exists(&self, name: &str) -> bool {
        resolve(&self.root, name).is_some()
    }

    /// Handle one datagram from `client`, returning the reply packet.
    /// An ACK that completes the transfer has no reply.
    pub fn handle(&mut self, client: ClientId, packet: &[u8], now: Instant) -> Option<Vec<u8>> {
        if packet.len() < 4 {
            return None;
        }
        match u16::from_be_bytes([packet[0], packet[1]]) {
            OP_RRQ => Some(self.start(client, &packet[2..], now)),
            OP_WRQ => Some(error_packet(ERR_ACCESS_VIOLATION, "read-only TFTP server")),
            OP_ACK => self.acknowledge(client, u16::from_be_bytes([packet[2], packet[3]]), now),
            OP_ERROR => {
                self.transfers.remove(&client);
                None
            }
            _ => Some(error_packet(
                ERR_ILLEGAL_OPERATION,
                "only read requests are supported",
            )),
        }
    }

    /// Retransmit outstanding blocks whose acknowledgement is overdue.
    pub fn poll(&mut self, now: Instant) -> Vec<(ClientId, Vec<u8>)> {
        let mut due = Vec::new();
        self.transfers.retain(|client, transfer| {
            if now.duration_since(transfer.last_sent) < RETRANSMIT_TIMEOUT {
                return true;
            }
            if transfer.retries >= MAX_RETRIES {
                return false;
            }
            transfer.retries += 1;
            transfer.last_sent = now;
            due.push((*client, transfer.last_packet.clone()));
            true
        });
        due
    }

    fn start(&mut self, client: ClientId, request: &[u8], now: Instant) -> Vec<u8> {
        let Some((name, mode)) = parse_request(request) else {
            return error_packet(ERR_ILLEGAL_OPERATION, "malformed read request");
        };
        if !mode.eq_ignore_ascii_case("octet") && !mode.eq_ignore_ascii_case("netascii") {
            return error_packet(ERR_NOT_DEFINED, "mode must be octet or netascii");
        }
        let Some(path) = resolve(&self.root, name) else {
            return error_packet(ERR_FILE_NOT_FOUND, "file not found");
        };
        let Ok(metadata) = fs::metadata(&path) else {
            return error_packet(ERR_FILE_NOT_FOUND, "file not found");
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return error_packet(ERR_ACCESS_VIOLATION, "file is not readable");
        }
        let Ok(bytes) = fs::read(&path) else {
            return error_packet(ERR_ACCESS_VIOLATION, "file is not readable");
        };
        let data = if mode.eq_ignore_ascii_case("netascii") {
            netascii(&bytes)
        } else {
            bytes
        };

        let mut transfer = Transfer {
            data,
            block: 0,
            offset: 0,
            block_len: 0,
            final_block: false,
            last_packet: Vec::new(),
            retries: 0,
            last_sent: now,
        };
        let packet = send_block(&mut transfer, 1, now);
        self.transfers.insert(client, transfer);
        packet
    }

    fn acknowledge(&mut self, client: ClientId, block: u16, now: Instant) -> Option<Vec<u8>> {
        let transfer = self.transfers.get_mut(&client)?;
        // A duplicate or stale ACK neither retires nor advances the transfer.
        if block != transfer.block {
            return None;
        }
        if transfer.final_block {
            self.transfers.remove(&client);
            return None;
        }
        let next = transfer.block.wrapping_add(1);
        let offset = transfer.offset + transfer.block_len;
        Some(send_block_at(transfer, next, offset, now))
    }
}

fn send_block(transfer: &mut Transfer, block: u16, now: Instant) -> Vec<u8> {
    let offset = transfer.offset;
    send_block_at(transfer, block, offset, now)
}

fn send_block_at(transfer: &mut Transfer, block: u16, offset: usize, now: Instant) -> Vec<u8> {
    let end = (offset + BLOCK_SIZE).min(transfer.data.len());
    let data = &transfer.data[offset.min(end)..end];
    let mut packet = vec![0u8; 4 + data.len()];
    packet[0..2].copy_from_slice(&OP_DATA.to_be_bytes());
    packet[2..4].copy_from_slice(&block.to_be_bytes());
    packet[4..].copy_from_slice(data);
    transfer.block = block;
    transfer.offset = offset;
    transfer.block_len = data.len();
    transfer.final_block = data.len() < BLOCK_SIZE;
    transfer.retries = 0;
    transfer.last_sent = now;
    transfer.last_packet = packet.clone();
    packet
}

fn parse_request(request: &[u8]) -> Option<(&str, &str)> {
    let name_end = request.iter().position(|&byte| byte == 0)?;
    let mode_start = name_end + 1;
    let mode_end = request[mode_start..].iter().position(|&byte| byte == 0)? + mode_start;
    let name = std::str::from_utf8(&request[..name_end]).ok()?;
    let mode = std::str::from_utf8(&request[mode_start..mode_end]).ok()?;
    if name.is_empty() {
        return None;
    }
    Some((name, mode))
}

fn netascii(bytes: &[u8]) -> Vec<u8> {
    let mut translated = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    for &byte in bytes {
        match byte {
            b'\n' => translated.extend_from_slice(b"\r\n"),
            b'\r' => translated.extend_from_slice(b"\r\0"),
            _ => translated.push(byte),
        }
    }
    translated
}

fn resolve(root: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let relative = Path::new(name.trim_start_matches('/'));
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let root = fs::canonicalize(root).ok()?;
    let full = fs::canonicalize(root.join(relative)).ok()?;
    (full.starts_with(&root) && full.is_file()).then_some(full)
}

fn error_packet(code: u16, message: &str) -> Vec<u8> {
    let mut packet = vec![0u8; 4 + message.len() + 1];
    packet[0..2].copy_from_slice(&OP_ERROR.to_be_bytes());
    packet[2..4].copy_from_slice(&code.to_be_bytes());
    packet[4..4 + message.len()].copy_from_slice(message.as_bytes());
    packet
}
