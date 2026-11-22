//! UltraSPARC-IIi directed I/D TLB accesses. Sun manual §§15.2, 15.9.8–9.

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entry {
    tag: u64,
    data: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tlb {
    tag_access: u64,
    entries: [Entry; 64],
}

impl Default for Tlb {
    fn default() -> Self {
        Self {
            tag_access: 0,
            entries: [Entry::default(); 64],
        }
    }
}

impl Tlb {
    pub(super) fn tag_access(&self) -> u64 {
        self.tag_access
    }

    pub(super) fn stage_tag(&mut self, tag: u64) {
        // Tag Access takes VA[43:13] and context[12:0], ignoring VA[63:44].
        self.tag_access = (((tag << 20) as i64) >> 20) as u64;
    }

    pub(super) fn index(address: u64) -> Result<usize, String> {
        if address & !0x1f8 != 0 {
            return Err(format!("unsupported IIi indexed TLB address {address:#x}"));
        }
        Ok((address >> 3) as usize)
    }

    pub(super) fn read_tag(&self, index: usize) -> u64 {
        self.entries[index].tag
    }

    pub(super) fn read_data(&self, index: usize) -> u64 {
        self.entries[index].data
    }

    pub(super) fn write_data(&mut self, index: usize, value: u64, instruction: bool) {
        // Retain V/size/NFO/IE, Used, PA[40:13] and L/CP/CV/E/P/W/G.
        // Soft fields read zero. Diagnostic CAM/RAM size fields are read-only
        // thermometer codes derived from Size (000, 001, 011 or 111).
        let size = (value >> 61) & 3;
        let size_bits = (1u64 << size) - 1;
        let mut data = value & (0xf800_0000_0000_0000 | 0x0000_03ff_ffff_e000 | 0x7f);
        data |= (size_bits << 42) | (size_bits << 45);
        if instruction {
            data &= !((1 << 60) | (1 << 59) | 0x1a); // NFO, IE, CV, E, W
        }
        self.entries[index] = Entry {
            tag: self.tag_access,
            data,
        };
    }
}
