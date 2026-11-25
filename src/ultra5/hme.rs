//! PCIO Happy Meal Ethernet registers and internal Clause 22 PHY.
//! Packet and descriptor transfers are serviced by the board over the PBM IOMMU.
#[derive(Default)]
pub(super) struct Hme {
    registers: std::collections::BTreeMap<u32, u32>,
    pub(super) link: bool,
    pub(super) status: u32,
    // One bits enable delivery internally; the guest register has inverted polarity.
    irq_enable: u32,
    pub(super) tx_index: u32,
    pub(super) rx_index: u32,
    pub(super) assembling: Option<Vec<u8>>,
    pub(super) checksum_offsets: Option<(usize, usize)>,
    pub(super) outgoing: std::collections::VecDeque<Vec<u8>>,
    frame: u32,
    config: u32,
    clock: bool,
    data: bool,
    drive: bool,
    shift: u32,
    bits: u8,
    response: u32,
    remaining: u8,
    mdi: Option<bool>,
    control: u16,
    advertisement: Option<u16>,
}

impl Hme {
    pub(super) fn reg(&self, off: u32) -> u32 {
        self.registers.get(&off).copied().unwrap_or(0)
    }
    pub(super) fn interrupt(&self) -> bool {
        self.status & self.irq_enable != 0
    }
    pub(super) fn carrier(&self) -> bool {
        self.link && self.control & 0x0c00 == 0
    }
    pub(super) fn accepts(&self, frame: &[u8]) -> bool {
        let Some(dst) = frame.get(..6) else {
            return false;
        };
        let cfg = self.reg(0x630c);
        if cfg & 0x40 != 0 || dst == [0xff; 6] {
            return true;
        }
        if dst.chunks_exact(2).enumerate().all(|(i, pair)| {
            self.reg(0x6320 - 4 * i as u32) as u16 == u16::from_be_bytes([pair[0], pair[1]])
        }) {
            return true;
        }
        if dst[0] & 1 == 0 {
            return false;
        }
        let bit = crate::devices::ethernet::crc(dst) >> 26;
        cfg & 0x400 != 0
            || (cfg & 0x800 != 0 && self.reg(0x634c - 4 * (bit >> 4)) & (1 << (bit & 15)) != 0)
    }
    pub fn read(&mut self, off: u32) -> Option<u32> {
        match off {
            0 | 0x2000 | 0x6208 | 0x6308 => Some(0),
            0x100 => Some(std::mem::take(&mut self.status)),
            0x104 => Some(!self.irq_enable),
            0x7018 => Some(
                if self.config & 2 != 0 && self.config >> 10 & 31 == 1 && self.config & 1 == 0 {
                    u32::from(self.phy_read((self.config >> 3) & 31)) << 16
                } else {
                    0
                },
            ),
            0x700c => Some(self.frame),
            _ if Self::writable_mask(off).is_some() => Some(self.reg(off)),
            0x7000 => Some(self.clock.into()),
            0x7004 => Some(self.data.into()),
            0x7008 => Some(self.drive.into()),
            0x7010 => Some(self.config | (u32::from(self.mdi.unwrap_or(true)) << 8)),
            _ => None,
        }
    }

    pub fn write(&mut self, off: u32, value: u32) -> bool {
        match off {
            0 => {
                if value & 3 == 3 {
                    // The shared external interface leaves loopback when OBP
                    // resets both engines between self-test and network use.
                    self.registers.remove(&0x6000);
                }
                if value & 1 != 0 {
                    self.tx_index = 0;
                    self.assembling = None;
                    self.checksum_offsets = None;
                    self.status &= !0x7f40_ff00;
                    self.registers
                        .retain(|&addr, _| !(0x2000..0x4000).contains(&addr));
                }
                if value & 2 != 0 {
                    self.rx_index = 0;
                    self.status &= !0x003f_00ff;
                    self.registers
                        .retain(|&addr, _| !(0x4000..0x6000).contains(&addr));
                }
            }
            0x104 => self.irq_enable = !value,
            0x2008 | 0x4004 => {
                if off == 0x2008 {
                    self.tx_index = 0
                } else {
                    self.rx_index = 0
                }
                self.registers.insert(off, value & !7);
            }
            0x6208 | 0x6308 => {
                if value & 1 != 0 {
                    self.registers.remove(&(off + 4));
                }
            }
            0x700c => {
                let phy = (value >> 23) & 31;
                let reg = (value >> 18) & 31;
                let present = phy == 1 && self.config & 1 == 0;
                match value >> 28 {
                    6 => {
                        let data = if present { self.phy_read(reg) } else { 0xffff };
                        self.frame = value & 0xffff_0000 | (1 << 16) | u32::from(data);
                    }
                    5 => {
                        if present {
                            self.phy_write(reg, value as u16);
                        }
                        self.frame = value | (1 << 16);
                    }
                    _ => return false,
                }
            }
            _ if Self::writable_mask(off).is_some() => {
                self.registers
                    .insert(off, value & Self::writable_mask(off).unwrap());
            }
            0x7000 => {
                let high = value & 1 != 0;
                if high && !self.clock {
                    self.rising_edge();
                } else if !high && self.clock {
                    // Present the PHY bit during MDC low and hold it through
                    // the sampling edge. Re-reading the pin cannot shift it.
                    self.mdi = self
                        .remaining
                        .checked_sub(1)
                        .map(|bit| self.response >> bit & 1 != 0);
                }
                self.clock = high;
            }
            0x7004 => self.data = value & 1 != 0,
            0x7008 => self.drive = value & 1 != 0,
            0x7010 => self.config = value & 0x7cff,
            _ => return false,
        }
        true
    }

    fn writable_mask(off: u32) -> Option<u32> {
        Some(match off {
            4 => 15,
            0x104 => u32::MAX,
            0x2004 => 0x7ff,
            0x2008 | 0x4004 => 0xffff_fff8,
            0x202c => 15,
            0x4000 => 0x7f_0639,
            0x6000 => 0x3ff,
            0x620c | 0x630c => 0xffff,
            0x6210..=0x6250 | 0x6310..=0x635c if off & 3 == 0 => 0xffff,
            0x7014 => 0xffff,
            _ => return None,
        })
    }

    fn phy_read(&self, reg: u32) -> u16 {
        match reg {
            0 => self.control,
            1 => {
                0x7809
                    | if self.carrier() {
                        4 | (self.control & 0x1000) >> 7
                    } else {
                        0
                    }
            }
            2 => 0x2000, // National Semiconductor DP83840
            3 => 0x5c00,
            5 if self.carrier() => 0x41e1,
            6 if self.carrier() => 1,
            0x19 => 1 | if self.carrier() { 0x20 | 0x80 } else { 0 },
            4 => self.advertisement.unwrap_or(0x01e1),
            _ => 0,
        }
    }

    fn phy_write(&mut self, reg: u32, value: u16) {
        match reg {
            0 => {
                self.control = if value & 0x8000 == 0 {
                    value & 0x7d80
                } else {
                    0
                }
            }
            4 => self.advertisement = Some(value),
            _ => {}
        }
    }

    fn rising_edge(&mut self) {
        // Turnaround and data occupy 18 clocks after REGAD even if the
        // station keeps output-enable asserted during the turnaround.
        if self.remaining != 0 {
            self.remaining -= 1;
            return;
        }
        if !self.drive {
            self.bits = 0;
            return;
        }
        if self.bits == 0 && self.data {
            return; // idle/preamble
        }
        self.shift = (self.shift << 1) | u32::from(self.data);
        self.bits += 1;
        if self.bits == 14 && self.shift & 0x3c00 == 0x1800 {
            let addr = (self.shift >> 5) & 31;
            let reg = self.shift & 31;
            self.response = if addr == 1 && self.config & 1 == 0 {
                (1 << 17) | u32::from(self.phy_read(reg))
            } else {
                0x3ffff
            };
            self.remaining = 18; // Z, driven turnaround zero, sixteen data bits
            self.bits = 0;
        }
        if self.bits == 32 {
            if self.shift >> 28 == 5 && self.shift >> 23 & 31 == 1 && self.config & 1 == 0 {
                self.phy_write(self.shift >> 18 & 31, self.shift as u16);
            }
            self.bits = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Hme;
    fn send_bits(mif: &mut Hme, value: u32, count: u8) {
        for bit in (0..count).rev() {
            mif.write(0x7004, (value >> bit) & 1);
            mif.write(0x7000, 0);
            mif.write(0x7000, 1);
        }
    }
    fn read_header(mif: &mut Hme, addr: u8, reg: u8) {
        mif.write(0x7008, 1);
        send_bits(mif, 0x1800 | (u32::from(addr) << 5) | u32::from(reg), 14);
    }
    fn read_phy(mif: &mut Hme, addr: u8, reg: u8) -> u32 {
        read_header(mif, addr, reg);
        mif.write(0x7008, 0);
        let mut data = 0;
        for _ in 0..18 {
            mif.write(0x7000, 0);
            mif.write(0x7000, 1);
            data = (data << 1) | (mif.read(0x7010).unwrap() >> 8 & 1);
        }
        data
    }

    fn obp_read(mif: &mut Hme, addr: u8, reg: u8) -> u16 {
        read_header(mif, addr, reg);
        // The original ROM clocks both turnaround bits with output enabled.
        for _ in 0..2 {
            mif.write(0x7000, 0);
            mif.write(0x7000, 1);
        }
        mif.write(0x7008, 0);
        let mut word = 0;
        for _ in 0..16 {
            mif.write(0x7000, 0);
            let pin = mif.read(0x7010).unwrap() & 0x100;
            assert_eq!(mif.read(0x7010).unwrap() & 0x100, pin);
            word = (word << 1) | u16::from(pin != 0);
            mif.write(0x7000, 1);
        }
        mif.write(0x7000, 0);
        assert_ne!(mif.read(0x7010).unwrap() & 0x100, 0); // idle pull-up
        word
    }

    #[test]
    fn obp_bitbang_reads_phy_ids_and_reset_without_turnaround_bits_in_data() {
        let mut mif = Hme::default();
        mif.write(0x7010, 4);
        assert_eq!(obp_read(&mut mif, 1, 2), 0x2000);
        assert_eq!(obp_read(&mut mif, 1, 3), 0x5c00);
        assert_eq!(obp_read(&mut mif, 2, 2), 0xffff);
        mif.write(0x7008, 1);
        send_bits(&mut mif, 0x5082_1400, 32); // write BMCR, PHY 1
        assert_eq!(obp_read(&mut mif, 1, 0), 0x1400);
        mif.write(0x7008, 1);
        send_bits(&mut mif, 0x5082_8000, 32);
        assert_eq!(obp_read(&mut mif, 1, 0), 0);
        assert_eq!(obp_read(&mut mif, 1, 4), 0x01e1); // includes a set low bit
    }

    #[test]
    fn combined_engine_reset_clears_shared_xif_loopback() {
        let mut h = Hme::default();
        h.write(0x6000, 3);
        h.write(0, 1);
        assert_eq!(h.read(0x6000), Some(3));
        h.write(0, 2);
        assert_eq!(h.read(0x6000), Some(3));
        h.write(0, 3);
        assert_eq!(h.read(0x6000), Some(0));
    }

    #[test]
    fn mii_frame_and_bitbang_share_phy_state_and_dma_resets_are_independent() {
        let mut h = Hme::default();
        // Clause 22 read opcode, PHY 1, status register 1.
        assert!(h.write(0x700c, 0x6084_0000));
        assert_eq!(h.read(0x700c), Some(0x6085_7809));
        h.write(0x700c, 0x5090_00a1);
        assert_eq!(read_phy(&mut h, 1, 4), 0x200a1);
        h.write(0x2008, 0x1238);
        h.write(0x4004, 0x4560);
        h.write(0, 2);
        assert_eq!(
            (h.read(0), h.read(0x2008), h.read(0x4004)),
            (Some(0), Some(0x1238), Some(0))
        );
        h.write(0, 1);
        assert_eq!(h.read(0x2008), Some(0));
    }
    #[test]
    fn multicast_hash_selects_the_programmed_group() {
        let mut h = Hme::default();
        let multicast = [1, 0, 0x5e, 0, 0, 1];
        h.write(0x630c, 0x801);
        assert!(!h.accepts(&multicast));
        // Reflected CRC-32 of 01:00:5e:00:00:01 before final XOR selects bit 54.
        h.write(0x6340, 1 << 6);
        assert!(h.accepts(&multicast));
        h.write(0x630c, 1);
        assert!(!h.accepts(&multicast));
    }

    #[test]
    fn linked_phy_reports_negotiation_and_isolation_in_both_mif_modes() {
        let mut h = Hme {
            link: true,
            ..Hme::default()
        };
        h.write(0x700c, 0x5080_1200); // enable and restart autonegotiation
        assert_eq!(read_phy(&mut h, 1, 1), 0x2782d);
        h.write(0x7010, (1 << 10) | (1 << 3) | 2); // poll PHY 1, BMSR
        assert_eq!(h.read(0x7018), Some(0x782d_0000));
        h.write(0x700c, 0x5080_1400); // isolate the PHY
        assert_eq!(h.read(0x7018), Some(0x7809_0000));
        assert!(!h.carrier());
    }

    #[test]
    fn onboard_mdio_turnaround_and_unplugged_link() {
        let mut mif = Hme::default();
        mif.write(0x7010, 4);
        assert_eq!(read_phy(&mut mif, 1, 1), 0x27809);
        assert_eq!(read_phy(&mut mif, 2, 1), 0x3ffff);
    }
}
