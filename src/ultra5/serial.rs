//! Polled SAB 82532 serial path used by Ultra 5 OBP. Each channel has a
//! 32-byte CPU-accessible transmit pool. XF hands that pool to the transmitter.

use std::collections::VecDeque;

const STATUS_COMMAND: u8 = 0x20;
const CCR0: u8 = 0x2c;
const IPC: u8 = 0x39;

pub(super) struct Serial {
    channels: [Channel; 2],
    ipc: u8,
    pvr: u8,
    pcr: u8,
    pim: u8,
    iva: u8,
}

impl Default for Serial {
    fn default() -> Self {
        let mut channels = [Channel::default(), Channel::default()];
        for ch in &mut channels {
            ch.registers[0x1a..0x1c].fill(0xff);
        }
        Self {
            channels,
            ipc: 0,
            pvr: 0,
            pcr: 0xff,
            pim: 0xff,
            iva: 0,
        }
    }
}

#[derive(Default)]
struct Channel {
    registers: [u8; 32],
    fifo: [u8; 32],
    used: usize,
    transmitted: Vec<u8>,
    timeout_at: Option<u64>,
    rx: VecDeque<u8>,
    rx_pool: usize,
    rx_read: usize,
    isr0: u8,
    isr1: u8,
}

impl Serial {
    pub fn advance_to(&mut self, now: u64) {
        for ch in &mut self.channels {
            if ch.timeout_at.is_some_and(|deadline| deadline <= now) {
                ch.timeout_at = None;
                if ch.rx_pool == 0 && !ch.rx.is_empty() {
                    ch.isr0 |= 0x40;
                }
            }
        }
    }

    pub fn ccr0(&self) -> [u8; 2] {
        std::array::from_fn(|i| self.channels[i].registers[usize::from(CCR0 - 0x20)])
    }

    pub fn take_output(&mut self) -> [Vec<u8>; 2] {
        std::array::from_fn(|i| std::mem::take(&mut self.channels[i].transmitted))
    }

    /// A complete space-to-mark BREAK pulse on a powered asynchronous channel.
    pub fn receive_break_a(&mut self) -> bool {
        let ch = &mut self.channels[0];
        if ch.registers[0x0c] & 0x83 != 0x83 || ch.registers[2] & 8 == 0 {
            return false;
        }
        ch.isr1 |= 0xc0; // BRK and BRKT, acknowledge on ISR1 read
        true
    }

    pub fn receive(&mut self, port: usize, byte: u8, now: u64) -> bool {
        let Some(ch) = self.channels.get_mut(port) else {
            return false;
        };
        if ch.registers[0x0c] & 0x83 != 0x83 || ch.registers[2] & 8 == 0 {
            return false;
        }
        let status = ch.registers[8] & 0x10 != 0;
        let capacity = if ch.rx_pool == 0 {
            32
        } else {
            ch.rx_pool - ch.rx_read + 32
        };
        if ch.rx.len() + 1 + usize::from(status) > capacity {
            return false;
        }
        ch.rx.push_back(byte);
        if status {
            ch.rx.push_back(0);
        }
        ch.check_threshold();
        ch.timeout_at = Some(now.saturating_add(ch.receive_timeout_ns()));
        true
    }

    pub fn read(&mut self, offset: u8) -> Result<u8, String> {
        let channel = &mut self.channels[usize::from(offset / 0x40)];
        let reg = offset % 0x40;
        match reg {
            0..=0x1f if channel.rx_read < channel.rx_pool => {
                channel.rx_read += 1;
                Ok(channel.rx.pop_front().unwrap())
            }
            STATUS_COMMAND => {
                Ok((u8::from(channel.tx_ready()) << 6) | (u8::from(!channel.rx.is_empty()) << 5))
            }
            0x34 => Ok(0x02), // VSTR, SAB 82532 revision 3.2
            0x38 => Ok(self.channels.iter().enumerate().fold(0, |v, (i, c)| {
                let isr = u8::from(c.isr0 & !c.registers[0x1a] != 0)
                    | (u8::from(c.isr1 & !c.registers[0x1b] != 0) << 1);
                v | (isr << (2 - 2 * i))
            })),
            0x3c => Ok(self.pvr | self.pcr), // undriven input pins pulled up
            0x3e => Ok(self.pcr),
            0x3d => Ok(0), // no external port transitions
            0x2a => Ok((channel.rx_pool & 31) as u8),
            0x3a => Ok(std::mem::take(&mut channel.isr0)
                & if self.ipc & 0x80 != 0 {
                    0xff
                } else {
                    !channel.registers[0x1a]
                }),
            0x3b => Ok(std::mem::take(&mut channel.isr1)
                & if self.ipc & 0x80 != 0 {
                    0xff
                } else {
                    !channel.registers[0x1b]
                }),
            IPC => Ok(self.ipc),
            CCR0 | 0x22..=0x28 | 0x2d..=0x2f | 0x3f => {
                Ok(channel.registers[usize::from(reg - 0x20)])
            }
            _ => Err(format!("unsupported SAB 82532 read at offset {offset:#x}")),
        }
    }

    pub fn write(&mut self, offset: u8, value: u8) -> Result<(), String> {
        let channel = &mut self.channels[usize::from(offset / 0x40)];
        let reg = offset % 0x40;
        match reg {
            0..=0x1f => {
                if !channel.tx_ready() {
                    return Err("SAB 82532 transmit pool unavailable".into());
                }
                channel.fifo[channel.used] = value;
                channel.used += 1;
            }
            STATUS_COMMAND => match value {
                0x01 | 0x40 | 0x41 => {
                    if value & 1 != 0 {
                        channel.used = 0;
                        channel.isr1 = 0;
                    }
                    if value & 0x40 != 0 {
                        channel.timeout_at = None;
                        channel.rx.clear();
                        channel.rx_read = 0;
                        channel.rx_pool = 0;
                        channel.isr0 = 0;
                    }
                }
                0x20 => {
                    channel.timeout_at = None;
                    if channel.rx_pool == 0 {
                        channel.rx_pool = channel.rx.len().min(32);
                    }
                    if channel.rx_pool != 0 {
                        channel.isr0 |= 0x80;
                    }
                }
                0x80 => {
                    channel.rx.drain(..channel.rx_pool - channel.rx_read);
                    channel.rx_pool = 0;
                    channel.rx_read = 0;
                    channel.isr0 &= !0x81;
                    channel.check_threshold();
                }
                0x08 if channel.registers[usize::from(CCR0 - 0x20)] & 0x83 == 0x83 => {
                    channel
                        .transmitted
                        .extend_from_slice(&channel.fifo[..channel.used]);
                    channel.used = 0;
                    channel.isr1 |= 0x21;
                }
                _ => return Err(format!("unsupported SAB 82532 command {value:#04x}")),
            },
            0x35 if channel.tx_ready() => {
                channel.transmitted.push(value);
                channel.isr1 |= 0x20;
            }
            CCR0 if value & 0x20 != 0 => {
                return Err(format!("reserved SAB 82532 CCR0 bit in {value:#04x}"))
            }
            IPC => self.ipc = value,
            0x38 => self.iva = value,
            0x3c => self.pvr = value,
            0x3d => self.pim = value,
            0x3e => self.pcr = value,
            // Writing BGR and the interrupt masks does not modify their read aliases.
            CCR0 | 0x22..=0x28 | 0x2b | 0x2d..=0x2f | 0x34 | 0x3a | 0x3b | 0x3f => {
                channel.registers[usize::from(reg - 0x20)] = value;
            }
            _ => return Err(format!("unsupported SAB 82532 write at offset {offset:#x}")),
        }
        Ok(())
    }
}

impl Channel {
    fn receive_timeout_ns(&self) -> u64 {
        // ASYNC receiver timeout is four full frames at the programmed rate.
        // The Ultra 5 ESCC2 baud generator uses a 29.4912 MHz input.
        let dafo = self.registers[7];
        let bits = u64::from(10 - (dafo & 3) + ((dafo >> 2) & 1) + ((dafo >> 5) & 1));
        let br = u64::from(self.registers[0x14]) | (u64::from(self.registers[0xe] & 0xc0) << 2);
        let divider = if self.registers[0x1f] & 0x40 != 0 {
            ((br & 63) + 1) << (br >> 6)
        } else {
            br + 1
        };
        (4 * bits * 16 * divider * 1_000_000_000u64).div_ceil(29_491_200)
    }

    fn check_threshold(&mut self) {
        let threshold = [1, 4, 16, 32][usize::from((self.registers[8] >> 2) & 3)];
        if self.rx_pool == 0 && self.rx.len() >= threshold {
            self.rx_pool = threshold;
            self.isr0 |= 1;
        }
    }

    fn tx_ready(&self) -> bool {
        // Power enabled in ASYNC mode. Command and transmission complete
        // synchronously, so CEC is clear on the next status read.
        self.registers[usize::from(CCR0 - 0x20)] & 0x83 == 0x83 && self.used < self.fifo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_timeout_renews_and_exposes_a_short_receive_pool() {
        let mut serial = Serial::default();
        for (reg, value) in [
            (0x2c, 0xc3),
            (0x22, 8),
            (0x28, 0x4c),
            (0x3f, 0x40),
            (0x34, 0xaf),
            (0x27, 0),
            (0x39, 0x80),
            (0x3a, 0xff),
        ] {
            serial.write(reg, value).unwrap();
        }
        // 29.4912 MHz / (16 * 48 * 4) = 9600 baud, 8N1.
        assert!(serial.receive(0, b'A', 0));
        assert!(serial.receive(0, b'B', 1_000_000));
        serial.advance_to(5_166_666);
        assert_eq!(serial.read(0x3a).unwrap(), 0);
        serial.advance_to(5_166_667);
        assert_eq!(serial.read(0x38).unwrap(), 0); // masked sources do not assert GIS
        assert_eq!(serial.read(0x3a).unwrap(), 0x40);
        assert_eq!(serial.read(0x3a).unwrap(), 0);
        serial.write(0x20, 0x20).unwrap();
        assert_eq!(serial.read(0x3a).unwrap(), 0x80);
        assert_eq!(serial.read(0x2a).unwrap(), 2);
        assert_eq!([serial.read(0).unwrap(), serial.read(1).unwrap()], *b"AB");
        serial.write(0x20, 0x80).unwrap();
        assert!(serial.receive_break_a());
        assert_eq!(serial.read(0x3b).unwrap(), 0xc0);
        assert_eq!(serial.read(0x3b).unwrap(), 0);
    }

    #[test]
    fn polled_receive_releases_only_the_current_pool() {
        let mut serial = Serial::default();
        assert!(!serial.receive(0, b'X', 0));
        serial.write(0x2c, 0xc3).unwrap();
        serial.write(0x22, 8).unwrap();
        serial.write(0x39, 0x80).unwrap();
        serial.write(0x28, 0x0c).unwrap(); // threshold 32
        assert!(serial.receive(0, b'X', 0));
        assert_eq!(serial.read(0x20).unwrap() & 0x20, 0x20);
        serial.write(0x20, 0x20).unwrap();
        assert!(serial.receive(0, b'Y', 1000));
        assert_eq!(serial.read(0x3a).unwrap(), 0x80);
        assert_eq!(serial.read(0x3a).unwrap(), 0);
        assert_eq!(serial.read(0x2a).unwrap(), 1);
        assert_eq!(serial.read(0).unwrap(), b'X');
        serial.write(0x20, 0x80).unwrap();
        serial.write(0x20, 0x20).unwrap();
        assert_eq!(serial.read(0).unwrap(), b'Y');
        serial.write(0x20, 0x80).unwrap();
        assert_eq!(serial.read(0x20).unwrap() & 0x20, 0);
    }

    #[test]
    fn channels_require_power_and_a_transmit_command_and_keep_fifo_order() {
        let mut dev = Serial::default();
        assert_eq!(dev.read(0x20).unwrap(), 0);
        assert!(dev.write(0, 0x57).is_err());
        for addr in [0x2c, 0x6c] {
            dev.write(addr, 0xc3).unwrap();
        }
        for i in 0..32 {
            dev.write(0, i).unwrap();
        }
        assert_eq!(dev.read(0x20).unwrap(), 0);
        assert_eq!(dev.read(0x60).unwrap(), 0x40);
        assert!(dev.write(0, 0xff).is_err());
        dev.write(0x40, 0x52).unwrap();
        assert_eq!(dev.take_output(), [vec![], vec![]]);
        dev.write(0x60, 0x41).unwrap(); // reset channel B alone
        dev.write(0x40, 0x53).unwrap();
        dev.write(0x60, 8).unwrap();
        assert_eq!(dev.take_output(), [vec![], vec![0x53]]);
        dev.write(0x20, 8).unwrap();
        assert_eq!(dev.take_output(), [(0..32).collect::<Vec<_>>(), vec![]]);
        assert_eq!(dev.read(0x20).unwrap(), 0x40);
        assert_eq!(dev.take_output(), [vec![], vec![]]);
    }

    #[test]
    fn shared_ipc_and_asymmetric_register_aliases() {
        let mut dev = Serial::default();
        dev.write(0x39, 0x80).unwrap();
        assert_eq!(dev.read(0x79).unwrap(), 0x80);
        dev.write(0x2c, 0xc3).unwrap();
        assert!(dev.write(0x2c, 0xe3).is_err());
        assert_eq!(dev.read(0x2c).unwrap(), 0xc3);
        assert_eq!(dev.read(0x6c).unwrap(), 0);
        dev.write(0x3a, 0xff).unwrap();
        assert_eq!(dev.read(0x3a).unwrap(), 0); // ISR0 is independent of IMR0
    }
}
