//! Z8530 asynchronous byte interface with receive, transmit and BREAK interrupts.
//! The sun4m interrupt controller uses interrupts without SCC acknowledge;
//! software acknowledge, line timing and synchronous protocols remain pending.
use crate::bus::BusErrorKind;
use std::collections::VecDeque;

pub const OUTPUT_CAPACITY: usize = 4096;
const RX_CAPACITY: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    B,
    A,
}
impl Channel {
    fn index(self) -> usize {
        match self {
            Self::B => 0,
            Self::A => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Port {
    wr: [u8; 16],
    rx: VecDeque<u8>,
    output: VecDeque<u8>,
    tx: Option<u8>,
    underrun: bool,
    break_active: bool,
    break_latch: Option<bool>,
    rx_pending: bool,
    rx_first_armed: bool,
    tx_pending: bool,
}

impl Default for Port {
    fn default() -> Self {
        let mut port = Self {
            wr: [0; 16],
            rx: VecDeque::new(),
            output: VecDeque::new(),
            tx: None,
            underrun: true,
            break_active: false,
            break_latch: None,
            rx_pending: false,
            rx_first_armed: false,
            tx_pending: false,
        };
        port.reset(true);
        port
    }
}
impl Port {
    fn reset(&mut self, hardware: bool) {
        self.wr[0] = 0;
        self.wr[1] &= 0x24;
        self.wr[3] &= !1;
        self.wr[4] |= 4;
        self.wr[5] &= 0x61;
        self.wr[10] &= 0x60;
        self.wr[14] = (self.wr[14] & 0xc3) | 0x20;
        self.wr[15] = 0xf8;
        if hardware {
            self.wr[10] = 0;
            self.wr[11] = 8;
            self.wr[14] = 0x30;
        }
        self.rx.clear();
        self.tx = None;
        self.underrun = true;
        self.break_active = false;
        self.break_latch = None;
        self.rx_pending = false;
        self.rx_first_armed = false;
        self.tx_pending = false;
        // Already transmitted bytes belong to the host endpoint; guest reset
        // cannot retract them. Keep output until the owner drains it.
    }
    fn mask(code: u8) -> u8 {
        match code & 3 {
            0 => 0x1f,
            1 => 0x7f,
            2 => 0x3f,
            _ => 0xff,
        }
    }
    fn receive(&mut self, value: u8) -> bool {
        if self.wr[3] & 1 == 0 || self.rx.len() == RX_CAPACITY || self.break_active {
            return false;
        }
        self.rx.push_back(value & Self::mask(self.wr[3] >> 6));
        self.update_receive_interrupt();
        true
    }
    fn update_receive_interrupt(&mut self) {
        match (self.wr[1] >> 3) & 3 {
            1 => {
                if self.rx_first_armed && !self.rx.is_empty() {
                    self.rx_pending = true;
                    self.rx_first_armed = false;
                }
            }
            2 => self.rx_pending = !self.rx.is_empty(),
            // Host byte delivery has no framing/parity/overrun errors.
            _ => self.rx_pending = false,
        }
    }
    fn pending(&self) -> u8 {
        (u8::from(self.rx_pending) << 2)
            | (u8::from(self.tx_pending) << 1)
            | u8::from(self.wr[1] & 1 != 0 && self.break_latch.is_some())
    }
    fn reset_external_status(&mut self) {
        // A persistent transition while the status was latched requests one
        // more interrupt after reset (Zilog 1992, 2.4.9 and 5.2.1).
        self.break_latch = match self.break_latch {
            Some(previous) if previous != self.break_active && self.wr[15] & 0x80 != 0 => {
                Some(self.break_active)
            }
            _ => None,
        };
    }
    fn can_send(&self, wr3: u8, wr14: u8) -> Result<(), BusErrorKind> {
        if wr14 & 0x10 != 0 {
            if wr3 & 1 != 0 && self.rx.len() == RX_CAPACITY {
                return Err(BusErrorKind::WouldBlock);
            }
        } else if self.output.len() == OUTPUT_CAPACITY {
            return Err(BusErrorKind::WouldBlock);
        }
        Ok(())
    }
    fn send(&mut self, value: u8) {
        let value = value & Self::mask(self.wr[5] >> 5);
        if self.wr[14] & 0x10 != 0 {
            self.receive(value);
        } else {
            self.output.push_back(value);
        }
        // The one-byte transmit buffer just became empty. Enabling interrupts
        // on an already empty buffer does not set TxIP (section 2.4.8.1).
        self.tx_pending = self.wr[1] & 2 != 0;
    }
    fn data_write(&mut self, value: u8) -> Result<(), BusErrorKind> {
        if self.tx.is_some() {
            return Err(BusErrorKind::WouldBlock);
        }
        if self.wr[5] & 8 != 0 {
            self.can_send(self.wr[3], self.wr[14])?;
            self.send(value);
        } else {
            self.tx_pending = false;
            self.tx = Some(value);
        }
        Ok(())
    }
    fn data_read(&mut self) -> u8 {
        let value = self.rx.pop_front().unwrap_or(0);
        self.rx_pending = false;
        self.update_receive_interrupt();
        value
    }
    fn write_register(&mut self, reg: usize, value: u8) -> Result<(), BusErrorKind> {
        match reg {
            1 if value & 0x80 != 0 => return Err(BusErrorKind::Unsupported("SCC WAIT/DMA mode")),
            3 if value & 0x1e != 0 => {
                return Err(BusErrorKind::Unsupported("SCC synchronous receive mode"))
            }
            4 if value & 0x0c == 0 => {
                return Err(BusErrorKind::Unsupported("SCC synchronous mode"))
            }
            5 if value & 0x15 != 0 => {
                return Err(BusErrorKind::Unsupported("SCC CRC or transmit break"))
            }
            10 if value != 0 => return Err(BusErrorKind::Unsupported("SCC encoding/SDLC mode")),
            // WR14's clock-source/encoding commands configure a disabled DPLL;
            // only Enter Search Mode enables it (Zilog 1992, section 5.2.16).
            14 if value & 0x0c != 0 || value & 0xe0 == 0x20 => {
                return Err(BusErrorKind::Unsupported("SCC DPLL/auto-echo mode"))
            }
            15 if value & 5 != 0 => {
                return Err(BusErrorKind::Unsupported("SCC extended register/FIFO mode"))
            }
            _ => {}
        }
        if reg == 8 {
            return self.data_write(value);
        }
        if reg == 5 && value & 8 != 0 && self.tx.is_some() {
            self.can_send(self.wr[3], self.wr[14])?;
        }
        let previous = self.wr[reg];
        self.wr[reg] = value;
        if reg == 1 {
            if (value ^ previous) & 0x18 != 0 {
                self.rx_pending = false;
                self.rx_first_armed = value & 0x18 == 8;
            }
            self.update_receive_interrupt();
            if value & 2 == 0 {
                self.tx_pending = false;
            }
        }
        if reg == 5 && value & 8 != 0 {
            if let Some(value) = self.tx.take() {
                self.send(value);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Serial {
    ports: [Port; 2],
    pointer: u8,
    vector: u8,
    master: u8,
}
impl Default for Serial {
    fn default() -> Self {
        Self {
            ports: [Port::default(), Port::default()],
            pointer: 0,
            vector: 0,
            master: 0xc0,
        }
    }
}
impl Serial {
    pub fn reset(&mut self) {
        for port in &mut self.ports {
            port.reset(true);
        }
        self.pointer = 0;
        self.master = 0xc0;
    }
    pub fn receive(&mut self, channel: Channel, value: u8) -> bool {
        self.ports[channel.index()].receive(value)
    }
    /// Supply a decoded asynchronous BREAK condition. Termination queues one
    /// null byte, as Z8530 section 4.2.2 specifies. FIFO-full release is retryable.
    /// Bit timing and parity errors remain pending.
    pub fn set_break(&mut self, channel: Channel, asserted: bool) -> Result<(), BusErrorKind> {
        let port = &mut self.ports[channel.index()];
        if port.wr[3] & 1 == 0 || port.break_active == asserted {
            return Ok(());
        }
        if !asserted {
            if port.wr[4] & 3 == 1 {
                return Err(BusErrorKind::Unsupported(
                    "SCC odd-parity BREAK termination",
                ));
            }
            if port.rx.len() == RX_CAPACITY {
                return Err(BusErrorKind::WouldBlock);
            }
        }
        port.break_active = asserted;
        if !asserted {
            port.receive(0);
        }
        if port.wr[15] & 0x80 != 0 && port.break_latch.is_none() {
            port.break_latch = Some(asserted);
        }
        Ok(())
    }
    pub fn take_output(&mut self, channel: Channel) -> Vec<u8> {
        self.ports[channel.index()].output.drain(..).collect()
    }
    pub(crate) fn pop_output(&mut self, channel: Channel) -> Option<u8> {
        self.ports[channel.index()].output.pop_front()
    }

    fn pending(&self) -> u8 {
        (self.ports[1].pending() << 3) | self.ports[0].pending()
    }

    pub fn irq(&self) -> bool {
        self.master & 8 != 0 && self.pending() != 0
    }

    fn status_vector(&self) -> u8 {
        let pending = self.pending();
        let code: u8 = [(0x20, 6), (0x10, 4), (8, 5), (4, 2), (2, 0), (1, 1)]
            .into_iter()
            .find_map(|(mask, code)| (pending & mask != 0).then_some(code))
            .unwrap_or(3);
        if self.master & 0x10 != 0 {
            (self.vector & !0x70) | (code.reverse_bits() >> 1)
        } else {
            (self.vector & !0x0e) | (code << 1)
        }
    }

    pub fn read(&mut self, offset: u64) -> Result<u8, BusErrorKind> {
        if offset >= 8 {
            return Err(BusErrorKind::Unmapped);
        }
        let channel = ((offset >> 2) & 1) as usize;
        if offset & 2 != 0 {
            return Ok(self.ports[channel].data_read());
        }
        let reg = self.pointer;
        self.pointer = 0;
        if matches!(reg, 2 | 6) && channel == 0 {
            return Ok(self.status_vector());
        }
        if matches!(reg, 3 | 7) {
            return Ok(if channel == 1 { self.pending() } else { 0 });
        }
        let port = &mut self.ports[channel];
        Ok(match reg {
            0 | 4 => {
                let break_status = if port.wr[15] & 0x80 != 0 {
                    port.break_latch.unwrap_or(port.break_active)
                } else {
                    port.break_active
                };
                0x28 | if port.underrun { 0x40 } else { 0 }
                    | if port.tx.is_none() { 4 } else { 0 }
                    | if break_status { 0x80 } else { 0 }
                    | u8::from(!port.rx.is_empty())
            }
            1 | 5 => 6 | u8::from(port.tx.is_none()),
            2 | 6 => self.vector,
            8 => port.data_read(),
            9 | 13 => port.wr[13],
            10 | 14 => 0,
            11 | 15 => port.wr[15] & 0xfa,
            12 => port.wr[12],
            _ => unreachable!("four-bit SCC register pointer"),
        })
    }

    pub fn write(&mut self, offset: u64, value: u8) -> Result<(), BusErrorKind> {
        if offset >= 8 {
            return Err(BusErrorKind::Unmapped);
        }
        let channel = ((offset >> 2) & 1) as usize;
        if offset & 2 != 0 {
            return self.ports[channel].data_write(value);
        }
        let reg = self.pointer;
        if reg == 0 {
            // CRC generator/checker resets are valid during initialization.
            // They have no observable effect in the supported asynchronous
            // modes, where neither CRC engine is used (Zilog 1992, 5.2.1).
            if value & 0x38 == 0x18 {
                return Err(BusErrorKind::Unsupported("SCC send abort command"));
            }
            if value & 0xc0 == 0xc0 {
                self.ports[channel].underrun = false;
            }
            let port = &mut self.ports[channel];
            match value & 0x38 {
                0x10 => port.reset_external_status(),
                0x20 => {
                    port.rx_first_armed = true;
                    port.update_receive_interrupt();
                }
                0x28 => port.tx_pending = false,
                // Error Reset has no error FIFO to unlock. Reset Highest IUS
                // has no effect without an SCC interrupt acknowledge cycle.
                _ => {}
            }
            self.pointer = (value & 7) | if value & 0x38 == 8 { 8 } else { 0 };
            return Ok(());
        }
        match reg {
            2 => self.vector = value,
            9 => {
                if value & 0x20 != 0 {
                    return Err(BusErrorKind::Unsupported(
                        "SCC software interrupt acknowledge",
                    ));
                }
                match value & 0xc0 {
                    0x40 => self.ports[0].reset(false),
                    0x80 => self.ports[1].reset(false),
                    0xc0 => {
                        self.ports[0].reset(true);
                        self.ports[1].reset(true);
                    }
                    _ => {}
                }
                self.master = if value & 0xc0 == 0xc0 {
                    (value & 3) | 0xc0
                } else {
                    value & !0xc0
                };
            }
            _ => self.ports[channel].write_register(usize::from(reg), value)?,
        }
        self.pointer = 0;
        Ok(())
    }
}
