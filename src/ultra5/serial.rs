//! Polled SAB 82532 transmit path used by Ultra 5 OBP. Each channel has a
//! 32-byte CPU-accessible transmit pool. XF hands that pool to the transmitter.

const STATUS_COMMAND: u8 = 0x20;
const CCR0: u8 = 0x2c;
const IPC: u8 = 0x39;

#[derive(Default)]
pub(super) struct Serial {
    channels: [Channel; 2],
    ipc: u8,
}

#[derive(Default)]
struct Channel {
    registers: [u8; 32],
    fifo: [u8; 32],
    used: usize,
    transmitted: Vec<u8>,
}

impl Serial {
    pub fn ccr0(&self) -> [u8; 2] {
        std::array::from_fn(|i| self.channels[i].registers[usize::from(CCR0 - 0x20)])
    }

    pub fn take_output(&mut self) -> [Vec<u8>; 2] {
        std::array::from_fn(|i| std::mem::take(&mut self.channels[i].transmitted))
    }

    pub fn read(&self, offset: u8) -> Result<u8, String> {
        let channel = &self.channels[usize::from(offset / 0x40)];
        let reg = offset % 0x40;
        match reg {
            STATUS_COMMAND => Ok(if channel.tx_ready() { 0x40 } else { 0 }), // STAR.XFW
            IPC => Ok(self.ipc),
            CCR0 | 0x22 | 0x27 | 0x28 | 0x2d | 0x2e | 0x3f => {
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
                0x41 | 0x01 => channel.used = 0, // XRES, optionally with RRES
                0x40 => (), // RRES: receiver starts empty in this transmit-only fixture
                0x08 if channel.registers[usize::from(CCR0 - 0x20)] & 0x83 == 0x83 => {
                    channel
                        .transmitted
                        .extend_from_slice(&channel.fifo[..channel.used]);
                    channel.used = 0;
                }
                _ => return Err(format!("unsupported SAB 82532 command {value:#04x}")),
            },
            CCR0 if value & 0x20 != 0 => {
                return Err(format!("reserved SAB 82532 CCR0 bit in {value:#04x}"))
            }
            IPC => self.ipc = value,
            // Boot-time ASYNC configuration. BGR, IMR0/1 and XBCH have
            // distinct read aliases; reads of these offsets stop explicitly.
            CCR0 | 0x22 | 0x27 | 0x28 | 0x2b | 0x2d | 0x2e | 0x34 | 0x3a | 0x3b | 0x3f => {
                channel.registers[usize::from(reg - 0x20)] = value;
            }
            _ => return Err(format!("unsupported SAB 82532 write at offset {offset:#x}")),
        }
        Ok(())
    }
}

impl Channel {
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
        assert!(dev.read(0x3a).is_err()); // ISR0 is a different register
    }
}
