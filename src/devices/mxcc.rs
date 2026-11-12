//! SuperSPARC MXCC register and stream-transfer subset.
//! Register fields follow Sun-4M System Architecture B.III.5.4.
use crate::bus::{Bus, BusError, BusErrorKind};

/// Physical CPU-0 module-control alias for a populated SuperSPARC module.
pub const MID8_PHYSICAL_BASE: u64 = 0x000f_f8c0_0000;
pub const MODULE_REGISTER_SIZE: u64 = 0x1000;
pub const MID8_PORT_ADDRESS: u64 = 0x000f_f8ff_fffc;
pub const PORT_IDENTITY: u64 = 0x0800_0104;
const CONTROL_MASK: u64 = 0x0000_0000_0000_023c; // RC, PF, MC, PE, CE
const STREAM_ADDRESS_MASK: u64 = 0x0000_001f_ffff_ffe0;
const STREAM_PA_MASK: u64 = 0x0000_000f_ffff_ffe0;
const READY: u64 = 1 << 63;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MxccState {
    data: [u64; 4],
    registers: [u64; 8],
}

impl MxccState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Read one full MXCC register. Unknown/reserved addresses read zero.
    pub fn read_u64(&self, address: u32) -> u64 {
        match address {
            0x01c0_0000 => self.data[0],
            0x01c0_0008 => self.data[1],
            0x01c0_0010 => self.data[2],
            0x01c0_0018 => self.data[3],
            0x01c0_0100 => self.registers[0],
            0x01c0_0200 => self.registers[1],
            0x01c0_0a00 => self.registers[3],
            0x01c0_0c00 => self.registers[5],
            0x01c0_0f00 => PORT_IDENTITY,
            _ => 0,
        }
    }

    /// Read a word from a 64-bit MXCC register. Bit 2 selects the low word.
    pub fn read_word(&self, address: u32) -> u32 {
        let register = address & !7;
        let value = self.read_u64(register);
        if address & 4 == 0 {
            (value >> 32) as u32
        } else {
            value as u32
        }
    }

    /// Write a full MXCC register, performing source/destination stream
    /// transfers synchronously. RDY is accepted only from hardware completion.
    pub fn write_u64(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        value: u64,
    ) -> Result<(), BusError> {
        match address {
            0x01c0_0000 => self.data[0] = value,
            0x01c0_0008 => self.data[1] = value,
            0x01c0_0010 => self.data[2] = value,
            0x01c0_0018 => self.data[3] = value,
            0x01c0_0100 => {
                self.registers[0] = value & STREAM_ADDRESS_MASK;
                let source = value & STREAM_PA_MASK;
                if Self::is_register_window(source) {
                    return Err(BusError {
                        address: source,
                        kind: BusErrorKind::Unsupported("MXCC stream into its own register window"),
                    });
                }
                let words = self.read_stream(bus, source)?;
                self.data = words;
                self.registers[0] |= READY;
            }
            0x01c0_0200 => {
                self.registers[1] = value & STREAM_ADDRESS_MASK;
                let destination = value & STREAM_PA_MASK;
                if Self::is_register_window(destination) {
                    return Err(BusError {
                        address: destination,
                        kind: BusErrorKind::Unsupported("MXCC stream into its own register window"),
                    });
                }
                self.write_stream(bus, destination)?;
                self.registers[1] |= READY;
            }
            0x01c0_0a00 => {
                self.registers[3] = (self.registers[3] & !CONTROL_MASK) | (value & CONTROL_MASK);
            }
            0x01c0_0c00 => self.registers[5] = value,
            0x01c0_0e00 => self.registers[6] &= !value,
            0x01c0_0f00 => {} // MID/device/revision/vendor are read-only.
            _ => {}
        }
        Ok(())
    }

    /// Write one word of a 64-bit MXCC register. Bits 63:32 are preserved for
    /// low-word writes and bits 31:0 for high-word writes.
    pub fn write_word(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        value: u32,
    ) -> Result<(), BusError> {
        let register = address & !7;
        let old = self.read_u64(register);
        let merged = if address & 4 == 0 {
            (old & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32)
        } else {
            (old & 0xffff_ffff_0000_0000) | u64::from(value)
        };
        self.write_u64(bus, register, merged)
    }

    /// Decode a CPU-module physical register address for the populated MID 8.
    pub fn physical_register(address: u64) -> Option<u32> {
        if address == MID8_PORT_ADDRESS {
            return Some(0x01c0_0f04);
        }
        if !(MID8_PHYSICAL_BASE..MID8_PHYSICAL_BASE + MODULE_REGISTER_SIZE).contains(&address) {
            return None;
        }
        Some(0x01c0_0000 | (address - MID8_PHYSICAL_BASE) as u32)
    }

    fn is_register_window(address: u64) -> bool {
        address == MID8_PORT_ADDRESS
            || (MID8_PHYSICAL_BASE..MID8_PHYSICAL_BASE + MODULE_REGISTER_SIZE).contains(&address)
    }

    fn read_stream(&self, bus: &mut impl Bus, address: u64) -> Result<[u64; 4], BusError> {
        let mut data = [0u64; 4];
        for (index, word) in data.iter_mut().enumerate() {
            let pair = bus.read_doubleword(address + 8 * index as u64)?;
            *word = (u64::from(pair[0]) << 32) | u64::from(pair[1]);
        }
        Ok(data)
    }

    fn write_stream(&self, bus: &mut impl Bus, address: u64) -> Result<(), BusError> {
        for (index, word) in self.data.iter().enumerate() {
            bus.write_doubleword(
                address + 8 * index as u64,
                [(word >> 32) as u32, *word as u32],
            )?;
        }
        Ok(())
    }
}
