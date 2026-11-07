//! Crystal CS4231 register subset at the SS5 onboard audio aperture.
//! This is a boot compatibility surface; sample conversion, DMA and IRQs are
//! not modeled.

use crate::bus::BusErrorKind;

const REGS: usize = 16;
const DREGS: usize = 32;
const REG_SIZE: u64 = 0x40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cs4231 {
    regs: [u32; REGS],
    dregs: [u8; DREGS],
}

impl Default for Cs4231 {
    fn default() -> Self {
        let mut cs = Self {
            regs: [0; REGS],
            dregs: [0; DREGS],
        };
        cs.reset();
        cs
    }
}

impl Cs4231 {
    pub fn reset(&mut self) {
        self.regs = [0; REGS];
        self.dregs = [0; DREGS];
        self.dregs[12] = 0x8a;
        self.dregs[25] = 0xa0;
    }

    pub fn read(&self, offset: u64) -> Result<u32, BusErrorKind> {
        if offset >= REG_SIZE || offset % 4 != 0 {
            return Err(BusErrorKind::Unsupported("CS4231 register lane"));
        }
        let index = (offset / 4) as usize;
        Ok(if index == 1 {
            let indirect = (self.regs[0] & 0x1f) as usize;
            if indirect == 3 {
                0
            } else {
                u32::from(self.dregs[indirect])
            }
        } else {
            self.regs[index]
        })
    }

    pub fn read_byte(&self, offset: u64) -> Result<u8, BusErrorKind> {
        if offset >= REG_SIZE {
            return Err(BusErrorKind::Unsupported("CS4231 register lane"));
        }
        Ok(self.read(offset & !3)? as u8)
    }

    pub fn read_half(&self, offset: u64) -> Result<u16, BusErrorKind> {
        if offset >= REG_SIZE {
            return Err(BusErrorKind::Unsupported("CS4231 register lane"));
        }
        Ok(self.read(offset & !3)? as u16)
    }

    pub fn write_byte(&mut self, offset: u64, value: u8) -> Result<(), BusErrorKind> {
        if offset >= REG_SIZE {
            return Err(BusErrorKind::Unsupported("CS4231 register lane"));
        }
        self.write(offset & !3, u32::from(value))
    }

    pub fn write_half(&mut self, offset: u64, value: u16) -> Result<(), BusErrorKind> {
        if offset >= REG_SIZE {
            return Err(BusErrorKind::Unsupported("CS4231 register lane"));
        }
        self.write(offset & !3, u32::from(value))
    }

    pub fn write(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        if offset >= REG_SIZE || offset % 4 != 0 {
            return Err(BusErrorKind::Unsupported("CS4231 register lane"));
        }
        let index = (offset / 4) as usize;
        if index == 1 {
            let indirect = (self.regs[0] & 0x1f) as usize;
            match indirect {
                11 | 25 => {}
                12 => self.dregs[12] = 0x8a | (value as u8 & 0x40),
                _ => self.dregs[indirect] = value as u8,
            }
        } else if index == 2 {
        } else if index == 4 {
            if value & 1 != 0 {
                self.reset();
            }
            self.regs[4] = value & 0x7f;
        } else {
            self.regs[index] = value;
        }
        Ok(())
    }
}
