//! UltraSPARC-IIi PCI bus module. Sun IIi User's Manual, chapter 19.

const BASE: u64 = 0x01fe_0000_0000;
pub(super) const PCI_CONFIG: u64 = 0x01fe_0100_0000;
const CONTROL_RW: u64 = (1 << 36) | (0x3f << 16) | (3 << 7) | 0xf;
const SERR: u64 = 1 << 34;

#[derive(Default)]
pub(super) struct Pbm {
    control: u64,
    // One bit for each 512 MiB region of the incoming 32-bit PCI address space.
    target_regions: u8,
    disable_retry_limit: bool,
    pub(super) header: PciHeader,
    enabled: u64,
    pci_interrupts: u64,
    onboard_interrupts: u64,
}

// First 16 configuration bytes of bus 0, device 0, function 0 in PCI byte
// order. The remaining bus-number registers use the board's existing decoder.
pub(super) struct PciHeader([u8; 16]);

impl Default for PciHeader {
    fn default() -> Self {
        // IIi §§19.3.1.1–10. IDs 108e:a000, command 0006, status 02a0,
        // revision zero, class 060000, fixed type-0 single-function header.
        Self([
            0x8e, 0x10, 0x00, 0xa0, 0x06, 0x00, 0xa0, 0x02, 0, 0, 0, 6, 0, 0, 0, 0,
        ])
    }
}

impl PciHeader {
    fn offset<const N: usize>(pa: u64) -> Option<usize> {
        let offset = pa.checked_sub(PCI_CONFIG)?;
        (matches!(N, 1 | 2 | 4 | 8) && offset < 16 && offset % N as u64 == 0)
            .then_some(offset as usize)
    }

    pub(super) fn read<const N: usize>(&self, pa: u64) -> Option<[u8; N]> {
        let start = Self::offset::<N>(pa)?;
        self.0.get(start..start + N)?.try_into().ok()
    }

    pub(super) fn write<const N: usize>(&mut self, pa: u64, data: [u8; N]) -> bool {
        let Some(start) = Self::offset::<N>(pa) else {
            return false;
        };
        // All callers issue naturally aligned 1-, 2-, 4- or 8-byte transactions.
        for (offset, value) in (start..).zip(data) {
            match offset {
                4 => self.0[4] = 6 | (value & 0x40), // PER; MSTR/MEM hardwired
                5 => self.0[5] = value & 1,          // SERR_EN
                7 => self.0[7] &= !(value & 0xf9),   // status errors are W1C
                0xd => self.0[0xd] = value & 0xf8,   // latency in units of 8 clocks
                _ => {}
            }
        }
        true
    }
}

impl Pbm {
    pub(super) fn selects_pci_memory(&self, address: u32) -> bool {
        self.target_regions & (1 << (address >> 29)) != 0
    }

    pub(super) fn offset(pa: u64) -> Option<u16> {
        u16::try_from(pa.checked_sub(BASE)?).ok()
    }

    pub(super) fn read(&self, offset: u16) -> Result<u64, String> {
        if let Some(ino) = Self::partial_ino(offset) {
            let valid = (self.enabled >> ino) & 1;
            return Ok((valid << 31) | (0x1f << 6) | u64::from(ino));
        }
        match offset {
            0x2000 => Ok(self.control),
            0x2020 => Ok(u64::from(self.disable_retry_limit) << 6),
            0x2028 => Ok(u64::from(self.target_regions)),
            0xa800 => Ok(self.pci_interrupts),
            0xa808 => Ok(self.onboard_interrupts),
            _ => Err(format!(
                "unsupported IIi PBM xword read at {:#x}",
                BASE + u64::from(offset)
            )),
        }
    }

    pub(super) fn write(&mut self, offset: u16, value: u64) -> Result<(), String> {
        if let Some(ino) = Self::partial_ino(offset) {
            let valid = (value >> 31) & 1;
            self.enabled = (self.enabled & !(1 << ino)) | (valid << ino);
            return Ok(());
        }
        match offset {
            0x2000 => {
                self.control = (self.control & SERR & !value) | (value & CONTROL_RW);
                Ok(())
            }
            0x2020 => {
                if value & 0x0e != 0 {
                    return Err("unsupported IIi diagnostic PCI parity inversion".into());
                }
                self.disable_retry_limit = value & (1 << 6) != 0;
                Ok(())
            }
            0x2028 => {
                self.target_regions = value as u8;
                Ok(())
            }
            0x1400..=0x14f8 if offset & 7 == 0 => {
                Self::set_interrupt(&mut self.pci_interrupts, offset - 0x1400, value)
            }
            0x1800..=0x1858 | 0x1870..=0x1880 if offset & 7 == 0 => {
                Self::set_interrupt(&mut self.onboard_interrupts, offset - 0x1800, value)
            }
            // OBP sweeps across these reserved mapping and clear ports with zero.
            // INOs 0x2c/0x2d have neither a partial map nor level interrupt state.
            0x1060 | 0x1068 | 0x1860 | 0x1868 if value == 0 => Ok(()),
            _ => Err(format!(
                "unsupported IIi PBM xword write at {:#x} value={value:#018x}",
                BASE + u64::from(offset)
            )),
        }
    }

    fn partial_ino(offset: u16) -> Option<u16> {
        if offset & 7 != 0 {
            return None;
        }
        match offset {
            0x0c00..=0x0c38 => Some(((offset - 0x0c00) / 8) * 4),
            0x1000..=0x1058 | 0x1070..=0x1080 => Some(0x20 + (offset - 0x1000) / 8),
            _ => None,
        }
    }

    fn set_interrupt(bank: &mut u64, offset: u16, value: u64) -> Result<(), String> {
        let state = value & 3;
        if state == 2 {
            return Err("reserved IIi interrupt clear state 2".into());
        }
        // One xword port per source, two bits per source in the diagnostic register.
        let shift = (offset / 8) * 2;
        *bank = (*bank & !(3 << shift)) | (state << shift);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_pci_regions_decode_both_edges_independently() {
        let mut pbm = Pbm::default();
        assert_eq!(pbm.read(0x2028).unwrap(), 0);
        for bits in 0..=255 {
            pbm.write(0x2028, !0xff | bits).unwrap();
            assert_eq!(pbm.read(0x2028).unwrap(), bits);
            for region in 0..8 {
                let enabled = bits & (1 << region) != 0;
                let start = region << 29;
                assert_eq!(pbm.selects_pci_memory(start), enabled);
                assert_eq!(pbm.selects_pci_memory(start | 0x1fff_ffff), enabled);
            }
        }
    }

    #[test]
    fn diagnostic_retry_bit_and_parity_rejection_preserve_target_selection() {
        let mut pbm = Pbm::default();
        assert_eq!(pbm.read(0x2020).unwrap(), 0);
        pbm.write(0x2028, 0x40).unwrap();
        pbm.write(0x2020, !0xe).unwrap();
        assert_eq!(pbm.read(0x2020).unwrap(), 0x40);
        for invert in [2, 4, 8] {
            assert!(pbm.write(0x2020, invert).is_err());
            assert_eq!(pbm.read(0x2020).unwrap(), 0x40);
        }
        pbm.write(0x2020, 0).unwrap();
        assert_eq!(pbm.read(0x2020).unwrap(), 0);
        assert_eq!(pbm.read(0x2028).unwrap(), 0x40);
    }

    #[test]
    fn pci_header_status_ack_is_confined_to_written_lanes() {
        let mut header = PciHeader::default();
        header.0[7] |= 0xf9; // Seed every sticky error in table 19-14.
        assert!(header.write(PCI_CONFIG + 4, [0xff, 0xff]));
        assert_eq!(header.read(PCI_CONFIG + 4), Some([0x46, 1, 0xa0, 0xfb]));
        assert!(header.write(PCI_CONFIG + 6, [0xff, 0x21]));
        assert_eq!(header.read(PCI_CONFIG + 4), Some([0x46, 1, 0xa0, 0xda]));
        let before = header.0;
        assert!(!header.write(PCI_CONFIG + 7, [0xff; 2]));
        assert!(!header.write(PCI_CONFIG + 16, [0xff]));
        assert_eq!(header.0, before);
        assert!(header.write(PCI_CONFIG, [0xff; 8]));
        assert_eq!(
            header.read(PCI_CONFIG),
            Some([0x8e, 0x10, 0, 0xa0, 0x46, 1, 0xa0, 2])
        );
    }

    #[test]
    fn pci_control_masks_reserved_bits_and_clears_serr_on_one() {
        let mut pbm = Pbm::default();
        assert_eq!(pbm.read(0x2000).unwrap(), 0);
        pbm.write(0x2000, u64::MAX).unwrap();
        assert_eq!(pbm.read(0x2000).unwrap(), 0x10_003f_018f);
        // Seed a latched bus error independently of any future PCI error producer.
        pbm.control |= 1 << 34;
        pbm.write(0x2000, 0x80).unwrap();
        assert_eq!(pbm.read(0x2000).unwrap(), 0x04_0000_0080);
        pbm.write(0x2000, 0x14_0020_010f).unwrap();
        assert_eq!(pbm.read(0x2000).unwrap(), 0x10_0020_010f);
    }

    #[test]
    fn partial_maps_have_fixed_vectors_and_independent_valid_bits() {
        let mut pbm = Pbm::default();
        let pci = (0..8).map(|slot| (0xc00 + 8 * slot, 0x7c0 + 4 * u64::from(slot)));
        let onboard = (0..=16)
            .filter(|&i| i != 12 && i != 13)
            .map(|i| (0x1000 + 8 * i, 0x7e0 + u64::from(i)));
        let ports: Vec<_> = pci.chain(onboard).collect();
        for &(offset, vector) in &ports {
            assert_eq!(pbm.read(offset).unwrap(), vector);
            pbm.write(offset, u64::MAX).unwrap();
        }
        pbm.write(0x1400, 3).unwrap();
        for &(offset, vector) in &ports {
            assert_eq!(pbm.read(offset).unwrap(), 0x8000_0000 | vector);
            pbm.write(offset, 0x7ff).unwrap();
            assert_eq!(pbm.read(offset).unwrap(), vector);
        }
        assert_eq!(pbm.read(0xa800).unwrap(), 3);
    }

    #[test]
    fn interrupt_clear_ports_select_two_diagnostic_bits_per_level() {
        let mut pbm = Pbm::default();
        assert_eq!(pbm.read(0xa800).unwrap(), 0);
        assert_eq!(pbm.read(0xa808).unwrap(), 0);
        pbm.write(0x1400, 3).unwrap(); // PCI A slot 0 INTA
        pbm.write(0x14f8, 1).unwrap(); // PCI B slot 3 INTD
        pbm.write(0x1858, 3).unwrap(); // Serial
        pbm.write(0x1880, 1).unwrap(); // PCI error
        assert!(pbm.write(0x1858, 2).is_err());
        for hole in [0x1060, 0x1068, 0x1860, 0x1868] {
            pbm.write(hole, 0).unwrap();
            assert!(pbm.write(hole, 1).is_err());
            assert!(pbm.read(hole).is_err());
        }
        assert_eq!(pbm.read(0xa808).unwrap(), (1 << 32) | (3 << 22));
        assert_eq!(pbm.read(0xa800).unwrap(), (1 << 62) | 3);
        assert!(pbm.write(0xa800, 0).is_err());
        assert!(pbm.read(0x14f8).is_err());
        pbm.write(0x14f8, 0).unwrap();
        assert_eq!(pbm.read(0xa800).unwrap(), 3);
    }
}
