//! Sun Type 5 US keyboard on SCC channel A, 1200 baud, 8N1.
use crate::devices::serial::{Channel, Serial};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Keyboard {
    queue: VecDeque<u8>,
    pressed: [bool; 128],
    phase: u64,
    led_command: bool,
    pending_command: Option<u8>,
    pub leds: u8,
    pub bell: bool,
    pub click: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            queue: [0xff, 4, 0x7f].into_iter().collect(),
            pressed: [false; 128],
            phase: 0,
            led_command: false,
            pending_command: None,
            leds: 0,
            bell: false,
            click: false,
        }
    }
}

impl Keyboard {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Sun station code, not ASCII. False indicates invalid code or host
    /// backpressure; no key state changes when an event cannot be queued.
    pub fn key(&mut self, code: u8, down: bool) -> bool {
        if code == 0 || code >= 0x7f {
            return false;
        }
        if self.pressed[code as usize] == down {
            return true;
        }
        let idle = !down
            && self
                .pressed
                .iter()
                .enumerate()
                .all(|(i, pressed)| !pressed || i == usize::from(code));
        if self.queue.len() + 1 + usize::from(idle) > 16 {
            return false;
        }
        self.pressed[code as usize] = down;
        self.queue.push_back(code | if down { 0 } else { 0x80 });
        if idle {
            self.queue.push_back(0x7f);
        }
        true
    }
    fn command(&mut self, value: u8) {
        if self.led_command {
            self.leds = value & 31;
            self.led_command = false;
            return;
        }
        match value {
            1 => {
                let pressed = self.pressed;
                self.reset();
                self.pressed = pressed;
                if let Some(code) = pressed.iter().position(|down| *down) {
                    self.queue[2] = code as u8;
                }
            }
            2 => self.bell = true,
            3 => self.bell = false,
            10 => self.click = true,
            11 => self.click = false,
            14 => self.led_command = true,
            15 => {
                self.queue.extend([0xfe, 0x21]);
            }
            _ => {} // Undefined keyboard commands have no response.
        }
    }
    pub(crate) fn advance_ns(&mut self, ns: u64, serial: &mut Serial) {
        while let Some(command) = self
            .pending_command
            .take()
            .or_else(|| serial.pop_output(Channel::A))
        {
            if !self.led_command && command == 15 && self.queue.len() > 14 {
                self.pending_command = Some(command);
                break;
            }
            self.command(command);
        }
        let phase = u128::from(self.phase) + u128::from(ns) * 3;
        let mut bytes = phase / 25_000_000; // ten bit times at 1200 baud
        self.phase = (phase % 25_000_000) as u64;
        while bytes != 0 {
            let Some(&byte) = self.queue.front() else {
                break;
            };
            if !serial.receive(Channel::A, byte) {
                break;
            }
            self.queue.pop_front();
            bytes -= 1;
        }
    }
}
