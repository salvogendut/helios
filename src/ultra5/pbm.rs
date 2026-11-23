//! UltraSPARC-IIi PCI bus module. Sun IIi User's Manual, chapter 19.

use super::iommu::Iommu;

const BASE: u64 = 0x01fe_0000_0000;
pub(super) const PCI_CONFIG: u64 = 0x01fe_0100_0000;
const CONTROL_RW: u64 = (1 << 36) | (0x3f << 16) | (3 << 7) | 0xf;
const SERR: u64 = 1 << 34;

#[derive(Default)]
pub(super) struct Pbm {
    control: u64,
    faults: FaultStatus,
    pub(super) iommu: Iommu,
    // One bit for each 512 MiB region of the incoming 32-bit PCI address space.
    target_regions: u8,
    disable_retry_limit: bool,
    pub(super) header: PciHeader,
    enabled: u64,
    pci_interrupts: u64,
    onboard_interrupts: u64,
}

// IIi tables 19-3, 19-44 and 19-46. The DMA byte mask reads 0x00ff at POR.
struct FaultStatus {
    pio_write: u64,
    dma_uncorrectable: u64,
    dma_correctable: u64,
}

impl Default for FaultStatus {
    fn default() -> Self {
        Self {
            pio_write: 0,
            dma_uncorrectable: 0xff << 32,
            dma_correctable: 0xff << 32,
        }
    }
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
    pub(super) fn master_abort(&mut self) {
        self.0[7] |= 0x20;
    }

    pub(super) fn target_abort(&mut self) {
        self.0[7] |= 0x10;
    }

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
            0x200..=0x217 | 0xa580..=0xa67f => self.iommu.read(offset),
            0x0010 | 0x0020 | 0x0100 | 0x0108 | 0x1a00 => Ok(0), // Reserved IIi CSRs (table 6-6).
            0x2000 => Ok(self.control),
            0x2010 => Ok(self.faults.pio_write),
            0x0030 => Ok(self.faults.dma_uncorrectable),
            0x0040 => Ok(self.faults.dma_correctable),
            0x2020 => Ok(u64::from(self.disable_retry_limit) << 6),
            0x2028 => Ok(u64::from(self.target_regions)),
            // The clear ports are write-only. Solaris reads back a posted clear.
            0x1400..=0x14f8 | 0x1800..=0x1858 | 0x1870..=0x1880 if offset & 7 == 0 => Ok(0),
            0x1c20 => Ok(0), // all prior DMA writes have completed
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
            0x200..=0x217 | 0xa580..=0xa67f => self.iommu.write(offset, value),
            0x0010 | 0x0020 | 0x0100 | 0x0108 | 0x1a00 => Ok(()),
            0x2000 => {
                self.control = (self.control & SERR & !value) | (value & CONTROL_RW);
                Ok(())
            }
            0x0030 | 0x0040 | 0x2010 => {
                // Only primary and secondary cause bits are write-one-to-clear.
                // Translation errors are logged by the uncorrectable DMA bank.
                let (status, flags) = match offset {
                    0x0030 => (&mut self.faults.dma_uncorrectable, 0x6f_u64),
                    0x0040 => (&mut self.faults.dma_correctable, 0x6c_u64),
                    _ => (&mut self.faults.pio_write, 0xff_u64),
                };
                *status &= !(value & (flags << 56));
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

    pub(super) fn assert_pci(&mut self, ino: u8) {
        let shift = 2 * ino;
        if (self.pci_interrupts >> shift) & 3 == 0 {
            self.pci_interrupts |= 1u64 << shift;
        }
    }
    pub(super) fn assert_onboard(&mut self, ino: u8) {
        let shift = 2 * (ino - 0x20);
        if (self.onboard_interrupts >> shift) & 3 == 0 {
            self.onboard_interrupts |= 1u64 << shift;
        }
    }

    pub(super) fn pending_vector(&self) -> Option<u8> {
        // The serial and keyboard sources outrank the IDE interrupt.
        [0x2b, 0x29, 0x2a, 0x27, 0x20]
            .into_iter()
            .find(|&ino| {
                self.enabled & (1u64 << ino) != 0
                    && self.onboard_interrupts >> (2 * (ino - 0x20)) & 3 == 1
            })
            .or_else(|| {
                (self.enabled & (1 << 0xc) != 0 && self.pci_interrupts >> (2 * 0xf) & 3 == 1)
                    .then_some(0xf)
            })
    }
    pub(super) fn sent_vector(&mut self, ino: u8) {
        if ino < 0x20 {
            self.pci_interrupts |= 3u64 << (2 * ino);
        } else {
            self.onboard_interrupts |= 3u64 << (2 * (ino - 0x20));
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
    fn mach64_shares_the_slot_map_and_acknowledges_its_own_pin() {
        let mut pbm = Pbm::default();
        pbm.assert_pci(0xf);
        assert_eq!(pbm.pending_vector(), None);
        pbm.write(0xc18, 1 << 31).unwrap();
        assert_eq!(pbm.pending_vector(), Some(0xf));
        pbm.sent_vector(0xf);
        assert_eq!(pbm.pending_vector(), None);
        assert_eq!(pbm.read(0xa800).unwrap(), 3 << 30);
        pbm.write(0x1460, 0).unwrap();
        assert_eq!(pbm.read(0xa800).unwrap(), 3 << 30);
        pbm.write(0x1478, 0).unwrap();
        assert_eq!(pbm.read(0xa800).unwrap(), 0);
    }

    #[test]
    fn fault_status_reset_and_w1c_are_specific_to_each_register() {
        let mut pbm = Pbm::default();
        let regs = [0x2010, 0x0030, 0x0040];
        for (offset, expected) in regs
            .into_iter()
            .zip([0, 0x00ff_0000_0000, 0x00ff_0000_0000])
        {
            assert_eq!(pbm.read(offset).unwrap(), expected);
            pbm.write(offset, u64::MAX).unwrap();
            assert_eq!(pbm.read(offset).unwrap(), expected);
        }
        // A secondary cause can remain set after acknowledgement of the primary.
        // Check every W1C bit with representative read-only transfer metadata.
        for (offset, causes, metadata) in [
            (
                0x2010,
                &[63, 62, 61, 60, 59, 58, 57, 56][..],
                0x005a_8000_0000,
            ),
            (0x0030, &[62, 61, 59, 58, 57, 56][..], 0xff00_a000_0000),
            (0x0040, &[62, 61, 59, 58][..], 0x009b_00ff_4000_0000),
        ] {
            for &cause in causes {
                let bit = 1u64 << cause;
                let captured = bit | metadata;
                match offset {
                    0x2010 => pbm.faults.pio_write = captured,
                    0x0030 => pbm.faults.dma_uncorrectable = captured,
                    _ => pbm.faults.dma_correctable = captured,
                }
                pbm.write(offset, !bit).unwrap();
                assert_eq!(pbm.read(offset).unwrap(), captured);
                let before = regs.map(|r| pbm.read(r).unwrap());
                pbm.write(offset, bit).unwrap();
                for (r, prior) in regs.into_iter().zip(before) {
                    assert_eq!(
                        pbm.read(r).unwrap(),
                        if r == offset { metadata } else { prior }
                    );
                }
            }
        }
    }

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
        assert_eq!(pbm.read(0x14f8).unwrap(), 0);
        pbm.write(0x14f8, 0).unwrap();
        assert_eq!(pbm.read(0xa800).unwrap(), 3);
    }
}
