//! Type-0 PCI configuration on the Ultra 5 onboard bus. Identities, grant/latency
//! hints and aperture sizes follow the 440 MHz machine's OpenBoot device tree.
pub(super) const BAR1: u32 = 0xf100_0000;

pub(super) struct PciFunction {
    header: [u8; 128],
    bar_masks: [u32; 6],
    command_mask: u16,
}

impl PciFunction {
    fn new(id: u32, class_rev: u32, command_mask: u16, pin: u8, grant: u8, latency: u8) -> Self {
        let mut header = [0; 128];
        header[0..4].copy_from_slice(&id.to_le_bytes());
        header[6..8].copy_from_slice(&0x0280u16.to_le_bytes());
        header[8..12].copy_from_slice(&class_rev.to_le_bytes());
        header[0x3d..0x40].copy_from_slice(&[pin, grant, latency]);
        Self {
            header,
            bar_masks: [0; 6],
            command_mask,
        }
    }

    fn bar(&mut self, index: usize, value: u32, mask: u32) {
        let at = 0x10 + 4 * index;
        self.header[at..at + 4].copy_from_slice(&value.to_le_bytes());
        self.bar_masks[index] = mask;
    }

    pub(super) fn onboard() -> [Self; 4] {
        let mut ebus = Self::new(0x1000_108e, 0x0680_0001, 0x146, 0, 10, 25);
        ebus.header[4] = 6;
        ebus.header[14] = 0x80;
        ebus.bar(0, 0xf000_0000, 0xff00_0000);
        ebus.bar(1, BAR1, 0xff80_0000);
        let mut hme = Self::new(0x1001_108e, 0x0200_0001, 0x146, 1, 10, 5);
        hme.bar(0, 0, 0xffff_8000);
        let mut ati = Self::new(0x4754_1002, 0x0300_009a, 0x147, 1, 8, 0);
        ati.bar(0, 0, 0xff00_0000);
        let mut ide = Self::new(0x0646_1095, 0x0101_8a03, 0x145, 1, 2, 4);
        ide.header[0x51] = 8; // secondary channel enabled
        ide.header[0x2c..0x30].copy_from_slice(&0x0646_1095u32.to_le_bytes());
        for (i, size) in [8, 4, 8, 4, 16].into_iter().enumerate() {
            ide.bar(i, 1, 0u32.wrapping_sub(size));
        }
        [ebus, hme, ati, ide]
    }

    pub(super) fn bar_address(&self, index: usize) -> u32 {
        let at = 0x10 + 4 * index;
        u32::from_le_bytes(self.header[at..at + 4].try_into().unwrap()) & self.bar_masks[index]
    }

    pub(super) fn claims_memory(&self, address: u32) -> bool {
        self.memory_enabled()
            && self.bar_masks.iter().enumerate().any(|(i, &mask)| {
                mask != 0
                    && self.header[0x10 + 4 * i] & 1 == 0
                    && address & mask == self.bar_address(i)
            })
    }

    pub(super) fn ata_port(&self, addr: u32) -> Option<(usize, u8)> {
        if self.header[4] & 1 == 0 {
            return None;
        }
        for channel in 0..2 {
            if channel == 1 && self.header[0x51] & 8 == 0 {
                continue;
            }
            let native = self.header[9] & (1 << (channel * 2)) != 0;
            let (cmd, ctrl) = if native {
                (
                    self.bar_address(channel * 2),
                    self.bar_address(channel * 2 + 1) + 2,
                )
            } else if channel == 0 {
                (0x1f0, 0x3f6)
            } else {
                (0x170, 0x376)
            };
            if let Some(offset) = addr.checked_sub(cmd).filter(|&offset| offset < 8) {
                return Some((channel, offset as u8));
            }
            if addr == ctrl {
                return Some((channel, 8));
            }
        }
        None
    }

    // CMD646 can report a channel request to software while its PCI INTA is
    // masked. BMIDESR latches rising edges until the driver writes one to bit 2.
    pub(super) fn ide_interrupts(&mut self, primary: bool, secondary: bool) -> bool {
        let lines = (u8::from(primary) << 2) | (u8::from(secondary) << 3);
        let rising = lines & !self.header[0x71];
        self.header[0x71] = (self.header[0x71] & !0x0c) | lines;
        self.header[0x50] |= rising & 4;
        self.header[0x57] |= (rising & 8) << 1;
        self.header[0x72] |= rising & 4;
        self.header[0x7a] |= (rising & 8) >> 1;
        lines & !(self.header[0x71] >> 2) & 0xc != 0
    }

    pub(super) fn bus_master_offset(&self, addr: u32) -> Option<usize> {
        if self.header[4] & 1 == 0 {
            return None;
        }
        addr.checked_sub(self.bar_address(4))
            .filter(|off| *off < 16)
            .map(|off| 0x70 + off as usize)
    }

    pub(super) fn dma_command(&self) -> Option<(bool, u32)> {
        (self.header[4] & 4 != 0 && self.header[0x70] & 1 != 0).then(|| {
            (
                self.header[0x70] & 8 != 0,
                u32::from_le_bytes(self.header[0x74..0x78].try_into().unwrap()),
            )
        })
    }
    pub(super) fn dma_finished(&mut self, failed: bool) {
        self.header[0x72] &= !1;
        self.header[0x72] |= 4 | (u8::from(failed) << 1);
    }
    pub(super) fn memory_enabled(&self) -> bool {
        self.header[4] & 2 != 0
    }

    pub(super) fn read<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        if !matches!(N, 1 | 2 | 4 | 8) || at % N != 0 {
            return None;
        }
        if at + N > 64
            && !(self.header[2..4] == [0x46, 0x06]
                && (at..at + N).all(|i| matches!(i,0x50..=0x5b | 0x70..=0x7f)))
        {
            return None;
        }
        self.header.get(at..at + N)?.try_into().ok()
    }

    pub(super) fn write<const N: usize>(&mut self, at: usize, bytes: [u8; N]) -> bool {
        if self.read::<N>(at).is_none() {
            return false;
        }
        for (reg, b) in (at..).zip(bytes) {
            match reg {
                4 | 5 => self.header[reg] = b & self.command_mask.to_le_bytes()[reg - 4],
                7 => self.header[reg] &= !(b & 0xf9),
                9 if self.header[2..4] == [0x46, 0x06] => {
                    self.header[reg] = (self.header[reg] & !5) | (b & 5)
                }
                0xc | 0xd | 0x3c => self.header[reg] = b,
                0x10..=0x27 => {
                    let mask = self.bar_masks[(reg - 0x10) / 4].to_le_bytes()[reg % 4];
                    self.header[reg] = (self.header[reg] & !mask) | (b & mask);
                }
                0x51..=0x56 | 0x58..=0x5b | 0x73 | 0x7b => self.header[reg] = b,
                0x50 => self.header[reg] &= !(b & 4),
                0x57 => self.header[reg] = (self.header[reg] & 0x10 & !b) | (b & !0x10),
                0x70 | 0x78 => self.header[reg] = b & 9, // direction and start
                0x72 | 0x7a => self.header[reg] = (self.header[reg] & 6 & !b) | (b & 0x60),
                0x74 | 0x7c => self.header[reg] = b & !3,
                0x75..=0x77 | 0x7d..=0x7f => self.header[reg] = b,
                0x71 => self.header[reg] = (self.header[reg] & 0x0c) | (b & 0x33),
                _ => {}
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ide_channel_events_survive_task_file_ack_and_have_separate_clear_bits() {
        let mut f = PciFunction::onboard().into_iter().last().unwrap();
        f.write(0x71, [0x30]);
        assert!(!f.ide_interrupts(true, false));
        assert!(!f.ide_interrupts(false, false));
        assert_eq!(f.read(0x50), Some([4]));
        assert_eq!(f.read(0x72), Some([4]));
        f.write(0x50, [4]);
        assert_eq!(f.read(0x50), Some([0]));
        assert_eq!(f.read(0x72), Some([4]));
        f.write(0x72, [4]);
        assert_eq!(f.read(0x72), Some([0]));
        f.write(0x71, [0]);
        assert!(f.ide_interrupts(false, true));
        f.ide_interrupts(false, false);
        assert_eq!(f.read(0x57), Some([0x10]));
        f.write(0x57, [0x94]);
        assert_eq!(f.read(0x57), Some([0x84]));
        assert_eq!(f.read(0x7a), Some([4]));
    }
}
