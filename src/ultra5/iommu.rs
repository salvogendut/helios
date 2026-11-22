//! UltraSPARC-IIi PBM IOMMU programming interface, manual §19.3.2.
//! The PCI master DMA path is separate from these CPU-visible registers.

const DIAGNOSTIC_ENABLE: u64 = 1 << 1;
const CONTROL_RW: u64 = 0x00ff_0007;
const ERROR_STATUS: u64 = 7 << 24;
const TSB_BASE_MASK: u64 = ((1 << 34) - 1) & !0x1fff;

const TAG_BASE: u16 = 0xa580;
const DATA_BASE: u16 = 0xa600;
const PAGE_64K: u32 = 1 << 19;
const VPN: u32 = 0x7ffff;
const VALID: u32 = 1 << 30;
const CACHEABLE: u32 = 1 << 28;
const HIGH_PA: u32 = 0x0fe0_0000;

#[derive(Default, Copy, Clone)]
struct Entry {
    tag: u32,
    data: u32,
}

#[derive(Default)]
pub(super) struct Iommu {
    control: u64,
    tsb_base: u64,
    entries: [Entry; 16],
}

impl Iommu {
    fn diagnostic_index(&self, offset: u16) -> Result<usize, String> {
        if self.control & DIAGNOSTIC_ENABLE == 0 {
            return Err(format!("IIi IOMMU diagnostic mode disabled at {offset:#x}"));
        }
        // Each bank is 128 bytes with one xword per entry.
        Ok(usize::from((offset & 0x7f) / 8))
    }

    pub(super) fn read(&self, offset: u16) -> Result<u64, String> {
        match offset {
            0x200 => Ok(self.control),
            0x208 => Ok(self.tsb_base),
            TAG_BASE..=0xa5f8 if offset & 7 == 0 => {
                Ok(u64::from(self.entries[self.diagnostic_index(offset)?].tag))
            }
            DATA_BASE..=0xa678 if offset & 7 == 0 => {
                let data = self.entries[self.diagnostic_index(offset)?].data;
                // Bits 27:21 are synthesized from cacheability on read.
                let high = if data & CACHEABLE == 0 { HIGH_PA } else { 0 };
                Ok(u64::from(data | high))
            }
            _ => Err(format!(
                "unsupported IIi IOMMU xword read at offset {offset:#x}"
            )),
        }
    }

    pub(super) fn write(&mut self, offset: u16, value: u64) -> Result<(), String> {
        match offset {
            0x200 => self.control = (value & CONTROL_RW) | (self.control & ERROR_STATUS & !value),
            0x208 => self.tsb_base = value & TSB_BASE_MASK,
            0x210 => {
                let vpn = (value as u32) >> 13;
                for entry in &mut self.entries {
                    let mask = if entry.tag & PAGE_64K == 0 {
                        VPN
                    } else {
                        VPN & !7
                    };
                    if (entry.tag ^ vpn) & mask == 0 {
                        entry.data &= !VALID;
                    }
                }
            }
            TAG_BASE..=0xa5f8 if offset & 7 == 0 => {
                let i = self.diagnostic_index(offset)?;
                self.entries[i].tag = value as u32 & 0x01ff_ffff;
                if value & (1 << 22) != 0 {
                    // A tag written with ERR set logs ERR and ERRSTS in control.
                    self.control = (self.control & !ERROR_STATUS) | ((value << 2) & ERROR_STATUS);
                }
            }
            DATA_BASE..=0xa678 if offset & 7 == 0 => {
                let i = self.diagnostic_index(offset)?;
                self.entries[i].data = value as u32 & 0x701f_ffff;
            }
            _ => {
                return Err(format!(
                    "unsupported IIi IOMMU xword write at offset {offset:#x}"
                ))
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_fields_error_acknowledgement_and_8k_base_alignment() {
        let mut io = Iommu::default();
        assert_eq!(io.read(0x200).unwrap(), 0);
        io.write(0x200, u64::MAX).unwrap();
        assert_eq!(io.read(0x200).unwrap(), 0x00ff_0007);
        io.write(0x208, u64::MAX).unwrap();
        assert_eq!(io.read(0x208).unwrap(), 0x3_ffff_e000);
        io.write(0x208, 0x3ff5_c321).unwrap();
        assert_eq!(io.read(0x208).unwrap(), 0x3ff5_c000);
        io.write(0xa580, 0x01c0_0050).unwrap(); // ERRSTS=3, ERR=1
        assert_eq!(io.read(0x200).unwrap(), 0x07ff_0007);
        io.write(0x200, 0x0200_0002).unwrap();
        assert_eq!(io.read(0x200).unwrap(), 0x0500_0002);
        io.write(0x200, 0x0500_0002).unwrap();
        assert_eq!(io.read(0x200).unwrap(), 2);
        assert_eq!(io.read(0xa580).unwrap(), 0x01c0_0050);
    }

    #[test]
    fn diagnostic_lanes_masks_and_mode_are_checked_before_mutation() {
        let mut io = Iommu::default();
        io.write(0x200, 2).unwrap();
        for slot in 0..16 {
            let offset = 8 * slot;
            let word = u64::from(slot) + 1;
            io.write(0xa580 + offset, 0xffff_ffff_fe00_0000 | word)
                .unwrap();
            io.write(0xa600 + offset, 0xffff_ffff_ffe0_0000 | word)
                .unwrap();
        }
        io.write(0x200, 1).unwrap();
        for offset in [0xa580, 0xa5f8, 0xa600, 0xa678] {
            assert!(io.read(offset).is_err());
            assert!(io.write(offset, 0).is_err());
        }
        io.write(0x200, 2).unwrap();
        for offset in [0x204, 0x20c, 0xa57f, 0xa581, 0xa5ff, 0xa604, 0xa680] {
            assert!(io.read(offset).is_err());
            assert!(io.write(offset, 0).is_err());
        }
        for slot in 0..16 {
            let offset = 8 * slot;
            let word = u64::from(slot) + 1;
            assert_eq!(io.read(0xa580 + offset).unwrap(), word);
            assert_eq!(io.read(0xa600 + offset).unwrap(), 0x7000_0000 | word);
        }
        io.write(0xa678, 0x4000_1234).unwrap();
        assert_eq!(io.read(0xa678).unwrap(), 0x4fe0_1234);
    }

    #[test]
    fn address_flush_uses_the_size_of_each_installed_entry() {
        let mut io = Iommu::default();
        io.write(0x200, 2).unwrap();
        for (slot, tag) in [(0, 0x4000), (1, 0x4001), (15, 0x8_4100)] {
            io.write(0xa580 + 8 * slot, tag).unwrap();
            io.write(0xa600 + 8 * slot, 0x7000_002a).unwrap();
        }
        io.write(0x200, 1).unwrap();
        io.write(0x210, 0xffff_ffff_0800_1fff).unwrap(); // first 8 KiB page
        io.write(0x210, 0x0820_ffff).unwrap(); // last byte of the 64 KiB page
        assert!(io.read(0x210).is_err());
        io.write(0x200, 2).unwrap();
        assert_eq!(io.read(0xa600).unwrap(), 0x3000_002a);
        assert_eq!(io.read(0xa608).unwrap(), 0x7000_002a);
        assert_eq!(io.read(0xa678).unwrap(), 0x3000_002a);
        assert_eq!(io.read(0xa5f8).unwrap(), 0x8_4100);
    }
}
