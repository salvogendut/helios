//! Sun Mouse Systems five-byte protocol on SCC channel B, 1200 baud.
use crate::devices::serial::{Channel, Serial};
use std::collections::VecDeque;

const BYTE_NS: u64 = 1_000_000_000 / 120;

#[derive(Clone, Debug, Default)]
pub struct Mouse {
    buttons: u8,
    dx: i32,
    dy: i32,
    changed: bool,
    queue: VecDeque<u8>,
    phase: u64,
}

impl Mouse {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Button index 0/1/2 is left/middle/right.
    pub fn button(&mut self, button: u8, down: bool) {
        if button > 2 {
            return;
        }
        let mask = 1 << (2 - button);
        let old = self.buttons;
        self.buttons = if down {
            self.buttons | mask
        } else {
            self.buttons & !mask
        };
        if self.buttons != old {
            self.changed = true;
            self.schedule();
        }
    }

    /// Relative host motion; positive X is right and positive Y is down.
    pub fn motion(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        self.dx = self.dx.saturating_add(dx);
        self.dy = self.dy.saturating_add(dy);
        self.schedule();
    }

    pub(crate) fn advance_ns(&mut self, ns: u64, serial: &mut Serial) {
        self.schedule();
        let phase = u128::from(self.phase) + u128::from(ns) * 1_000_000_000;
        let mut bytes = phase / u128::from(BYTE_NS);
        self.phase = (phase % u128::from(BYTE_NS)) as u64;
        while bytes != 0 {
            if self.queue.is_empty() {
                self.schedule();
                if self.queue.is_empty() {
                    break;
                }
            }
            let byte = self.queue[0];
            if !serial.receive(Channel::B, byte) {
                break;
            }
            self.queue.pop_front();
            bytes -= 1;
        }
    }

    fn schedule(&mut self) {
        while self.queue.len() + 5 <= 32 && (self.dx != 0 || self.dy != 0 || self.changed) {
            let dx = self.dx.clamp(-128, 127);
            let dy = self.dy.clamp(-128, 127);
            self.dx -= dx;
            self.dy -= dy;
            self.changed = false;
            self.queue.push_back(0x80 | ((!self.buttons) & 7));
            self.queue.push_back(dx as i8 as u8);
            self.queue.push_back(dy as i8 as u8);
            self.queue.push_back(0);
            self.queue.push_back(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_byte_packets_encode_buttons_and_signed_deltas() {
        let mut mouse = Mouse::default();
        mouse.motion(5, -2);
        assert_eq!(
            mouse.queue.iter().copied().collect::<Vec<_>>(),
            [0x87, 5, 0xfe, 0, 0]
        );
        mouse.queue.clear();
        mouse.button(0, true);
        assert_eq!(
            mouse.queue.iter().copied().collect::<Vec<_>>(),
            [0x83, 0, 0, 0, 0]
        );
        mouse.queue.clear();
        mouse.button(1, true);
        mouse.button(2, true);
        mouse.button(0, false);
        assert_eq!(
            mouse.queue.iter().copied().collect::<Vec<_>>(),
            [0x81, 0, 0, 0, 0, 0x80, 0, 0, 0, 0, 0x84, 0, 0, 0, 0]
        );
    }

    #[test]
    fn serial_channel_b_receives_five_byte_packets_at_the_slow_byte_rate() {
        let mut serial = Serial::default();
        for (register, value) in [(4, 0x44), (14, 3), (3, 0xc1), (5, 0xea)] {
            serial.write(0, register).unwrap();
            serial.write(0, value).unwrap();
        }
        let mut mouse = Mouse::default();
        mouse.motion(3, -4);
        for _ in 0..5 {
            mouse.advance_ns(super::BYTE_NS, &mut serial);
        }
        let bytes: Vec<_> = (0..5).map(|_| serial.read(2).unwrap()).collect();
        assert_eq!(bytes, [0x87, 3, 0xfc, 0, 0]);
    }

    #[test]
    fn large_motion_is_split_without_losing_the_remainder() {
        let mut mouse = Mouse::default();
        mouse.motion(300, -130);
        let packets = mouse.queue.len() / 5;
        assert_eq!(packets, 3);
        let total_dx: i32 = (0..packets)
            .map(|i| mouse.queue[i * 5 + 1] as i8 as i32)
            .sum();
        let total_dy: i32 = (0..packets)
            .map(|i| mouse.queue[i * 5 + 2] as i8 as i32)
            .sum();
        assert_eq!((total_dx, total_dy), (300, -130));
    }
}
