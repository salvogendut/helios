//! Register aperture used by the onboard ATI 3D Rage II+ firmware probe.
//! PCI exposes the little-endian register block at BAR0 + 0x7ffc00.
pub(super) struct Mach64 {
    registers: [u8; 1024],
    pll: [u8; 64],
    vram: Box<[u8]>,
}
impl Default for Mach64 {
    fn default() -> Self {
        let mut chip = Self {
            registers: [0; 1024],
            pll: [0; 64],
            vram: vec![0; 4 << 20].into_boxed_slice(),
        };
        chip.registers[0xe0..0xe4].copy_from_slice(&0x9a00_4754u32.to_le_bytes());
        chip.registers[0xb0] = 7; // four MiB SGRAM, 512-KiB units
        chip
    }
}
impl Mach64 {
    fn index(off: u32, n: usize) -> Result<usize, String> {
        off.checked_sub(0x7ffc00).map(|v|v as usize).filter(|&i|i+n<=1024 && i%n==0 && matches!(i&!3,0x00..=0x0c|0x14..=0x24|0x2c|0x34|0x40..=0x48|0x78|0x90|0xa0|0xac|0xb0|0xc0|0xc4|0xd0|0xe0|0xe4|0x1b4))
        .ok_or_else(||format!("unsupported Mach64 aperture +{off:#x} width {n}"))
    }
    pub fn read<const N: usize>(&self, off: u32) -> Result<[u8; N], String> {
        if (off & 0x7f_ffff) < 0x7f_f800 {
            return Ok(std::array::from_fn(|k| self.vram[Self::vram_index(off, k)]));
        }
        let i = Self::index(off, N)?;
        Ok(std::array::from_fn(|k| {
            if i + k == 0x92 {
                self.pll[usize::from(self.registers[0x91] >> 2)]
            } else {
                self.registers[i + k]
            }
        }))
    }
    fn vram_index(off: u32, lane: usize) -> usize {
        ((off as usize + lane) & 0x3f_ffff) ^ if off & 0x80_0000 != 0 { 3 } else { 0 }
    }
    pub fn write<const N: usize>(&mut self, off: u32, bytes: [u8; N]) -> Result<(), String> {
        if (off & 0x7f_ffff) < 0x7f_f800 {
            for (k, b) in bytes.into_iter().enumerate() {
                self.vram[Self::vram_index(off, k)] = b;
            }
            return Ok(());
        }
        let i = Self::index(off, N)?;
        for (j, b) in (i..).zip(bytes) {
            match j {
                0xe0..=0xe7 => {}
                0x92 if self.registers[0x91] & 2 != 0 => {
                    self.pll[usize::from(self.registers[0x91] >> 2)] = b
                }
                0x92 => {}
                0xa2 => self.registers[j] = b & !0xa0,
                _ => self.registers[j] = b,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn register_widths_pll_selection_and_framebuffer_storage_are_independent() {
        let mut ati = Mach64::default();
        ati.write(0x7ffc00, [1, 2, 3, 4]).unwrap();
        assert_eq!(ati.read::<2>(0x7ffc02).unwrap(), [3, 4]);
        assert!(ati.read::<4>(0x7ffc02).is_err());
        assert!(ati.read::<4>(0x7ffe00).is_err());
        ati.write(0x7ffc91, [0x12]).unwrap();
        ati.write(0x7ffc92, [0x55]).unwrap();
        ati.write(0x7ffc91, [0x14]).unwrap();
        assert_eq!(ati.read::<1>(0x7ffc92).unwrap(), [0]);
        ati.write(0x7ffc91, [0x10]).unwrap();
        assert_eq!(ati.read::<1>(0x7ffc92).unwrap(), [0x55]);
        ati.write(0x3fff00, [5, 6, 7, 8]).unwrap();
        assert_eq!(ati.read::<4>(0x3fff00).unwrap(), [5, 6, 7, 8]);
        assert_eq!(ati.read::<4>(0x7ffc00).unwrap(), [1, 2, 3, 4]);
    }
}
