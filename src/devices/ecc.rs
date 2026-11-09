//! Sun4m SMC ECC memory-controller register subset used by the SS-20/Axil.
//! The controller reports its identity and retains the configuration, fault
//! and event registers. Parity/ECC generation, error injection and the
//! correctable-error interrupt remain unmodeled.

use crate::bus::BusErrorKind;

pub const ECC_SIZE: u64 = 0x40;
const MER: usize = 0;
const MDR: usize = 1;
const MFSR: usize = 2;
const VCR: usize = 3;
const MFAR0: usize = 4;
const MFAR1: usize = 5;
const DR: usize = 6;
const ECR0: usize = 7;
const ECR1: usize = 8;
const SMC_VERSION: u32 = 0x2000_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ecc {
    version: u32,
    regs: [u32; 16],
}

impl Default for Ecc {
    fn default() -> Self {
        Self::new(SMC_VERSION)
    }
}

impl Ecc {
    /// `version` is the implementation/version field of the Memory Enable
    /// Register: EMC (SS-10) 0x1000_0000, SMC (SS-20) 0x2000_0000.
    pub fn new(version: u32) -> Self {
        let mut ecc = Self {
            version,
            regs: [0; 16],
        };
        ecc.reset();
        ecc
    }

    pub fn reset(&mut self) {
        self.regs = [0; 16];
        self.regs[MER] = self.version;
        self.regs[MDR] = 0x20;
        self.regs[MFAR0] = 0x07c0_0000;
    }

    fn index(offset: u64) -> Result<usize, BusErrorKind> {
        if offset % 4 != 0 {
            return Err(BusErrorKind::Misaligned);
        }
        let index = (offset / 4) as usize;
        if index >= self::ECC_SIZE as usize / 4 {
            return Err(BusErrorKind::Unmapped);
        }
        Ok(index)
    }

    pub fn read(&self, offset: u64) -> Result<u32, BusErrorKind> {
        Ok(self.regs[Self::index(offset)?])
    }

    pub fn write(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match Self::index(offset)? {
            MER => self.regs[MER] = self.version | (value & 0x0000_0bff),
            MDR => self.regs[MDR] = value & 0x7fff_ffff,
            MFSR => self.regs[MFSR] = value,
            VCR => self.regs[VCR] = value,
            MFAR0 => self.regs[MFAR0] = value,
            MFAR1 => self.regs[MFAR1] = value,
            DR => self.regs[DR] = value,
            ECR0 => self.regs[ECR0] = value,
            ECR1 => self.regs[ECR1] = value,
            _ => self.regs[Self::index(offset)?] = value,
        }
        Ok(())
    }
}
