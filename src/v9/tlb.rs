//! UltraSPARC-IIi I/D TLB translation and directed accesses. Sun manual chapter 15.

#[cfg(test)]
mod miss_tests;
#[cfg(test)]
mod tests;

use super::{CpuFault, TlbMiss, PHYSICAL_ADDRESS_MASK};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    Fetch,
    Read,
    Write,
    Flush,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Translation {
    pub(crate) pa: u64,
    pub(crate) cacheable: bool,
    pub(super) invert_endian: bool,
    pub(super) slot: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entry {
    tag: u64,
    data: u64,
}

impl Entry {
    fn matches_page(self, address: u64, context: u16) -> bool {
        self.data >> 63 != 0
            && (self.data & 1 != 0 || self.tag & 0x1fff == u64::from(context))
            && (self.tag ^ address) & !self.page_mask() == 0
    }

    fn page_mask(self) -> u64 {
        (8192u64 << (3 * ((self.data >> 61) & 3))) - 1
    }
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
    pub(super) fn lookup(
        &self,
        va: u64,
        context: u16,
        privileged: bool,
        access: Access,
    ) -> Result<Translation, CpuFault> {
        let mut hit = None;
        for (slot, entry) in self.entries.iter().enumerate() {
            if !entry.matches_page(va, context) {
                continue;
            }
            if hit.is_some() {
                return Err(format!("multiple IIi TLB matches for {access:?} VA={va:#x}").into());
            }
            hit = Some((slot, entry.data, entry.page_mask()));
        }
        let (slot, data, page_mask) = hit.ok_or(CpuFault::TlbMiss(TlbMiss {
            access,
            address: va,
            context,
        }))?;
        if !privileged && data & 4 != 0 {
            return Err(format!("IIi privileged page for {access:?} VA={va:#x}").into());
        }
        if access == Access::Write && data & 2 == 0 {
            return Err(format!("IIi write protection at VA={va:#x}").into());
        }
        if data & (1 << 60) != 0 || (access == Access::Flush && data & 8 != 0) {
            return Err(format!("IIi prohibited {access:?} page attributes at VA={va:#x}").into());
        }
        Ok(Translation {
            pa: (data & PHYSICAL_ADDRESS_MASK & !page_mask) | (va & page_mask),
            cacheable: data & 0x20 != 0,
            invert_endian: data & (1 << 59) != 0,
            slot: Some(slot),
        })
    }

    pub(super) fn set_used(&mut self, slot: usize, used: bool) -> bool {
        let data = &mut self.entries[slot].data;
        let old = *data & (1 << 41) != 0;
        *data = (*data & !(1 << 41)) | (u64::from(used) << 41);
        old
    }

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

    pub(super) fn demap(
        &mut self,
        address: u64,
        primary: u16,
        secondary: u16,
    ) -> Result<(), String> {
        if !matches!((address as i64) >> 43, 0 | -1) || address & 0xf != 0 {
            return Err(format!("invalid IIi demap address {address:#x}"));
        }
        let context = match (address >> 4) & 3 {
            0 => primary,
            1 => secondary,
            2 => 0,
            _ => return Ok(()),
        };
        for entry in &mut self.entries {
            let remove = if address & 0x40 == 0 {
                entry.matches_page(address, context)
            } else {
                entry.data & 1 == 0 && entry.tag & 0x1fff == u64::from(context)
            };
            if remove {
                entry.data &= !(1 << 63);
            }
        }
        Ok(())
    }

    pub(super) fn insert(&mut self, value: u64, instruction: bool) {
        // IIi §15.11.2. Choose the lowest eligible slot at each priority.
        const LOCK: u64 = 1 << 6;
        const USED: u64 = 1 << 41;
        let victim = if let Some(i) = self.entries.iter().position(|e| e.data >> 63 == 0) {
            i
        } else if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.data & (USED | LOCK) == 0)
        {
            i
        } else {
            // Model hardware Uclear at the replacement boundary.
            for e in &mut self.entries {
                e.data &= !USED;
            }
            self.entries
                .iter()
                .position(|e| e.data & LOCK == 0)
                .unwrap_or(63)
        };
        self.write_data(victim, value, instruction);
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
