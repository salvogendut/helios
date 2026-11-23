//! Integer rectangle path of the ATI drawing engine. Coordinates are in pixels,
//! offsets in eight-byte units, and the pitch field in groups of eight pixels.
use super::Mach64;

#[derive(Default, Clone, Copy)]
struct Surface {
    base: usize,
    pitch: usize,
    bytes: usize,
}

impl Surface {
    fn new(layout: u32, format: u32) -> Result<Self, String> {
        let bytes = match format {
            2 => 1,
            3 | 4 => 2,
            5 => 3,
            6 => 4,
            _ => return Err(format!("unsupported Mach64 surface depth {format}")),
        };
        Ok(Self {
            base: (layout as usize & 0xfffff) * 8,
            pitch: (layout >> 22) as usize * 8,
            bytes,
        })
    }
    fn address(self, x: i32, y: i32) -> usize {
        self.base.wrapping_add(
            (y as usize)
                .wrapping_mul(self.pitch)
                .wrapping_add(x as usize)
                .wrapping_mul(self.bytes),
        )
    }
}

fn mix_pixel(op: u32, s: u32, d: u32) -> Result<u32, String> {
    Ok(match op {
        0 => !d,
        1 => 0,
        2 => u32::MAX,
        3 => d,
        4 => !s,
        5 => s ^ d,
        6 => !(s ^ d),
        7 => s,
        8 => !(s & d),
        9 => !s | d,
        10 => s | !d,
        11 => s | d,
        12 => s & d,
        13 => s & !d,
        14 => !s & d,
        15 => !(s | d),
        _ => return Err(format!("unsupported Mach64 pixel mix {op:#x}")),
    })
}

#[derive(Default, Clone, Copy)]
struct Paint {
    surface: Surface,
    mask: u32,
    compare: u32,
    compare_mask: u32,
    compare_value: u32,
}

impl Paint {
    fn new(chip: &Mach64) -> Result<Self, String> {
        let compare = chip.reg(0x308);
        if !matches!(compare & 7, 0 | 1 | 4 | 5) {
            return Err(format!("unsupported Mach64 color compare {compare:#x}"));
        }
        Ok(Self {
            surface: Surface::new(chip.reg(0x100), chip.reg(0x2d0) & 15)?,
            mask: chip.reg(0x2c8),
            compare,
            compare_mask: chip.reg(0x304),
            compare_value: chip.reg(0x300) & chip.reg(0x304),
        })
    }
    fn pixel(self, chip: &mut Mach64, x: i32, y: i32, op: u32, source: u32) -> Result<(), String> {
        let d = chip.read_pixel(self.surface, x, y);
        let component = if self.compare & (1 << 24) != 0 {
            source
        } else {
            d
        };
        let equal = component & self.compare_mask == self.compare_value;
        if match self.compare & 7 {
            1 => true,
            4 => !equal,
            5 => equal,
            _ => false,
        } {
            return Ok(());
        }
        let out = (d & !self.mask) | (mix_pixel(op, source, d)? & self.mask);
        let address = self.surface.address(x, y);
        for (k, b) in out.to_le_bytes()[..self.surface.bytes].iter().enumerate() {
            let index = address.wrapping_add(k) & (chip.vram.len() - 1);
            chip.vram[index] = *b;
        }
        Ok(())
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct HostRectangle {
    paint: Paint,
    color: [u32; 2],
    mix: [u32; 2],
    origin: [i32; 2],
    direction: [i32; 2],
    clip: [i32; 4],
    size: [u32; 2],
    column: u32,
    row: u32,
    lsb_first: bool,
    byte_align: bool,
}

impl HostRectangle {
    fn new(chip: &Mach64) -> Result<Self, String> {
        let mode = chip.reg(0x2d8);
        let color = |selector: u32| match selector & 7 {
            0 => Ok(chip.reg(0x2c0)),
            1 => Ok(chip.reg(0x2c4)),
            _ => Err(format!("unsupported Mach64 host color source {mode:#x}")),
        };
        let mix = [chip.reg(0x2d4) & 31, chip.reg(0x2d4) >> 16 & 31];
        for op in mix {
            mix_pixel(op, 0, 0)?;
        }
        let coordinate = |i| i32::from(chip.reg(i) as i16);
        Ok(Self {
            paint: Paint::new(chip)?,
            color: [color(mode)?, color(mode >> 8)?],
            mix,
            origin: [coordinate(0x104), coordinate(0x108)],
            direction: [0, 1].map(|b| {
                if chip.reg(0x130) & (1 << b) != 0 {
                    1
                } else {
                    -1
                }
            }),
            clip: [0x2a0, 0x2a4, 0x2ac, 0x2b0].map(coordinate),
            size: [chip.reg(0x110) & 0x1fff, chip.reg(0x114) & 0x7fff],
            column: 0,
            row: 0,
            lsb_first: chip.reg(0x2d0) & (1 << 24) != 0,
            byte_align: chip.reg(0x240) & 1 != 0,
        })
    }
    fn byte(&mut self, chip: &mut Mach64, value: u8) -> Result<(), String> {
        for i in 0..8 {
            let bit = usize::from((value >> if self.lsb_first { i } else { 7 - i }) & 1);
            let x = self.origin[0] + self.direction[0] * self.column as i32;
            let y = self.origin[1] + self.direction[1] * self.row as i32;
            if (self.clip[0]..=self.clip[1]).contains(&x)
                && (self.clip[2]..=self.clip[3]).contains(&y)
            {
                self.paint
                    .pixel(chip, x, y, self.mix[bit], self.color[bit])?;
            }
            self.column += 1;
            if self.column == self.size[0] {
                self.column = 0;
                self.row += 1;
                if self.row == self.size[1] || self.byte_align {
                    break;
                }
            }
        }
        Ok(())
    }
}

impl Mach64 {
    fn set_reg(&mut self, off: usize, value: u32) {
        self.registers[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn register_pair(&mut self, written: usize, low: usize, high: usize, pair: usize) {
        if written == pair {
            let v = self.reg(pair);
            self.set_reg(low, v & 0xffff);
            self.set_reg(high, v >> 16);
        } else if written == low || written == high {
            self.set_reg(pair, (self.reg(low) & 0xffff) | self.reg(high) << 16);
        }
    }

    pub(super) fn engine_write(&mut self, off: usize, end: usize) -> Result<(), String> {
        let index = off & !3;
        match index {
            0x104..=0x10c | 0x134 | 0x2e8 => {
                if index == 0x134 {
                    self.set_reg(0x10c, self.reg(index));
                }
                if index == 0x2e8 {
                    self.set_reg(0x10c, self.reg(index).rotate_left(16));
                }
                self.register_pair(
                    if matches!(index, 0x134 | 0x2e8) {
                        0x10c
                    } else {
                        index
                    },
                    0x108,
                    0x104,
                    0x10c,
                );
                self.set_reg(0x134, self.reg(0x10c));
                self.set_reg(0x2e8, self.reg(0x10c).rotate_left(16));
            }
            0x110..=0x118 | 0x2ec => {
                if index == 0x2ec {
                    self.set_reg(0x118, self.reg(index).rotate_left(16));
                }
                self.register_pair(
                    if index == 0x2ec { 0x118 } else { index },
                    0x114,
                    0x110,
                    0x118,
                );
                self.set_reg(0x2ec, self.reg(0x118).rotate_left(16));
                if index != 0x114 && end >= index + 2 && self.reg(0xd0) & 0x100 != 0 {
                    self.rectangle()?;
                }
            }
            0x120 if end >= index + 2 && self.reg(0xd0) & 0x100 != 0 => {
                if self.reg(0x120) & (1 << 31) == 0 {
                    self.line()?;
                }
            }
            0x200..=0x23c => {
                if let Some(mut transfer) = self.host.take() {
                    for i in off..end {
                        transfer.byte(self, self.registers[i])?;
                        if transfer.row == transfer.size[1] {
                            break;
                        }
                    }
                    if transfer.row < transfer.size[1] {
                        self.host = Some(transfer);
                    }
                    self.generation = self.generation.wrapping_add(1);
                }
            }
            0x330 => {
                let value = self.reg(0x330);
                self.set_reg(0x130, value & 0xffff);
                self.set_reg(0x1b4, value >> 16 & 255);
                self.set_reg(0x288, value >> 24 & 7);
                self.set_reg(0x240, value >> 28 & 3);
            }
            0x130 | 0x1b4 | 0x288 | 0x240 => {
                let value = (self.reg(0x130) & 0xffff)
                    | ((self.reg(0x1b4) & 255) << 16)
                    | ((self.reg(0x288) & 7) << 24)
                    | ((self.reg(0x240) & 3) << 28);
                self.set_reg(0x330, value);
            }
            0x184..=0x18c => self.register_pair(index, 0x188, 0x184, 0x18c),
            0x190..=0x198 => self.register_pair(index, 0x194, 0x190, 0x198),
            0x2a0..=0x2a8 => self.register_pair(index, 0x2a0, 0x2a4, 0x2a8),
            0x2ac..=0x2b4 => self.register_pair(index, 0x2ac, 0x2b0, 0x2b4),
            _ => {}
        }
        Ok(())
    }

    fn read_pixel(&self, surface: Surface, x: i32, y: i32) -> u32 {
        let a = surface.address(x, y);
        let mut bytes = [0; 4];
        for (i, v) in bytes[..surface.bytes].iter_mut().enumerate() {
            *v = self.vram[(a.wrapping_add(i)) & (self.vram.len() - 1)];
        }
        u32::from_le_bytes(bytes)
    }
    fn line(&mut self) -> Result<(), String> {
        let ctrl = self.reg(0x130);
        let mode = self.reg(0x2d8);
        if ctrl & 0x40 != 0 || mode >> 16 & 3 != 0 || mode >> 8 & 7 > 1 {
            return Err(format!(
                "unsupported Mach64 line source {mode:#x}, control {ctrl:#x}"
            ));
        }
        let paint = Paint::new(self)?;
        let color = self.reg(if mode & 0x100 == 0 { 0x2c0 } else { 0x2c4 });
        let op = self.reg(0x2d4) >> 16 & 31;
        mix_pixel(op, 0, 0)?;
        let coord = |a| i32::from(self.reg(a) as i16);
        let (mut x, mut y) = (coord(0x104), coord(0x108));
        let clip_x = coord(0x2a0)..=coord(0x2a4);
        let clip_y = coord(0x2ac)..=coord(0x2b0);
        let (dx, dy) = (
            if ctrl & 1 == 0 { -1 } else { 1 },
            if ctrl & 2 == 0 { -1 } else { 1 },
        );
        let signed = |v: u32| ((v << 14) as i32) >> 14;
        let mut err = signed(self.reg(0x124));
        let inc = signed(self.reg(0x128));
        let dec = signed(self.reg(0x12c));
        let limit = self.reg(0x120) & 0x7fff;
        for i in 0..limit {
            if (i + 1 < limit || ctrl & 0x20 != 0) && clip_x.contains(&x) && clip_y.contains(&y) {
                paint.pixel(self, x, y, op, color)?;
            }
            let diagonal = err > 0 || (err == 0 && ctrl & (1 << 11) == 0);
            if ctrl & 4 == 0 || diagonal {
                x += dx;
            }
            if ctrl & 4 != 0 || diagonal {
                y += dy;
            }
            err = signed((err + if diagonal { dec } else { inc }) as u32);
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    fn rectangle(&mut self) -> Result<(), String> {
        let mode = self.reg(0x2d8);
        let kind = (mode >> 8) & 7;
        let width = (self.reg(0x110) & 0x1fff) as i32;
        let height = (self.reg(0x114) & 0x7fff) as i32;
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.host = None;
        if mode >> 16 & 3 == 2 && self.reg(0x2d0) >> 16 & 15 == 0 {
            self.host = Some(HostRectangle::new(self)?);
            return Ok(());
        }
        if mode >> 16 & 3 != 0 || !matches!(kind, 0 | 1 | 3) {
            return Err(format!("unsupported Mach64 rectangle source {mode:#x}"));
        }
        let format = self.reg(0x2d0);
        let paint = Paint::new(self)?;
        let src = if kind == 3 {
            Some(Surface::new(self.reg(0x180), format >> 8 & 15)?)
        } else {
            None
        };
        let dx = if self.reg(0x130) & 1 != 0 { 1 } else { -1 };
        let dy = if self.reg(0x130) & 2 != 0 { 1 } else { -1 };
        let x0 = (self.reg(0x104) as i16) as i32;
        let y0 = (self.reg(0x108) as i16) as i32;
        let sx = (self.reg(0x184) as i16) as i32;
        let sy = (self.reg(0x188) as i16) as i32;
        let clip = |start: i32, step: i32, count: i32, lo: u32, hi: u32| {
            let (lo, hi) = ((lo as i16) as i32, (hi as i16) as i32);
            let (a, b) = if step > 0 {
                (lo - start, hi - start)
            } else {
                (start - hi, start - lo)
            };
            a.max(0)..(b + 1).min(count)
        };
        let xs = clip(x0, dx, width, self.reg(0x2a0), self.reg(0x2a4));
        let ys = clip(y0, dy, height, self.reg(0x2ac), self.reg(0x2b0));
        let op = self.reg(0x2d4) >> 16 & 31;
        mix_pixel(op, 0, 0)?;
        let color = self.reg(if kind == 0 { 0x2c0 } else { 0x2c4 });
        for j in ys {
            for i in xs.clone() {
                let (x, y) = (x0 + dx * i, y0 + dy * j);
                let s = src.map_or(color, |surface| {
                    self.read_pixel(surface, sx + dx * i, sy + dy * j)
                });
                paint.pixel(self, x, y, op, s)?;
            }
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Mach64;
    #[test]
    fn bresenham_signed_error_tie_and_last_pixel_follow_dst_control() {
        let mut chip = Mach64::default();
        let write =
            |c: &mut Mach64, i: u32, v: u32| c.write(0x7ffc00 + i, v.to_le_bytes()).unwrap();
        for (i, v) in [
            (0xd0, 0x100),
            (0x100, 1 << 22),
            (0x2d0, 2),
            (0x2d8, 0x100),
            (0x2d4, 7 << 16),
            (0x2c4, 0x33),
            (0x2c8, 255),
            (0x2a8, 7 << 16),
            (0x2b4, 7 << 16),
            (0x130, 0x23),
            (0x128, 2),
            (0x12c, 0x3fffc),
            (0x124, 0x3ffff),
            (0x120, 4),
        ] {
            write(&mut chip, i, v);
        }
        assert_eq!(chip.read::<4>(0).unwrap(), [0x33, 0x33, 0, 0]);
        assert_eq!(chip.read::<4>(8).unwrap(), [0, 0, 0x33, 0x33]);
        write(&mut chip, 0x10c, (1 << 16) | 2);
        write(&mut chip, 0x130, 7); // y-major, leave last pixel open
        write(&mut chip, 0x124, 0);
        write(&mut chip, 0x120, 3);
        assert_eq!(chip.read::<4>(16).unwrap(), [0, 0x33, 0, 0]);
        assert_eq!(chip.read::<4>(24).unwrap(), [0, 0, 0x33, 0]);
        assert_eq!(chip.read::<4>(32).unwrap(), [0; 4]);
    }

    #[test]
    fn host_monochrome_expands_addressed_bytes_and_consumes_clipped_pixels() {
        let mut chip = Mach64::default();
        let write =
            |c: &mut Mach64, i: u32, v: u32| c.write(0x7ffc00 + i, v.to_le_bytes()).unwrap();
        for (i, v) in [
            (0xd0, 0x100),
            (0x100, 2 << 22),
            (0x130, 3),
            (0x240, 1),
            (0x2d0, 2),
            (0x2d8, 0x20100),
            (0x2d4, 0x70007),
            (0x2c0, 1),
            (0x2c4, 6),
            (0x2c8, 255),
            (0x2a8, 8 << 16 | 1),
            (0x2b4, 1 << 16),
            (0x118, 10 << 16 | 2),
        ] {
            write(&mut chip, i, v);
        }
        assert_eq!(chip.read::<1>(0x7fff38).unwrap(), [1]);
        chip.write(0x7ffe00, [0xa1, 0x80, 0x49, 0]).unwrap();
        assert_eq!(chip.read::<1>(0x7fff38).unwrap(), [0]);
        assert_eq!(chip.read::<8>(0).unwrap(), [0, 1, 6, 1, 1, 1, 1, 6]);
        assert_eq!(chip.read::<2>(8).unwrap(), [6, 0]);
        assert_eq!(chip.read::<8>(16).unwrap(), [0, 6, 1, 1, 6, 1, 1, 6]);
        write(&mut chip, 0x2d0, (1 << 24) | 2);
        write(&mut chip, 0x118, 8 << 16 | 1);
        chip.write(0x7ffe1c, [2]).unwrap();
        assert_eq!(chip.read::<8>(0).unwrap(), [0, 6, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn fill_copy_and_plane_mask_follow_the_scissors_and_programmed_direction() {
        let mut c = Mach64::default();
        let reg =
            |chip: &mut Mach64, a: u32, v: u32| chip.write(0x7ffc00 + a, v.to_le_bytes()).unwrap();
        for (a, v) in [
            (0xd0, 0x100),
            (0x100, 2 << 22),
            (0x180, 2 << 22),
            (0x130, 3),
            (0x2d0, 0x202),
            (0x2d8, 0x100),
            (0x2d4, 7 << 16),
            (0x2c4, 0xc3),
            (0x2c8, 0x0f),
            (0x2a8, (9 << 16) | 2),
            (0x2b4, 6 << 16),
        ] {
            reg(&mut c, a, v);
        }
        c.write(16, [0xa0; 8]).unwrap();
        reg(&mut c, 0x10c, 1);
        reg(&mut c, 0x118, (5 << 16) | 1);
        assert_eq!(
            c.read::<8>(16).unwrap(),
            [0xa0, 0xa0, 0xa3, 0xa3, 0xa3, 0xa0, 0xa0, 0xa0]
        );
        c.write(16, [0, 1, 2, 3, 4, 5, 6, 7]).unwrap();
        reg(&mut c, 0x2c8, 0xff);
        reg(&mut c, 0x2d8, 0x300);
        reg(&mut c, 0x130, 2);
        reg(&mut c, 0x18c, (4 << 16) | 1);
        reg(&mut c, 0x2e8, (1 << 16) | 6);
        reg(&mut c, 0x2ec, (1 << 16) | 5);
        assert_eq!(c.read::<8>(16).unwrap(), [0, 1, 0, 1, 2, 3, 4, 7]);
    }
}
