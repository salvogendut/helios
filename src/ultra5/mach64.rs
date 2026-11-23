//! Register aperture used by the onboard ATI 3D Rage II+ firmware probe.
//! PCI exposes the little-endian register block at BAR0 + 0x7ffc00.
mod engine;
mod raster;

const COMMAND_FIFO_ENTRIES: u8 = 48;

pub(super) struct Mach64 {
    registers: [u8; 1024],
    pll: [u8; 64],
    raster_events: [u64; 2],
    raster_period: u64,
    host: Option<engine::HostRectangle>,
    palette: [[u8; 3]; 256],
    write_component: usize,
    read_component: usize,
    pub(super) generation: u64,
    vram: Box<[u8]>,
}
impl Default for Mach64 {
    fn default() -> Self {
        let mut chip = Self {
            registers: [0; 1024],
            pll: [0; 64],
            raster_events: [u64::MAX; 2],
            raster_period: 0,
            host: None,
            palette: [[0; 3]; 256],
            write_component: 0,
            read_component: 0,
            generation: 0,
            vram: vec![0; 4 << 20].into_boxed_slice(),
        };
        chip.registers[0xe0..0xe4].copy_from_slice(&0x9a00_4754u32.to_le_bytes());
        chip.registers[0xc2] = 0xff;
        chip.registers[0xb0] = 7; // four MiB SGRAM, 512-KiB units
        chip
    }
}
impl Mach64 {
    fn reg(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.registers[offset..offset + 4].try_into().unwrap())
    }

    pub(super) fn size(&self) -> Option<(usize, usize)> {
        let control = self.reg(0x1c);
        if control & 0x0200_0040 != 0x0200_0000 || !(2..=6).contains(&((control >> 8) & 7)) {
            return None;
        }
        let width = (((self.reg(0) >> 16) & 0xff) + 1) * 8;
        let height = ((self.reg(8) >> 16) & 0x7ff) + 1;
        let pitch = (self.reg(0x14) >> 22) * 8;
        (pitch >= width).then_some((width as usize, height as usize))
    }

    pub(super) fn pixels(&self, output: &mut [u32]) {
        let Some((w, h)) = self.size() else {
            return;
        };
        let layout = self.reg(0x14);
        let origin = ((layout & 0xfffff) * 8) as usize;
        let pitch = ((layout >> 22) * 8) as usize;
        let full_color = self.reg(0xc4) & 0x100 != 0;
        let lut = self.palette.map(|rgb| {
            let [r, g, b] = rgb.map(|c| {
                u32::from(if full_color {
                    c
                } else {
                    ((c & 63) << 2) | ((c & 63) >> 4)
                })
            });
            (r << 16) | (g << 8) | b
        });
        let depth = (self.reg(0x1c) >> 8) & 7;
        let bytes = match depth {
            2 => 1,
            3 | 4 => 2,
            5 => 3,
            _ => 4,
        };
        let vmask = self.vram.len() - 1;
        let expand5 = |v: u32| (v << 3) | (v >> 2);
        let expand6 = |v: u32| (v << 2) | (v >> 4);
        for (y, row) in output.chunks_exact_mut(w).take(h).enumerate() {
            for (x, pixel) in row.iter_mut().enumerate() {
                let p = origin + bytes * (y * pitch + x);
                let octet = |lane: usize| u32::from(self.vram[(p + lane) & vmask]);
                *pixel = match depth {
                    2 => lut[(octet(0) & u32::from(self.registers[0xc2])) as usize],
                    3 | 4 => {
                        let rgb = octet(0) | (octet(1) << 8);
                        let (r, g) = if depth == 3 {
                            (rgb >> 10 & 31, expand5(rgb >> 5 & 31))
                        } else {
                            (rgb >> 11, expand6(rgb >> 5 & 63))
                        };
                        (expand5(r) << 16) | (g << 8) | expand5(rgb & 31)
                    }
                    _ => octet(0) | (octet(1) << 8) | (octet(2) << 16),
                };
            }
        }
        self.cursor(output, w, h);
    }

    fn cursor(&self, output: &mut [u32], w: usize, h: usize) {
        if self.reg(0xd0) & 0x80 == 0 {
            return;
        }
        let pos = self.reg(0x6c);
        let (left, top) = ((pos & 0x7ff) as usize, (pos >> 16 & 0x7ff) as usize);
        if left >= w || top >= h {
            return;
        }
        let off = self.reg(0x70);
        let (skip_x, skip_y) = ((off & 63) as usize, (off >> 16 & 63) as usize);
        let base = ((self.reg(0x68) & 0xf_ffff) as usize) * 8;
        let colors = [self.reg(0x60) >> 8, self.reg(0x64) >> 8];
        // The driver has already advanced CUR_OFFSET for clipped top rows.
        // CUR_VERT_OFFSET reduces the visible height.
        for (y, row) in output
            .chunks_exact_mut(w)
            .take(h)
            .skip(top)
            .take(64 - skip_y)
            .enumerate()
        {
            for (x, pixel) in row.iter_mut().skip(left).take(64 - skip_x).enumerate() {
                let bit = x + skip_x;
                let data = self.vram[(base + y * 16 + bit / 4) & (self.vram.len() - 1)];
                match (data >> (2 * (bit % 4))) & 3 {
                    v @ 0..=1 => *pixel = colors[usize::from(v)],
                    3 => *pixel ^= 0xff_ffff,
                    _ => {}
                }
            }
        }
    }

    fn index(off: u32, n: usize) -> Result<usize, String> {
        off.checked_sub(0x7ffc00)
            .map(|v|v as usize)
            .filter(|&i| n<=4 && i+n<=1024 && i%n==0 && matches!(i&!3,
                0x00..=0x24|0x2c|0x34|0x40..=0x48|0x60..=0x70|0x78..=0x90|0xa0|0xac|0xb0|0xc0|0xc4|0xd0|0xe0|0xe4|
                0x100..=0x118|0x120..=0x12c|0x130|0x134|0x180..=0x1b4|0x200..=0x23c|0x240|0x280..=0x288|0x2a0..=0x2d8|0x2e8|0x2ec|0x300..=0x308|0x310|0x320|0x330|0x338))
            .ok_or_else(||format!("unsupported Mach64 aperture +{off:#x} width {n}"))
    }
    pub fn read<const N: usize>(&mut self, off: u32) -> Result<[u8; N], String> {
        if (off & 0x7f_ffff) < 0x7f_f800 {
            let swap = self.aperture_lane_mask::<N>(off)?;
            return Ok(std::array::from_fn(|k| {
                self.vram[(off as usize + (k ^ swap)) & 0x3f_ffff]
            }));
        }
        let i = Self::index(off, N)?;
        if (0x200..0x240).contains(&i) {
            return Err("read from a write-only Mach64 HOST_DATA register".into());
        }
        Ok(std::array::from_fn(|k| {
            if i + k == 0x338 {
                u8::from(self.host.is_some())
            } else if i + k == 0x33a {
                // GUI_STAT.FIFO_CNT. The synchronous engine consumes every
                // command write immediately, including incoming host data.
                COMMAND_FIFO_ENTRIES
            } else if i + k == 0x92 {
                self.pll[usize::from(self.registers[0x91] >> 2)]
            } else if i + k == 0xc1 {
                let value = self.palette[self.registers[0xc3] as usize][self.read_component];
                self.read_component = (self.read_component + 1) % 3;
                if self.read_component == 0 {
                    self.registers[0xc3] = self.registers[0xc3].wrapping_add(1);
                }
                value
            } else {
                self.registers[i + k]
            }
        }))
    }
    fn aperture_lane_mask<const N: usize>(&self, off: u32) -> Result<usize, String> {
        // On the GTB each 8 MiB aperture has a separate endian field in MEM_CNTL.
        // The 32-bit PCI bus splits a CPU xword into two dword transfers.
        let shift = if off & 0x80_0000 == 0 { 0 } else { 2 };
        match (self.registers[0xb3] >> shift) & 3 {
            0 => Ok(0),
            1 => Ok((N - 1).min(1)),
            2 => Ok((N - 1).min(3)),
            _ => Err(format!("reserved Mach64 aperture endian mode at +{off:#x}")),
        }
    }
    pub fn write<const N: usize>(&mut self, off: u32, bytes: [u8; N]) -> Result<(), String> {
        if (off & 0x7f_ffff) < 0x7f_f800 {
            let swap = self.aperture_lane_mask::<N>(off)?;
            for (k, b) in bytes.into_iter().enumerate() {
                let p = (off as usize + (k ^ swap)) & 0x3f_ffff;
                if self.vram[p] != b {
                    self.vram[p] = b;
                    self.generation = self.generation.wrapping_add(1);
                }
            }
            return Ok(());
        }
        let i = Self::index(off, N)?;
        for (j, b) in (i..).zip(bytes) {
            if matches!(j,0..=3|8..=11|0x14..=0x17|0x1c..=0x1f|0x60..=0x73|0xc2|0xc4..=0xc7|0xd0)
                && self.registers[j] != b
            {
                self.generation = self.generation.wrapping_add(1);
            }
            match j {
                0x12 | 0x13 => {}
                0x18 => {
                    self.registers[j] = (self.registers[j] & 0x14 & !b) | (b & 0x0a);
                }
                0xc0 => {
                    self.registers[j] = b;
                    self.write_component = 0;
                }
                0xc3 => {
                    self.registers[j] = b;
                    self.read_component = 0;
                }
                0xc1 => {
                    let v = &mut self.palette[self.registers[0xc0] as usize][self.write_component];
                    if *v != b {
                        *v = b;
                        self.generation = self.generation.wrapping_add(1);
                    }
                    self.write_component = (self.write_component + 1) % 3;
                    if self.write_component == 0 {
                        self.registers[0xc0] = self.registers[0xc0].wrapping_add(1);
                    }
                }
                0xe0..=0xe7 | 0x338..=0x33b => {}
                0x92 if self.registers[0x91] & 2 != 0 => {
                    self.pll[usize::from(self.registers[0x91] >> 2)] = b
                }
                0x92 => {}
                0xa2 => self.registers[j] = b & !0xa0,
                _ => self.registers[j] = b,
            }
        }
        self.engine_write(i, i + N)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gui_reports_free_fifo_entries_for_solaris_rectangle_fills() {
        let mut chip = Mach64::default();
        const STATUS: u32 = 0x7fff38;
        chip.write(STATUS, [0xff; 4]).unwrap();
        let status = u32::from_le_bytes(chip.read(STATUS).unwrap());
        // Xsun requests ten entries to start a fill, then all 48 before returning.
        assert!(status >= 10 << 16);
        assert_eq!(status, 48 << 16);
        assert_eq!(chip.read::<2>(STATUS + 2).unwrap(), [48, 0]);
        assert_eq!(chip.read::<1>(STATUS + 2).unwrap(), [48]);
    }

    #[test]
    fn big_endian_aperture_swaps_within_the_selected_pci_beat() {
        let mut chip = Mach64::default();
        let a = 0x80_0100;
        chip.write(0x7ffcb3, [2 << 2]).unwrap();
        chip.write(a, [1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        chip.write(a + 1, [9]).unwrap();
        chip.write(a + 2, [10, 11]).unwrap();
        assert_eq!(chip.read::<8>(0x100).unwrap(), [4, 9, 11, 10, 8, 7, 6, 5]);
        assert_eq!(chip.read::<2>(a + 2).unwrap(), [10, 11]);
        assert_eq!(chip.read::<1>(a + 1).unwrap(), [9]);
        assert_eq!(chip.read::<4>(a).unwrap(), [10, 11, 9, 4]);
    }
    #[test]
    fn indexed_xsun_pixels_and_independent_aperture_modes() {
        let mut chip = Mach64::default();
        let upper = 0x80_0100;
        let lower = 0x100;
        let pixels = [1, 2, 4, 8, 16, 32, 64, 128];
        // MEM_CNTL programmed by the Solaris 7 driver for the 8-bit greeter.
        chip.write(0x7ffcb0, 0x0032_10b7u32.to_le_bytes()).unwrap();
        chip.write(upper, pixels).unwrap();
        assert_eq!(chip.read::<8>(lower).unwrap(), pixels);
        chip.write(0x7ffcb3, [(1 << 2) | 2]).unwrap();
        assert_eq!(
            chip.read::<8>(upper).unwrap(),
            [2, 1, 8, 4, 32, 16, 128, 64]
        );
        assert_eq!(
            chip.read::<8>(lower).unwrap(),
            [8, 4, 2, 1, 128, 64, 32, 16]
        );
        chip.write(0x7ffcb3, [3 << 2]).unwrap();
        assert!(chip.write(upper, [0; 8]).is_err());
        assert_eq!(chip.read::<8>(lower).unwrap(), pixels);
    }
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

#[cfg(test)]
mod scanout_tests {
    use super::*;
    const REGS: u32 = 0x7ffc00;

    #[test]
    fn hardware_cursor_renders_all_four_codes_with_clipping_and_vram_stride() {
        let mut chip = Mach64::default();
        for (off, value) in [
            (0, 1 << 16),
            (8, 2 << 16),
            (0x14, 2 << 22),
            (0x1c, 0x0200_0200),
            (0x60, 0x1122_3300),
            (0x64, 0xaabb_ccff),
            (0x68, 0x100),
            (0x6c, 0x0001_0001),
            (0x70, 0x003e_0001),
            (0xd0, 0x80),
        ] {
            chip.write(REGS + off, u32::to_le_bytes(value)).unwrap();
        }
        chip.write(0x800, [0xe4, 0]).unwrap();
        chip.write(0x810, [0]).unwrap();
        let mut out = [0; 48];
        chip.pixels(&mut out);
        assert_eq!(&out[16..21], &[0, 0xaabbcc, 0, 0xffffff, 0x112233]);
        assert_eq!(out[33], 0x112233);
        let generation = chip.generation;
        chip.write(REGS + 0xd0, [0]).unwrap();
        assert_ne!(chip.generation, generation);
        chip.pixels(&mut out);
        assert_eq!(out, [0; 48]);
    }

    #[test]
    fn truecolor_scanout_uses_the_selected_depth_and_pixel_pitch() {
        for (mode, bytes, color) in [
            (3, vec![0x1f, 0x7c], 0xff00ff),
            (4, vec![0xe0, 0x07], 0x00ff00),
            (5, vec![0x56, 0x34, 0x12], 0x123456),
            (6, vec![0x56, 0x34, 0x12, 0xfc], 0x123456),
        ] {
            let mut chip = Mach64::default();
            chip.write(REGS + 8, (1u32 << 16).to_le_bytes()).unwrap();
            chip.write(REGS + 0x14, ((2u32 << 22) | 3).to_le_bytes())
                .unwrap();
            chip.write(REGS + 0x1c, (0x0200_0000u32 | mode << 8).to_le_bytes())
                .unwrap();
            for (k, &b) in bytes.iter().enumerate() {
                chip.write((24 + 16 * bytes.len() + k) as u32, [b]).unwrap();
            }
            let mut out = [0; 16];
            chip.pixels(&mut out);
            assert_eq!(chip.size(), Some((8, 2)));
            assert_eq!(out[0], 0);
            assert_eq!(out[8], color);
        }
    }
    #[test]
    fn indexed_scanout_uses_crtc_origin_pitch_dac_mask_and_rgb_triplets() {
        let mut chip = Mach64::default();
        chip.write(REGS, (1u32 << 16).to_le_bytes()).unwrap(); // 16 displayed pixels
        chip.write(REGS + 8, (1u32 << 16).to_le_bytes()).unwrap(); // two lines
        chip.write(REGS + 0x14, ((3u32 << 22) | 1).to_le_bytes())
            .unwrap(); // pitch 24, start 8
        chip.write(REGS + 0x1c, 0x03000200u32.to_le_bytes())
            .unwrap();
        chip.write(REGS + 0xc4, 0x100u32.to_le_bytes()).unwrap();
        chip.write(REGS + 0xc2, [0x0f]).unwrap();
        chip.write(REGS + 0xc0, [1]).unwrap();
        for c in [0xa1, 0xb2, 0xc3, 0x12, 0x34, 0x56] {
            chip.write(REGS + 0xc1, [c]).unwrap();
        }
        chip.write(8, [0x21, 0x32]).unwrap();
        chip.write(32, [0x32, 0x21]).unwrap();
        let mut pixels = [0; 32];
        assert_eq!(chip.size(), Some((16, 2)));
        chip.pixels(&mut pixels);
        assert_eq!(pixels[0..2], [0xa1b2c3, 0x123456]);
        assert_eq!(pixels[16..18], [0x123456, 0xa1b2c3]);
        let generation = chip.generation;
        chip.write(REGS + 0xc3, [2]).unwrap();
        assert_eq!(chip.read::<1>(REGS + 0xc1).unwrap(), [0x12]);
        assert_eq!(chip.read::<1>(REGS + 0xc1).unwrap(), [0x34]);
        assert_eq!(chip.generation, generation);
        chip.write(REGS + 0x1c, 0x03000240u32.to_le_bytes())
            .unwrap(); // CRTC display disable
        assert_eq!(chip.size(), None);
        assert_ne!(chip.generation, generation);
    }
}
