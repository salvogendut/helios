//! Ultra 5 Super I/O UARTs and their serial input devices.
//! National PC87332, §§6.2–6.5: LCR.DLAB selects the divisor at offsets 0/1.

use crate::devices::keyboard::Keyboard;
use crate::devices::mouse::Mouse;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct KeyboardUart {
    keyboard: Option<Keyboard>,
    rx: VecDeque<u8>,
    next_rx: u64,
    divisor: [u8; 2],
    line_control: u8,
    modem_control: u8,
    fifo_enabled: bool,
    tx_end_ns: u64,
    frame_ns: u64,
    mouse: Option<Mouse>,
    ier: u8,
    scratch: u8,
    trigger: u8,
    thre_pending: bool,
    rx_timeout: u64,
}

const KEY_BYTE_NS: u64 = 8_333_334; // 1200 baud, 8N1, rounded up to nanoseconds.
impl KeyboardUart {
    pub(super) fn bus_reset(&mut self, now: u64) {
        let keyboard = self.keyboard.is_some();
        let mouse = self.mouse.is_some();
        *self = Self::default();
        if keyboard {
            self.attach(now);
        }
        if mouse {
            self.attach_mouse();
        }
    }

    pub(super) fn attach(&mut self, now: u64) {
        if self.keyboard.is_none() {
            self.keyboard = Some(Keyboard::default());
            self.next_rx = now.saturating_add(KEY_BYTE_NS);
        }
    }
    pub(super) fn key(&mut self, code: u8, down: bool, now: u64) -> bool {
        let Some(k) = self.keyboard.as_mut() else {
            return false;
        };
        if !k.has_bytes() {
            self.next_rx = now.saturating_add(KEY_BYTE_NS);
        }
        k.key(code, down)
    }

    pub(super) fn attach_mouse(&mut self) {
        self.mouse.get_or_insert_with(Mouse::default);
    }

    pub(super) fn mouse_input(&mut self, now: u64, apply: impl FnOnce(&mut Mouse)) {
        if let Some(mouse) = self.mouse.as_mut() {
            if mouse.idle() {
                self.next_rx = now.saturating_add(KEY_BYTE_NS);
            }
            apply(mouse);
        }
    }

    fn has_input(&self) -> bool {
        self.keyboard.as_ref().is_some_and(Keyboard::has_bytes)
            || self.mouse.as_ref().is_some_and(|m| !m.idle())
    }
    fn advance_rx(&mut self, now: u64) {
        while now >= self.next_rx && self.rx.len() < if self.fifo_enabled { 16 } else { 1 } {
            let byte = self
                .keyboard
                .as_mut()
                .and_then(Keyboard::pop_byte)
                .or_else(|| self.mouse.as_mut().and_then(Mouse::pop_byte));
            let Some(byte) = byte else { break };
            self.rx.push_back(byte);
            self.rx_timeout = self.next_rx.saturating_add(4 * KEY_BYTE_NS);
            self.next_rx = self.next_rx.saturating_add(KEY_BYTE_NS);
        }
    }
    fn cause(&self, now: u64) -> u8 {
        if self.ier & 1 != 0 && !self.rx.is_empty() {
            if !self.fifo_enabled || self.rx.len() >= [1, 4, 8, 14][usize::from(self.trigger)] {
                return 4;
            }
            if now >= self.rx_timeout {
                return 12;
            }
        }
        if self.ier & 2 != 0
            && self.thre_pending
            && self.tx_end_ns.saturating_sub(now) <= self.frame_ns
        {
            return 2;
        }
        1
    }
    pub(super) fn interrupt(&mut self, now: u64) -> bool {
        if self.ier == 0 {
            return false;
        }
        self.advance_rx(now);
        self.cause(now) != 1
    }
    pub(super) fn next_event(&self, now: u64) -> Option<u64> {
        let mut deadline = None;
        if self.ier & 1 != 0 {
            if self.has_input() {
                deadline = Some(self.next_rx.max(now));
            }
            if !self.rx.is_empty() && self.fifo_enabled && self.rx_timeout > now {
                deadline = Some(deadline.map_or(self.rx_timeout, |x| x.min(self.rx_timeout)));
            }
        }
        if self.ier & 2 != 0 && self.thre_pending {
            let thre = self.tx_end_ns.saturating_sub(self.frame_ns);
            if thre > now {
                deadline = Some(deadline.map_or(thre, |x| x.min(thre)));
            }
        }
        deadline
    }
    pub(super) fn read(&mut self, register: u8, now: u64) -> Result<u8, String> {
        self.advance_rx(now);
        match register {
            0 | 1 if self.line_control & 0x80 != 0 => Ok(self.divisor[register as usize]),
            0 => {
                self.rx_timeout = now.saturating_add(4 * KEY_BYTE_NS);
                Ok(self.rx.pop_front().unwrap_or(0))
            }
            1 => Ok(self.ier),
            2 => {
                let cause = self.cause(now);
                if cause == 2 {
                    self.thre_pending = false;
                }
                Ok(if self.fifo_enabled { 0xc0 } else { 0 } | cause)
            }
            6 => Ok(if self.modem_control & 16 != 0 {
                ((self.modem_control & 2) << 3)
                    | ((self.modem_control & 1) << 5)
                    | ((self.modem_control & 12) << 4)
            } else {
                0
            }),
            7 => Ok(self.scratch),
            3 => Ok(self.line_control),
            4 => Ok(self.modem_control),
            5 => Ok(u8::from(!self.rx.is_empty())
                | match self.tx_end_ns.saturating_sub(now) {
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
            0 => {
                if self
                    .keyboard
                    .as_ref()
                    .is_some_and(|k| !k.can_accept_command(value))
                {
                    return Err("Ultra 5 keyboard response queue full".into());
                }
                self.transmit(now, normal_clock)?;
                self.thre_pending = true;
                if let Some(keyboard) = self.keyboard.as_mut() {
                    keyboard.command(value);
                    self.next_rx = self.tx_end_ns.saturating_add(KEY_BYTE_NS);
                }
            }
            1 => {
                if self.ier & 2 == 0 && value & 2 != 0 {
                    self.thre_pending = true;
                }
                self.ier = value & 15;
            }
            2 => {
                self.trigger = value >> 6;
                if value & 2 != 0 || (value & 1 != 0) != self.fifo_enabled {
                    self.rx.clear();
                }
                let enable = value & 1 != 0;
                if (enable != self.fifo_enabled || (enable && value & 4 != 0))
                    && self.tx_end_ns > now
                {
                    // FIFO clears leave the byte already shifting on the wire.
                    self.tx_end_ns = now + (self.tx_end_ns - now - 1) % self.frame_ns + 1;
                }
                self.fifo_enabled = enable;
            }
            3 => self.line_control = value,
            4 => self.modem_control = value & 31,
            7 => self.scratch = value,
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
        if !normal_clock || divisor == 0 {
            return Err(format!("unsupported Ultra 5 UART clock normal={normal_clock} LCR={:#04x} divisor={divisor}",self.line_control));
        }
        // PC87332 §6.3: 24 MHz / 13 / divisor / 16, ten bits per 8N1 frame.
        let data_bits = 5 + u64::from(self.line_control & 3);
        let parity = u64::from(self.line_control & 8 != 0);
        let stop_halves = if self.line_control & 4 == 0 {
            2
        } else if data_bits == 5 {
            3
        } else {
            4
        };
        let half_bits = 2 * (1 + data_bits + parity) + stop_halves;
        let frame = (half_bits * 13 * 16 * divisor * 1_000).div_ceil(48);
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
        // Account for both the byte shifting out and any queued transmitter bytes.
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
            assert_eq!(uart.read(0, end).unwrap(), 0);
        }
    }
}

#[cfg(test)]
mod attached_tests {
    use super::*;
    #[test]
    fn fifo_timeout_and_transmit_interrupts_are_acknowledged_separately() {
        let mut uart = KeyboardUart::default();
        uart.attach(0);
        for t in 1..=3 {
            uart.read(0, t * KEY_BYTE_NS).unwrap();
        }
        uart.write(2, 0xc1, 0, true).unwrap();
        uart.write(1, 3, 0, true).unwrap();
        assert_eq!(uart.read(2, 0).unwrap(), 0xc2);
        assert_eq!(uart.read(2, 0).unwrap(), 0xc1);
        assert!(uart.key(0x30, true, 0));
        assert!(!uart.interrupt(KEY_BYTE_NS));
        assert_eq!(uart.next_event(KEY_BYTE_NS), Some(5 * KEY_BYTE_NS));
        assert!(uart.interrupt(5 * KEY_BYTE_NS));
        assert_eq!(uart.read(2, 5 * KEY_BYTE_NS).unwrap(), 0xcc);
        assert_eq!(uart.read(0, 5 * KEY_BYTE_NS).unwrap(), 0x30);
        assert!(!uart.interrupt(5 * KEY_BYTE_NS));
    }

    #[test]
    fn reset_layout_and_key_bytes_are_serialized_through_the_receiver() {
        let mut u = KeyboardUart::default();
        u.attach(0);
        u.write(3, 0x83, 0, true).unwrap();
        u.write(0, 96, 0, true).unwrap();
        u.write(3, 3, 0, true).unwrap();
        u.write(2, 1, 0, true).unwrap();
        u.write(0, 1, 0, true).unwrap();
        let mut now = u.tx_end_ns;
        for byte in [0xff, 4, 0x7f] {
            assert_eq!(u.read(5, now + KEY_BYTE_NS - 1).unwrap() & 1, 0);
            now += KEY_BYTE_NS;
            assert_eq!(u.read(5, now).unwrap() & 1, 1);
            assert_eq!(u.read(0, now).unwrap(), byte);
        }
        u.write(0, 15, now, true).unwrap();
        now = u.tx_end_ns + 2 * KEY_BYTE_NS;
        assert_eq!(u.read(0, now).unwrap(), 0xfe);
        assert_eq!(u.read(0, now).unwrap(), 0x21);
        now += 1_000_000_000;
        assert!(u.key(0x1e, true, now));
        assert!(u.key(0x1e, false, now));
        assert_eq!(u.read(5, now + KEY_BYTE_NS - 1).unwrap() & 1, 0);
        for byte in [0x1e, 0x9e, 0x7f] {
            now += KEY_BYTE_NS;
            assert_eq!(u.read(0, now).unwrap(), byte);
        }
    }
}
