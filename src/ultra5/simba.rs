//! Simba's two downstream PCI buses. Register offsets and window sizes are
//! sourced in docs/references.md. Configuration bytes are in PCI lane order.

use super::pbm::PCI_CONFIG;

const APB_CONFIG: u64 = PCI_CONFIG + 0x800;

pub(super) struct Simba {
    config: [[u8; 256]; 2],
}

impl Default for Simba {
    fn default() -> Self {
        let mut config = [[0; 256]; 2];
        for port in &mut config {
            // Both functions on the reference Ultra 5 identify as 108e:5000,
            // revision 11 (hex), conventional PCI-to-PCI bridge class 060400.
            port[..4].copy_from_slice(&0x5000_108eu32.to_le_bytes());
            port[8..12].copy_from_slice(&0x0604_0011u32.to_le_bytes());
            port[0x0e] = 0x81; // multifunction PCI bridge, Sun System Handbook
            port[6..8].copy_from_slice(&0x02a0u16.to_le_bytes());
            port[0x1e..0x20].copy_from_slice(&0x0280u16.to_le_bytes());
        }
        // The boot path must already forward the OBP flash on secondary B.
        // Other power-on window straps are unknown.
        config[1][4] = 2;
        config[1][0xdf] = 0x80;
        Self { config }
    }
}

impl Simba {
    fn decode<const N: usize>(pa: u64) -> Option<(usize, usize)> {
        let offset = usize::try_from(pa.checked_sub(APB_CONFIG)?).ok()?;
        if offset >= 512 || !matches!(N, 1 | 2 | 4 | 8) || offset % N != 0 {
            return None;
        }
        let start = offset & 0xff;
        (start..start + N)
            .all(|i| {
                matches!(i, 0..=0x0f | 0x18..=0x1b | 0x1e..=0x1f | 0x28..=0x2f | 0x3c..=0x3f | 0xc0 |
                0xc8..=0xcf | 0xd8..=0xdc | 0xde..=0xef)
            })
            .then_some((offset >> 8, start))
    }

    pub(super) fn read<const N: usize>(&self, pa: u64) -> Option<[u8; N]> {
        let (port, start) = Self::decode::<N>(pa)?;
        self.config[port][start..start + N].try_into().ok()
    }

    pub(super) fn write<const N: usize>(&mut self, pa: u64, bytes: [u8; N]) -> bool {
        let Some((port, start)) = Self::decode::<N>(pa) else {
            return false;
        };
        let regs = &mut self.config[port];
        for (i, value) in (start..).zip(bytes) {
            match i {
                4 => regs[i] = value & 0x47, // I/O, memory, master, parity response
                5 => regs[i] = value & 1,    // SERR enable
                7 | 0x1f => regs[i] &= !(value & 0xf9), // sticky PCI errors, W1C
                0x0c => regs[i] = value,     // cache line size in DWORDs
                0x0d | 0x1b => regs[i] = value & 0xf8,
                0x18..=0x1a | 0xc0 | 0xd8..=0xdc | 0xde..=0xdf => regs[i] = value,
                0x3e => regs[i] = value & 0x23, // secondary parity, SERR and master abort reporting
                0xe0 => regs[i] = value & 0x0f, // bus grants
                0xe1 => regs[i] = value & 1,    // error interrupt enable
                0xe2 => regs[i] = value & 0x20, // arbiter parking
                0xe4 => regs[i] = (regs[i] & 4 & !value) | (value & 1),
                // Idle DMA/PIO error latches have no error producer yet.
                _ => {}
            }
        }
        true
    }

    pub(super) fn onboard_config(&self, pa: u64) -> Option<(usize, usize)> {
        let b = &self.config[1];
        if b[0x18] != 0 || b[0x19] == 0 || b[0x19] > b[0x1a] {
            return None;
        }
        let base = PCI_CONFIG + (u64::from(b[0x19]) << 16);
        let offset = u16::try_from(pa.checked_sub(base)?).ok()?;
        let function = match offset >> 8 {
            0x08 => 0,
            0x09 => 1,
            0x10 => 2,
            0x18 => 3,
            _ => return None,
        };
        Some((function, usize::from(offset & 0xff)))
    }

    pub(super) fn forwards_onboard_io(&self, addr: u32) -> bool {
        let window = 1 << (addr >> 21);
        let selects = |port: &[u8; 256]| port[4] & 1 != 0 && port[0xde] & window != 0;
        selects(&self.config[1]) && !selects(&self.config[0])
    }

    pub(super) fn forwards_onboard_memory(&self, address: u32) -> bool {
        let window = 1 << (address >> 29);
        let selects = |port: &[u8; 256]| port[4] & 2 != 0 && port[0xdf] & window != 0;
        selects(&self.config[1]) && !selects(&self.config[0])
    }

    pub(super) fn signal_onboard_target_abort(&mut self) -> bool {
        let port = &mut self.config[1];
        if port[0x3e] & 0x20 == 0 {
            return false;
        }
        port[7] |= 0x08; // signaled target abort on primary
        true
    }

    /// Latch Received Master Abort on secondary B. With Bridge Control[5]
    /// clear the primary transaction completes with all ones or a dropped write.
    pub(super) fn complete_onboard_master_abort(&mut self) -> bool {
        let port = &mut self.config[1];
        port[0x1f] |= 0x20;
        port[0x3e] & 0x20 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pci_status_acknowledges_written_errors_in_the_selected_function() {
        let mut apb = Simba::default();
        for port in &mut apb.config {
            port[7] |= 0xf9;
            port[0x1f] |= 0xf9;
        }
        assert!(apb.write(APB_CONFIG + 4, [0x47, 1, 0xff, 0x41]));
        assert_eq!(apb.read(APB_CONFIG + 4), Some([0x47, 1, 0xa0, 0xba]));
        assert_eq!(apb.read(APB_CONFIG + 0x106), Some([0xa0, 0xfb]));
        assert!(apb.write(APB_CONFIG + 0x11e, [0xff; 2]));
        assert_eq!(apb.read(APB_CONFIG + 0x11e), Some([0x80, 0x02]));
        assert_eq!(apb.read(APB_CONFIG + 0x1e), Some([0x80, 0xfb]));
        for base in [APB_CONFIG, APB_CONFIG + 0x100] {
            for offset in [0xc8, 0xe8] {
                assert!(apb.write(base + offset, [0xff; 8]));
                assert_eq!(apb.read(base + offset), Some([0; 8]));
            }
        }
    }

    #[test]
    fn unsupported_spans_do_not_write_any_lanes_or_alias_other_functions() {
        let mut apb = Simba::default();
        assert!(apb.write(APB_CONFIG + 0xd8, [1, 2, 3, 4]));
        let before = apb.config;
        for pa in [
            APB_CONFIG + 0xd8,
            APB_CONFIG + 0xd9,
            APB_CONFIG + 0x200,
            APB_CONFIG + 0x10000 + 0xd8,
            APB_CONFIG - 0x100,
        ] {
            assert!(!apb.write(pa, [0xff; 8]));
            assert_eq!(apb.read::<8>(pa), None);
        }
        assert_eq!(apb.config, before);
    }

    #[test]
    fn onboard_memory_requires_an_exclusive_enabled_bus_b_window() {
        let mut apb = Simba::default();
        for bit in 0..8 {
            assert!(apb.write(APB_CONFIG + 0x1df, [1 << bit]));
            for index in 0..8 {
                for edge in [0, 0x1fff_ffff] {
                    let address = index << 29 | edge;
                    assert_eq!(apb.forwards_onboard_memory(address), bit == index);
                }
            }
        }
        assert!(apb.write(APB_CONFIG + 0xdf, [0x80]));
        assert!(apb.write(APB_CONFIG + 4, [2]));
        assert!(!apb.forwards_onboard_memory(0xf000_0000));
        assert!(apb.write(APB_CONFIG + 0x104, [1])); // I/O enabled, memory disabled
        assert!(!apb.forwards_onboard_memory(0xf000_0000));
    }
}
