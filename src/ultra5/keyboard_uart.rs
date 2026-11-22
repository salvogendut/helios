//! Polled configuration of the Ultra 5's Super I/O UART1 with no keyboard attached.
//! National PC87332, §§6.2–6.5: LCR.DLAB selects the divisor at offsets 0/1.

#[derive(Default)]
pub(super) struct KeyboardUart {
    divisor: [u8; 2],
    line_control: u8,
    modem_control: u8,
    fifo_enabled: bool,
}

impl KeyboardUart {
    pub(super) fn read(&self, register: u8) -> Result<u8, String> {
        match register {
            0 | 1 if self.line_control & 0x80 != 0 => Ok(self.divisor[register as usize]),
            1 => Ok(0),                                        // IER: polled operation
            2 => Ok(if self.fifo_enabled { 0xc1 } else { 1 }), // IIR: no pending interrupt
            3 => Ok(self.line_control),
            4 => Ok(self.modem_control),
            5 => Ok(0x60), // LSR: no received byte; both transmitter stages empty
            _ => Err(format!(
                "unsupported Ultra 5 keyboard UART read at +{register:#x}"
            )),
        }
    }

    pub(super) fn write(&mut self, register: u8, value: u8) -> Result<(), String> {
        match register {
            0 | 1 if self.line_control & 0x80 != 0 => self.divisor[register as usize] = value,
            1 if value == 0 => {} // interrupts disabled
            // The RX/TX reset bits are strobes; both FIFOs are already empty.
            2 if value & !0x07 == 0 => self.fifo_enabled = value & 1 != 0,
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
}
