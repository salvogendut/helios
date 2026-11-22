//! Diagnostic SRAM on the UltraSPARC-IIi module. Sun manual §§18.5, A.9.

use std::collections::BTreeMap;

const DATA_BASE: u64 = 1 << 39;
const TAG_BASE: u64 = 2 << 39;
const CAPACITY: u32 = 2 << 20;
const LINE_SIZE: u32 = 64;
// Odd parity [17:16], state [15:14], physical tag [11:0].
const TAG_FIELDS: u32 = 0x0003_cfff;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Ecache {
    // Sparse by SRAM byte offset. Unwritten diagnostic cells start at zero.
    data: BTreeMap<u32, u64>,
    tags: BTreeMap<u32, u32>,
    tag_data: u32,
    elim: u8,
}

impl Ecache {
    pub(super) fn upa_config(&self) -> u64 {
        u64::from(self.elim) << 33
    }

    pub(super) fn set_upa_config(&mut self, value: u64) -> Result<(), String> {
        let elim = ((value >> 33) & 7) as u8;
        if matches!(elim, 1 | 2) || value >> 36 != 0 {
            return Err(format!("unsupported IIi UPA_CONFIG {value:#x}"));
        }
        self.elim = elim;
        Ok(())
    }

    pub(super) fn tag_data(&self) -> u64 {
        u64::from(self.tag_data)
    }

    pub(super) fn set_tag_data(&mut self, value: u64) {
        self.tag_data = value as u32 & TAG_FIELDS;
    }

    fn data_offset(address: u64) -> Result<u32, String> {
        let offset = address.wrapping_sub(DATA_BASE);
        if offset > u64::from(CAPACITY) || offset & 7 != 0 {
            return Err(format!("unsupported IIi E-cache data address {address:#x}"));
        }
        // OBP writes once at the installed capacity to test for wrap to cell 0.
        // Other addresses outside the documented data window remain unmapped.
        Ok(offset as u32 & (CAPACITY - 1))
    }

    fn tag_offset(&self, address: u64) -> Result<u32, String> {
        let offset = address.wrapping_sub(TAG_BASE);
        if offset >= u64::from(CAPACITY) || offset & u64::from(LINE_SIZE - 1) != 0 {
            return Err(format!("unsupported IIi E-cache tag address {address:#x}"));
        }
        let capacity = match self.elim {
            4 => 1 << 20,
            5 => 512 << 10,
            6 | 7 => 256 << 10,
            _ => CAPACITY, // POR (0) or 2 MiB (3)
        };
        Ok(offset as u32 & (capacity - 1))
    }

    pub(super) fn write(&mut self, address: u64, value: u64) -> Result<(), String> {
        if address >> 39 == 2 {
            let offset = self.tag_offset(address)?;
            if self.tag_data == 0 {
                self.tags.remove(&offset);
            } else {
                self.tags.insert(offset, self.tag_data);
            }
        } else {
            let offset = Self::data_offset(address)?;
            if value == 0 {
                self.data.remove(&offset);
            } else {
                self.data.insert(offset, value);
            }
        }
        Ok(())
    }

    /// A tag load transfers SRAM to the staging register. Its integer result
    /// is undefined on IIi. Leave the destination register as it was.
    pub(super) fn read(&mut self, address: u64) -> Result<Option<u64>, String> {
        if address >> 39 == 2 {
            let offset = self.tag_offset(address)?;
            self.tag_data = self.tags.get(&offset).copied().unwrap_or(0);
            Ok(None)
        } else {
            let offset = Self::data_offset(address)?;
            Ok(Some(self.data.get(&offset).copied().unwrap_or(0)))
        }
    }
}
