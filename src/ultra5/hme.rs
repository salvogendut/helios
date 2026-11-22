//! MIF management pins of the PCIO Happy Meal Ethernet function.
//! The internal PHY is on MDIO0 at Clause 22 address 1, with no link partner.
#[derive(Default)]
pub(super) struct Mif {
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

impl Mif {
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            0x7000 => Some(self.clock.into()),
            0x7004 => Some(self.data.into()),
            0x7008 => Some(self.drive.into()),
            0x7010 => Some(self.config | (u32::from(self.mdi.unwrap_or(true)) << 8)),
            _ => None,
        }
    }

    pub fn write(&mut self, off: u32, value: u32) -> bool {
        match off {
            0x7000 => {
                let high = value & 1 != 0;
                if high && !self.clock {
                    self.rising_edge();
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

    fn rising_edge(&mut self) {
        if !self.drive {
            self.mdi = if self.remaining > 0 {
                self.remaining -= 1;
                Some((self.response >> self.remaining) & 1 != 0)
            } else {
                None
            };
            self.bits = 0;
            return;
        }
        self.mdi = None;
        if self.bits == 0 && self.data {
            return; // idle/preamble
        }
        self.shift = (self.shift << 1) | u32::from(self.data);
        self.bits += 1;
        if self.bits == 14 && self.shift & 0x3c00 == 0x1800 {
            let addr = (self.shift >> 5) & 31;
            let reg = self.shift & 31;
            self.response = if addr == 1 && self.config & 1 == 0 {
                (1 << 17)
                    | u32::from(match reg {
                        0 => self.control,
                        1 => 0x7809, // 10/100, autonegotiation capable, link down
                        4 => self.advertisement.unwrap_or(0x01e1),
                        _ => 0,
                    })
            } else {
                0x3ffff
            };
            self.remaining = 18; // Z, driven turnaround zero, sixteen data bits
        }
        if self.bits == 32 {
            if self.shift >> 28 == 5 && self.shift >> 23 & 31 == 1 && self.config & 1 == 0 {
                match self.shift >> 18 & 31 {
                    0 => {
                        self.control = if self.shift & 0x8000 == 0 {
                            self.shift as u16 & 0x7d80
                        } else {
                            0
                        }
                    }
                    4 => self.advertisement = Some(self.shift as u16),
                    _ => {}
                }
            }
            self.bits = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Mif;
    fn read_phy(mif: &mut Mif, addr: u8, reg: u8) -> u32 {
        mif.write(0x7008, 1);
        let header = 0x1800 | (u32::from(addr) << 5) | u32::from(reg);
        for bit in (0..14).rev() {
            mif.write(0x7004, (header >> bit) & 1);
            mif.write(0x7000, 0);
            mif.write(0x7000, 1);
        }
        mif.write(0x7008, 0);
        let mut data = 0;
        for _ in 0..18 {
            mif.write(0x7000, 0);
            mif.write(0x7000, 1);
            data = (data << 1) | (mif.read(0x7010).unwrap() >> 8 & 1);
        }
        data
    }
    #[test]
    fn onboard_mdio_turnaround_and_unplugged_link() {
        let mut mif = Mif::default();
        mif.write(0x7010, 4);
        assert_eq!(read_phy(&mut mif, 1, 1), 0x27809);
        assert_eq!(read_phy(&mut mif, 2, 1), 0x3ffff);
    }
}
