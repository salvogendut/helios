//! Polled configuration of the Ultra 5's Super I/O UART1 with no keyboard attached.
//! National PC87332, §§6.2–6.5: LCR.DLAB selects the divisor at offsets 0/1.

#[derive(Default)]
pub(super) struct KeyboardUart {
    divisor: [u8; 2],
    line_control: u8,
    modem_control: u8,
    fifo_enabled: bool,
    tx_end_ns: u64,
    frame_ns: u64,
}

impl KeyboardUart {
    pub(super) fn read(&self, register: u8, now: u64) -> Result<u8, String> {
        match register {
            0 | 1 if self.line_control & 0x80 != 0 => Ok(self.divisor[register as usize]),
            1 => Ok(0),                                        // IER: polled operation
            2 => Ok(if self.fifo_enabled { 0xc1 } else { 1 }), // IIR: no pending interrupt
            3 => Ok(self.line_control),
            4 => Ok(self.modem_control),
            5 => Ok(match self.tx_end_ns.saturating_sub(now) {
                0 => 0x60,                                       // THRE and TEMT
                remaining if remaining <= self.frame_ns => 0x20, // shift register only
                _ => 0, // holding register or FIFO also occupied
            }),
            _ => Err(format!(
                "unsupported Ultra 5 keyboard UART read at +{register:#x}"
            )),
        }
    }

    pub(super) fn write(
        &mut self,
        register: u8,
        value: u8,
        now: u64,
        normal_clock: bool,
    ) -> Result<(), String> {
        match register {
            0 | 1 if self.line_control & 0x80 != 0 => self.divisor[register as usize] = value,
            0 => self.transmit(now, normal_clock)?,
            1 if value == 0 => {} // interrupts disabled
            2 if value & !0x07 == 0 => {
                let enable = value & 1 != 0;
                if (enable != self.fifo_enabled || (enable && value & 4 != 0))
                    && self.tx_end_ns > now
                {
                    // FIFO clears leave the byte already shifting on the wire.
                    self.tx_end_ns = now + (self.tx_end_ns - now - 1) % self.frame_ns + 1;
                }
                self.fifo_enabled = enable;
            }
            3 if value & 0x40 == 0 => self.line_control = value, // no BREAK generation
            4 if value & !0x0f == 0 => self.modem_control = value, // no loopback
            _ => {
                return Err(format!(
                    "unsupported Ultra 5 keyboard UART write at +{register:#x} value={value:#04x}"
                ))
            }
        }
        Ok(())
    }

    fn transmit(&mut self, now: u64, normal_clock: bool) -> Result<(), String> {
        let divisor = u64::from(u16::from_le_bytes(self.divisor));
        if !normal_clock || self.line_control != 3 || divisor == 0 {
            return Err("unsupported Ultra 5 keyboard UART transmit format or clock".into());
        }
        // PC87332 §6.3: 24 MHz / 13 / divisor / 16, ten bits per 8N1 frame.
        let frame = (10 * 13 * 16 * divisor * 1_000).div_ceil(24);
        let remaining = self.tx_end_ns.saturating_sub(now);
        if remaining != 0 {
            if self.frame_ns != frame {
                return Err("unsupported keyboard UART baud change during transmission".into());
            }
            let holding_capacity = if self.fifo_enabled { 16 } else { 1 };
            if remaining.div_ceil(frame) > holding_capacity {
                return Err("Ultra 5 keyboard UART transmit buffer full".into());
            }
        }
        // The disconnected wire consumes the byte. Only its duration and
        // queue occupancy can subsequently be observed by the guest.
        self.tx_end_ns = self.tx_end_ns.max(now).saturating_add(frame);
        self.frame_ns = frame;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_8n1_transmit_tracks_holding_fifo_and_shift_register() {
        let start = 300;
        let end = start + 8_320_000;
        for fifo in [false, true] {
            let mut uart = KeyboardUart::default();
            uart.write(3, 0x83, 0, true).unwrap();
            uart.write(0, 96, 0, true).unwrap();
            uart.write(3, 3, 0, true).unwrap();
            assert!(uart.write(0, 1, 0, false).is_err());
            uart.write(2, u8::from(fifo), start, true).unwrap();
            uart.write(0, 1, start, true).unwrap();
            assert_eq!(uart.read(5, start).unwrap(), 0x20);
            for _ in 0..if fifo { 16 } else { 1 } {
                uart.write(0, 0x42, start, true).unwrap();
            }
            assert!(uart.write(0, 0xff, start, true).is_err());
            assert_eq!(uart.read(5, start + 500).unwrap(), 0);
            // Switching mode or resetting the FIFO keeps the in-flight byte.
            uart.write(2, 5, start + 500, true).unwrap();
            assert_eq!(uart.read(5, end - 1).unwrap(), 0x20);
            assert_eq!(uart.read(5, end).unwrap(), 0x60);
            assert!(uart.read(0, end).is_err());
        }
    }
}
