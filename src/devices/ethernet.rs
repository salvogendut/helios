//! NCR92C990 MAC in the SS5 NCR89C100. See docs/ethernet.md.
//! Packets cross a bounded headless endpoint without preamble or FCS.
use crate::bus::BusErrorKind;
use crate::mmu::Width;
use std::collections::VecDeque;

const STOP: u16 = 4;
const IEN: u16 = 0x40;
const IDON: u16 = 0x100;
const IRQ_CAUSES: u16 = 0x5f00;
const QUEUE_LIMIT: usize = 64;
const POLL_NS: u64 = 1_600_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Transfer {
    pub address: u32,
    pub width: Width,
    pub write: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Init,
    TxDescriptor,
    TxData,
    TxStatus,
    RxDescriptor,
    RxData,
    RxCount,
    RxStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ethernet {
    rap: u16,
    csr: [u16; 4],
    initialized: bool,
    mode: u16,
    mac: [u8; 6],
    filter: u64,
    rings: [u32; 2],
    lengths: [u16; 2],
    positions: [u16; 2],
    phase: Phase,
    words: [u16; 12],
    index: usize,
    packet: Vec<u8>,
    poll_ns: u64,
    incoming: VecDeque<Vec<u8>>,
    outgoing: VecDeque<Vec<u8>>,
}

impl Default for Ethernet {
    fn default() -> Self {
        Self {
            rap: 0,
            csr: [STOP, 0, 0, 0],
            initialized: false,
            mode: 0,
            mac: [0; 6],
            filter: 0,
            rings: [0; 2],
            lengths: [1; 2],
            positions: [0; 2],
            phase: Phase::Idle,
            words: [0; 12],
            index: 0,
            packet: Vec::new(),
            poll_ns: 0,
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
        }
    }
}

impl Ethernet {
    pub fn reset(&mut self) {
        // Already transmitted frames remain available to the host endpoint.
        let outgoing = std::mem::take(&mut self.outgoing);
        *self = Self::default();
        self.outgoing = outgoing;
    }
    pub fn irq(&self) -> bool {
        self.csr[0] & IEN != 0 && self.csr[0] & IRQ_CAUSES != 0
    }
    pub fn mac_address(&self) -> Option<[u8; 6]> {
        self.initialized.then_some(self.mac)
    }
    pub fn take_frame(&mut self) -> Option<Vec<u8>> {
        self.outgoing.pop_front()
    }
    /// Enqueue an Ethernet frame without its FCS. False means endpoint full
    /// or invalid length; the caller retains responsibility for the frame.
    pub fn receive_frame(&mut self, frame: &[u8]) -> bool {
        if !(60..=1514).contains(&frame.len()) || self.incoming.len() == QUEUE_LIMIT {
            return false;
        }
        if self.csr[0] & 0x20 == 0 || self.loopback() {
            return true; // receiver disabled or disconnected from external pins
        }
        self.incoming.push_back(frame.to_vec());
        true
    }
    pub fn read(&self, offset: u64) -> Result<u16, BusErrorKind> {
        match offset {
            2 => Ok(self.rap),
            0 => {
                if self.rap == 0 {
                    let csr = self.csr[0];
                    Ok(csr
                        | if csr & IRQ_CAUSES != 0 { 0x80 } else { 0 }
                        | if csr & 0x7800 != 0 { 0x8000 } else { 0 })
                } else if self.rap == 3 || self.csr[0] & STOP != 0 {
                    Ok(self.csr[self.rap as usize])
                } else {
                    Err(BusErrorKind::Unsupported(
                        "active Ethernet address CSR read",
                    ))
                }
            }
            _ => Err(BusErrorKind::Unsupported("Ethernet register offset")),
        }
    }
    pub fn write(&mut self, offset: u64, value: u16) -> Result<(), BusErrorKind> {
        if offset == 2 {
            self.rap = value & 3;
            return Ok(());
        }
        if offset != 0 {
            return Err(BusErrorKind::Unsupported("Ethernet register offset"));
        }
        if self.rap != 0 {
            if self.rap == 3 && self.csr[0] & STOP == 0 && value & 7 != self.csr[3] & 7 {
                return Err(BusErrorKind::Unsupported(
                    "active Ethernet DMA bus-mode change",
                ));
            }
            if self.rap != 3 && self.csr[0] & STOP == 0 {
                return Err(BusErrorKind::Unsupported(
                    "active Ethernet address CSR write",
                ));
            }
            self.csr[self.rap as usize] = match self.rap {
                1 => value & 0xfffe,
                2 => value & 0xff,
                // NCR89C100 allows IPG programming while waiting for IDON.
                // Its upper CSR3 fields are not the PCnet interrupt mask.
                _ => {
                    if value & 0x8000 != 0 {
                        value & 0xfe07
                    } else {
                        value & 7
                    }
                }
            };
            return Ok(());
        }
        if value & STOP != 0 {
            self.csr[0] = STOP;
            self.csr[3] = 0;
            self.phase = Phase::Idle;
            self.packet.clear();
            self.incoming.clear();
            return Ok(());
        }
        if value & 3 == 3 || (value & 1 != 0 && self.csr[0] & STOP == 0) {
            return Err(BusErrorKind::Unsupported(
                "Ethernet INIT without STOP or with START",
            ));
        }
        if value & 2 != 0 && !self.initialized {
            return Err(BusErrorKind::Unsupported(
                "Ethernet START before initialization",
            ));
        }
        self.csr[0] &= !(value & 0x7f00);
        if value & 1 != 0 {
            self.csr[0] = 1;
            self.csr[3] &= 7;
            self.initialized = false;
            self.phase = Phase::Init;
            self.words = [0; 12];
            self.index = 0;
        }
        if value & 2 != 0 {
            self.csr[0] = (self.csr[0] & !STOP) | 2;
            if self.mode & 1 == 0 {
                self.csr[0] |= 0x20;
            }
            if self.mode & 2 == 0 {
                self.csr[0] |= 0x10;
            }
            self.poll_ns = 0;
        }
        if self.csr[0] & STOP == 0 {
            self.csr[0] = (self.csr[0] & !IEN) | (value & IEN);
            self.csr[0] |= value & 8;
        }
        Ok(())
    }
    pub(crate) fn advance_ns(&mut self, ns: u64) {
        self.poll_ns = self.poll_ns.saturating_sub(ns);
    }
    fn descriptor_address(&self, tx: bool) -> u32 {
        let ring = usize::from(tx);
        self.rings[ring].wrapping_add(u32::from(self.positions[ring]) * 8)
    }
    fn buffer_address(&self) -> u32 {
        u32::from(self.words[0]) | (u32::from(self.words[1] & 255) << 16)
    }
    fn packet_address(&self) -> u32 {
        let address = self.buffer_address().wrapping_add(self.index as u32);
        // SS5 wires the MAC's 16-bit bus in big-endian word order. Metadata
        // is always word-transparent; SWAP only controls packet byte lanes.
        address ^ if self.csr[3] & 4 != 0 { 0 } else { 1 }
    }
    fn accepts(&self, frame: &[u8]) -> bool {
        self.mode & 0x8000 != 0
            || frame[..6] == self.mac
            || frame[..6] == [255; 6]
            || (frame[0] & 1 != 0 && self.filter & (1 << (crc(&frame[..6]) >> 26)) != 0)
    }
    fn loopback(&self) -> bool {
        self.mode & 0x44 == 0x44
    }
    pub(crate) fn request(&mut self) -> Option<Transfer> {
        if self.phase == Phase::Idle && self.csr[0] & 0x20 != 0 {
            // Filter at most one queued frame per scheduling boundary.
            if let Some(frame) = self.incoming.pop_front() {
                if self.accepts(&frame) {
                    self.packet = frame;
                    let fcs = !crc(&self.packet);
                    self.packet.extend_from_slice(&fcs.to_le_bytes());
                    self.phase = Phase::RxDescriptor;
                    self.index = 0;
                }
            }
        }
        if self.phase == Phase::Idle
            && self.csr[0] & 0x10 != 0
            && (self.csr[0] & 8 != 0 || self.poll_ns == 0)
            && (self.loopback() || self.outgoing.len() < QUEUE_LIMIT)
        {
            self.phase = Phase::TxDescriptor;
            self.index = 0;
        }
        let (address, width, write) = match self.phase {
            Phase::Idle => return None,
            Phase::Init => (
                u32::from(self.csr[1]) | u32::from(self.csr[2]) << 16,
                Width::Half,
                None,
            ),
            Phase::TxDescriptor | Phase::RxDescriptor => (
                self.descriptor_address(self.phase == Phase::TxDescriptor),
                Width::Half,
                None,
            ),
            Phase::TxData => (self.packet_address(), Width::Byte, None),
            Phase::TxStatus => (
                self.descriptor_address(true) + 2,
                Width::Half,
                Some(u32::from(self.words[1] & 0x03ff)),
            ),
            Phase::RxData => (
                self.packet_address(),
                Width::Byte,
                Some(u32::from(self.packet[self.index])),
            ),
            Phase::RxCount => (
                self.descriptor_address(false) + 6,
                Width::Half,
                Some(self.packet.len() as u32),
            ),
            Phase::RxStatus => (
                self.descriptor_address(false) + 2,
                Width::Half,
                Some(u32::from((self.words[1] & 255) | 0x300)),
            ),
        };
        let address = if matches!(
            self.phase,
            Phase::Init | Phase::TxDescriptor | Phase::RxDescriptor
        ) {
            address.wrapping_add(self.index as u32 * 2)
        } else {
            address
        };
        Some(Transfer {
            address,
            width,
            write,
        })
    }
    pub(crate) fn complete(&mut self, value: u32) -> Result<(), BusErrorKind> {
        match self.phase {
            Phase::Init => {
                if self.index == 0 && (value & !0x8047 != 0 || value & 0x44 == 4) {
                    return Err(BusErrorKind::Unsupported("Ethernet loopback or test mode"));
                }
                self.words[self.index] = value as u16;
                self.index += 1;
                if self.index == 12 {
                    self.mode = self.words[0];
                    for i in 0..3 {
                        self.mac[i * 2] = self.words[i + 1] as u8;
                        self.mac[i * 2 + 1] = (self.words[i + 1] >> 8) as u8;
                    }
                    self.filter =
                        (0..4).fold(0, |a, i| a | (u64::from(self.words[i + 4]) << (i * 16)));
                    for i in 0..2 {
                        let low = self.words[8 + i * 2];
                        let high = self.words[9 + i * 2];
                        self.rings[i] = u32::from(low & !7) | (u32::from(high & 255) << 16);
                        self.lengths[i] = 1 << (high >> 13);
                    }
                    self.positions = [0; 2];
                    self.initialized = true;
                    self.csr[0] |= IDON;
                    self.phase = Phase::Idle;
                }
            }
            Phase::TxDescriptor | Phase::RxDescriptor => {
                let tx = self.phase == Phase::TxDescriptor;
                // Validate the last descriptor read before consuming it, so a
                // host stop leaves an identical pending request for retry.
                if self.index == 3 && self.words[1] & 0x8000 != 0 {
                    if self.words[2] & 0xf000 != 0xf000 {
                        return Err(BusErrorKind::Unsupported("Ethernet buffer length encoding"));
                    }
                    let length = 0x1000 - usize::from(self.words[2] & 0xfff);
                    let valid_length = if self.loopback() {
                        (8..=32).contains(&length)
                    } else {
                        (60..=1514).contains(&length)
                    };
                    if tx && (self.words[1] & 0x300 != 0x300 || !valid_length) {
                        return Err(BusErrorKind::Unsupported(
                            "Ethernet chained, runt or oversized transmit",
                        ));
                    }
                    if !tx && length < self.packet.len() {
                        return Err(BusErrorKind::Unsupported("Ethernet chained receive"));
                    }
                }
                self.words[self.index] = value as u16;
                self.index += 1;
                if self.index == 4 {
                    self.index = 0;
                    if self.words[1] & 0x8000 == 0 {
                        self.phase = Phase::Idle;
                        if !tx {
                            self.csr[0] |= 0x1000;
                        }
                    } else {
                        self.phase = if tx { Phase::TxData } else { Phase::RxData };
                    }
                    if tx {
                        self.packet.clear();
                        self.csr[0] &= !8;
                        self.poll_ns = POLL_NS;
                    }
                }
            }
            Phase::TxData => {
                self.packet.push(value as u8);
                self.index += 1;
                if self.index == 0x1000 - usize::from(self.words[2] & 0xfff) {
                    self.phase = Phase::TxStatus;
                }
            }
            Phase::RxData => {
                self.index += 1;
                if self.index == self.packet.len() {
                    self.phase = Phase::RxCount;
                }
            }
            Phase::RxCount => self.phase = Phase::RxStatus,
            Phase::TxStatus | Phase::RxStatus => {
                let tx = self.phase == Phase::TxStatus;
                let ring = usize::from(tx);
                self.positions[ring] = (self.positions[ring] + 1) & (self.lengths[ring] - 1);
                self.csr[0] |= if tx { 0x200 } else { 0x400 };
                if tx {
                    if self.loopback() {
                        if self.incoming.len() < QUEUE_LIMIT && self.csr[0] & 0x20 != 0 {
                            self.incoming.push_back(std::mem::take(&mut self.packet));
                        } else if self.csr[0] & 0x20 != 0 {
                            self.csr[0] |= 0x1000;
                        }
                    } else {
                        self.outgoing.push_back(std::mem::take(&mut self.packet));
                    }
                }
                self.phase = Phase::Idle;
                self.poll_ns = 0;
            }
            Phase::Idle => unreachable!("completion without DMA request"),
        }
        Ok(())
    }
    pub(crate) fn memory_error(&mut self) {
        self.csr[0] = (self.csr[0] & !0x38) | 0x800;
        self.phase = Phase::Idle;
    }
}

// Ethernet's reflected CRC, before the final complement. The same CRC
// feeds the multicast hash and the FCS appended at the packet boundary.
fn crc(bytes: &[u8]) -> u32 {
    let mut value = u32::MAX;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ if value & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    value
}
