//! Sun TurboGX (CGsix): SBus FCode, 1 MiB indexed VRAM, BT458 and drawing registers.
//! Functional console subset and register provenance: docs/graphics.md.
use crate::bus::BusErrorKind;

pub const WIDTH: usize = 1152;
pub const HEIGHT: usize = 900;
pub const VRAM_BYTES: usize = 1024 * 1024;
pub const VRAM_OFFSET: u64 = 0x800000;
const FRAME_UNITS: u128 = 1472 * 943 * 1_000_000_000;
const VBLANK_UNITS: u128 = 1472 * HEIGHT as u128 * 1_000_000_000;

#[derive(Debug)]
pub struct Cgsix {
    rom: Vec<u32>,
    vram: Vec<u32>,
    palette: [[u8; 3]; 256],
    overlay: [[u8; 3]; 4],
    dac_address: u8,
    component: usize,
    dac_control: [u8; 4],
    fhc: u32,
    thc: [u32; 7],
    timing: [u32; 64],
    oscillator: u32,
    cursor_xy: u32,
    cursor: [u32; 64],
    fbc: [u32; 1024],
    tec: [u32; 1024],
    phase: u64,
    blink_frame: u8,
    vblank_pending: bool,
    generation: u64,
}

impl Cgsix {
    pub fn new(rom: &[u8]) -> Result<Self, BusErrorKind> {
        if rom.len() != 32768 {
            return Err(BusErrorKind::Unsupported(
                "CGsix requires a 32 KiB FCode ROM",
            ));
        }
        Ok(Self {
            rom: rom
                .chunks_exact(4)
                .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
                .collect(),
            vram: vec![0; VRAM_BYTES / 4],
            palette: [[0; 3]; 256],
            overlay: [[0; 3]; 4],
            dac_address: 0,
            component: 0,
            dac_control: [255, 0, 0x40, 0],
            fhc: 0x61b00800,
            thc: [0; 7],
            timing: [0; 64],
            oscillator: 0,
            cursor_xy: 0xffe0ffe0,
            cursor: [0; 64],
            fbc: [0; 1024],
            tec: [0; 1024],
            phase: 0,
            blink_frame: 0,
            vblank_pending: false,
            generation: 0,
        })
    }
    pub fn reset(&mut self) {
        self.fhc = 0x61b00800;
        self.thc = [0; 7];
        self.timing = [0; 64];
        self.oscillator = 0;
        self.fbc = [0; 1024];
        self.tec = [0; 1024];
        self.dac_address = 0;
        self.component = 0;
        self.dac_control = [255, 0, 0x40, 0];
        self.cursor_xy = 0xffe0ffe0;
        self.phase = 0;
        self.blink_frame = 0;
        self.vblank_pending = false;
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn scanout_key(&self) -> u64 {
        (u64::from(self.blink_frame) << 32) ^ self.generation
    }
    pub fn enabled(&self) -> bool {
        self.thc[6] & 0x400 != 0
    }
    pub fn pixels(&self, output: &mut [u32]) {
        assert_eq!(output.len(), WIDTH * HEIGHT);
        let command = self.dac_control[2];
        let (on, period) = match (command >> 4) & 3 {
            0 => (16, 64),
            1 => (16, 32),
            2 => (32, 64),
            _ => (64, 128),
        };
        let blink_off = self.blink_frame % period >= on;
        for (i, pixel) in output.iter_mut().enumerate() {
            let index = ((self.vram[i / 4] >> ((3 - i % 4) * 8)) as u8)
                & self.dac_control[0]
                & if blink_off { !self.dac_control[1] } else { 255 };
            let mut rgb = if command & 0x40 != 0 {
                self.palette[index as usize]
            } else {
                self.overlay[0]
            };
            let x = (i % WIDTH) as i32 - (self.cursor_xy >> 16) as i16 as i32;
            let y = (i / WIDTH) as i32 - self.cursor_xy as i16 as i32;
            if (0..32).contains(&x) && (0..32).contains(&y) {
                let bit = 31 - x;
                let mut index = ((self.cursor[y as usize] >> bit) & 1)
                    | (((self.cursor[y as usize + 32] >> bit) & 1) << 1);
                index &= u32::from(command & 3);
                if blink_off {
                    index &= !u32::from((command >> 2) & 3);
                }
                if index != 0 {
                    rgb = self.overlay[index as usize];
                }
            }
            *pixel = if self.enabled() {
                u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2])
            } else {
                0
            };
        }
    }
    pub(crate) fn advance_ns(&mut self, ns: u64) {
        let elapsed = u128::from(self.phase) + u128::from(ns) * 105_561_000;
        if (elapsed + FRAME_UNITS - VBLANK_UNITS) / FRAME_UNITS
            > (u128::from(self.phase) + FRAME_UNITS - VBLANK_UNITS) / FRAME_UNITS
        {
            self.vblank_pending = true;
        }
        self.blink_frame = ((u128::from(self.blink_frame) + elapsed / FRAME_UNITS) % 128) as u8;
        self.phase = (elapsed % FRAME_UNITS) as u64;
    }
    pub(crate) fn irq(&self) -> bool {
        self.vblank_pending && self.thc[6] & 0x20 != 0
    }
    pub(crate) fn read(&mut self, offset: u64, bytes: u64) -> Result<u32, BusErrorKind> {
        if offset % bytes != 0 {
            return Err(BusErrorKind::Misaligned);
        }
        let word = if offset < 32768 {
            self.rom[offset as usize / 4]
        } else if (VRAM_OFFSET..VRAM_OFFSET + VRAM_BYTES as u64).contains(&offset) {
            self.vram[(offset - VRAM_OFFSET) as usize / 4]
        } else if offset & !3 == 0x300000 {
            self.fhc
        } else {
            if bytes != 4 {
                return Err(BusErrorKind::Unsupported("narrow CGsix register access"));
            }
            return self.read_register(offset);
        };
        Ok(match bytes {
            1 => (word >> ((3 - offset % 4) * 8)) & 255,
            2 => (word >> ((2 - offset % 4) * 8)) & 65535,
            _ => word,
        })
    }
    pub(crate) fn write(&mut self, offset: u64, value: u32, mask: u32) -> Result<(), BusErrorKind> {
        if offset & 3 != 0 {
            return Err(BusErrorKind::Misaligned);
        }
        if (VRAM_OFFSET..VRAM_OFFSET + VRAM_BYTES as u64).contains(&offset) {
            let word = &mut self.vram[(offset - VRAM_OFFSET) as usize / 4];
            *word = (*word & !mask) | (value & mask);
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        if offset < 32768 {
            return Err(BusErrorKind::ReadOnly);
        }
        if mask != u32::MAX {
            return Err(BusErrorKind::Unsupported("masked CGsix register access"));
        }
        self.write_register(offset, value)?;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    fn dac_next(&mut self) {
        self.component += 1;
        if self.component == 3 {
            self.component = 0;
            self.dac_address = self.dac_address.wrapping_add(1);
        }
    }
    fn read_register(&mut self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0x200000 => {
                self.component = 0;
                Ok(u32::from(self.dac_address) << 24)
            }
            0x200004 => {
                let value = self.palette[self.dac_address as usize][self.component];
                self.dac_next();
                Ok(u32::from(value) << 24)
            }
            0x200008 if (4..8).contains(&self.dac_address) => {
                Ok(u32::from(self.dac_control[self.dac_address as usize - 4]) << 24)
            }
            0x20000c => {
                let value = self.overlay[self.dac_address as usize & 3][self.component];
                self.dac_next();
                Ok(u32::from(value) << 24)
            }
            0x300000 => Ok(self.fhc),
            0x280000 => Ok(self.oscillator),
            0x301090 | 0x301094 | 0x3010a0..=0x3010b0 | 0x3010c0..=0x3010cc => {
                Ok(self.timing[(offset - 0x301000) as usize / 4])
            }
            0x301800..=0x301818 => {
                if offset == 0x301818 {
                    Ok(self.thc[6]
                        | 0x10000
                        | if self.vblank_pending { 0x10 } else { 0 }
                        | if self.phase / (1472 * 1_000_000_000) >= HEIGHT as u64 {
                            0x100
                        } else {
                            0
                        })
                } else {
                    Ok(self.thc[(offset - 0x301800) as usize / 4])
                }
            }
            0x3018fc => Ok(self.cursor_xy),
            0x301900..=0x3019fc => Ok(self.cursor[(offset - 0x301900) as usize / 4]),
            0x700010 => Ok(0), // Synchronous engine: no operation remains busy.
            0x700004 => Ok(self.fbc[1]
                | if u128::from(self.phase) >= VBLANK_UNITS {
                    0x80000
                } else {
                    0
                }),
            0x700014 => {
                self.draw()?;
                Ok(0)
            }
            0x700018 => {
                self.blit()?;
                Ok(0)
            }
            0x700000..=0x700ffc => Ok(self.fbc[(offset - 0x700000) as usize / 4]),
            0x701000..=0x701ffc => Ok(self.tec[(offset - 0x701000) as usize / 4]),
            _ => Err(BusErrorKind::Unsupported("CGsix register")),
        }
    }
    fn write_register(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match offset {
            0x200000 => {
                self.dac_address = (value >> 24) as u8;
                self.component = 0;
            }
            0x200004 => {
                self.palette[self.dac_address as usize][self.component] = (value >> 24) as u8;
                self.dac_next();
            }
            0x200008 if (4..8).contains(&self.dac_address) => {
                self.dac_control[self.dac_address as usize - 4] = (value >> 24) as u8
            }
            // Shared TurboGX FCode also programs the BT467 pixel-clock command.
            // A 1 MiB TurboGX has a BT458, where address 9 is not decoded.
            0x200008 if self.dac_address == 9 => {}
            0x20000c => {
                self.overlay[self.dac_address as usize & 3][self.component] = (value >> 24) as u8;
                self.dac_next();
            }
            0x300000 => {
                if value & 0x2000 != 0 {
                    return Err(BusErrorKind::Unsupported(
                        "CGsix resolution or little-endian mode",
                    ));
                }
                if value & 0x8000 != 0 {
                    self.fbc = [0; 1024];
                    self.tec = [0; 1024];
                }
                self.fhc = (self.fhc & 0xfff00000) | (value & 0x000f3fff);
            }
            0x280000 => self.oscillator = value,
            0x301090 | 0x301094 | 0x3010a0..=0x3010b0 | 0x3010c0..=0x3010cc => {
                self.timing[(offset - 0x301000) as usize / 4] = value
            }
            0x301800..=0x301814 => self.thc[(offset - 0x301800) as usize / 4] = value,
            0x301818 => {
                if value & 0x400 != 0 && !self.enabled() && self.fhc & 0x1c00 != 0x0800 {
                    return Err(BusErrorKind::Unsupported("CGsix scanout resolution"));
                }
                if value & 0x10 != 0 {
                    self.vblank_pending = false;
                }
                self.thc[6] = value & 0x14ef;
            }
            0x3018fc => self.cursor_xy = value,
            0x301900..=0x3019fc => self.cursor[(offset - 0x301900) as usize / 4] = value,
            0x700004 => {
                // Zero in a mode field means leave that field unchanged.
                for shift in [7, 9, 11, 13, 15, 17, 20] {
                    let mask = 3 << shift;
                    if value & mask != 0 {
                        self.fbc[1] = (self.fbc[1] & !mask) | (value & mask);
                    }
                }
                if value & 0x40 != 0 {
                    self.fbc[1] = (self.fbc[1] & !0x30) | (value & 0x30);
                }
            }
            0x700010 | 0x700014 | 0x700018 => {}
            0x70001c => self.font(value)?,
            0x700108 => {
                // RASTEROP's two-bit selectors use zero to retain a field.
                // The Boolean truth table and rasterop/plot bits always load.
                for shift in [22, 24, 26, 28, 30] {
                    let mask = 3 << shift;
                    if value & mask != 0 {
                        self.fbc[0x108 / 4] = (self.fbc[0x108 / 4] & !mask) | (value & mask);
                    }
                }
                self.fbc[0x108 / 4] = (self.fbc[0x108 / 4] & !0x3ffff) | (value & 0x3ffff);
            }
            0x700800..=0x700918 => self.indexed_coordinate(offset, value)?,
            0x700000..=0x700ffc => self.fbc[(offset - 0x700000) as usize / 4] = value,
            0x701000..=0x701ffc => self.tec[(offset - 0x701000) as usize / 4] = value,
            _ => return Err(BusErrorKind::Unsupported("CGsix register")),
        }
        Ok(())
    }

    fn drawing_supported(&self) -> Result<(), BusErrorKind> {
        // Buffer selectors address the same physical store on this 1 MiB card.
        // Double-buffered TurboGX+ configurations are not implemented.
        let mode = self.fbc[1];
        let rop = self.fbc[0x108 / 4];
        if (mode >> 15) & 3 != 1
            || ![1, 2].contains(&((mode >> 17) & 3))
            || rop & 0x30000 != 0
            || self.fhc & 0x1c00 != 0x0800
        {
            return Err(BusErrorKind::Unsupported("CGsix drawing mode"));
        }
        Ok(())
    }

    fn pixel(&self, x: usize, y: usize) -> u8 {
        let i = y * WIDTH + x;
        (self.vram[i / 4] >> ((3 - i % 4) * 8)) as u8
    }

    // Destination clipping is inclusive; retain untouched planes and pixels.
    fn plot(&mut self, x: i64, y: i64, source: u8) {
        if !(0..WIDTH as i64).contains(&x)
            || !(0..HEIGHT as i64).contains(&y)
            || x < self.fbc[0xe0 / 4] as i32 as i64
            || y < self.fbc[0xe4 / 4] as i32 as i64
            || x > self.fbc[0xf0 / 4] as i32 as i64
            || y > self.fbc[0xf4 / 4] as i32 as i64
        {
            return;
        }
        self.plot_pixel(x, y, source);
    }

    fn plot_pixel(&mut self, x: i64, y: i64, source: u8) {
        let rop = self.fbc[0x108 / 4];
        let pixel_mask = match (rop >> 28) & 3 {
            1 => 0,
            3 => self.fbc[0x110 / 4],
            _ => u32::MAX,
        };
        if pixel_mask & (1 << (31 - x % 32)) == 0 {
            return;
        }
        let plane_mask = match rop >> 30 {
            1 => 0,
            3 => self.fbc[0x10c / 4] as u8,
            _ => 255,
        };
        let dst = self.pixel(x as usize, y as usize);
        let mut result = dst & !plane_mask;
        for bit in 0..8 {
            let selector =
                ((self.fbc[0x100 / 4] >> bit) & 1) * 2 + ((self.fbc[0x104 / 4] >> bit) & 1);
            let truth = (rop >> (selector * 4)) & 15;
            let inputs = ((source >> bit) & 1) * 2 + ((dst >> bit) & 1);
            result |= (((truth >> inputs) as u8 & 1) << bit) & plane_mask;
        }
        let i = y as usize * WIDTH + x as usize;
        let shift = (3 - i % 4) * 8;
        self.vram[i / 4] = (self.vram[i / 4] & !(255 << shift)) | (u32::from(result) << shift);
    }

    fn font(&mut self, value: u32) -> Result<(), BusErrorKind> {
        self.drawing_supported()?;
        let x = self.fbc[0x80 / 4] as i32 as i64;
        let end = self.fbc[0x90 / 4] as i32 as i64;
        let bits = if (self.fbc[1] >> 17) & 3 == 1 { 8 } else { 1 };
        if end - x + 1 > 32 / bits {
            return Err(BusErrorKind::Unsupported("CGsix font span"));
        }
        let y = self.fbc[0x84 / 4] as i32 as i64;
        for n in 0..(end - x + 1).max(0) {
            let source = if bits == 8 {
                (value >> (24 - n * 8)) as u8
            } else if value & (1 << (31 - n)) != 0 {
                255
            } else {
                0
            };
            self.plot(x + n, y, source);
        }
        for (coord, inc) in [(0x80, 0xd0), (0x90, 0xd0), (0x84, 0xd4), (0x94, 0xd4)] {
            self.fbc[coord / 4] = self.fbc[coord / 4].wrapping_add(self.fbc[inc / 4]);
        }
        self.fbc[0x1c / 4] = value;
        Ok(())
    }

    fn pattern(&self, x: i64, y: i64) -> u8 {
        match (self.fbc[0x108 / 4] >> 26) & 3 {
            1 => 0,
            3 => {
                let align = self.fbc[0x11c / 4];
                let px = (x - i64::from(align >> 16)) & 15;
                let py = (y - i64::from(align & 65535)) & 15;
                let pattern = self.fbc[0x120 / 4 + py as usize / 2];
                if pattern & (1 << (31 - (py % 2) * 16 - px)) != 0 {
                    255
                } else {
                    0
                }
            }
            _ => 255,
        }
    }

    // Sun FBC Reference Manual 3.0, section 3.2/table 3-1. These are
    // coordinate aliases, not drawing FIFOs. All primitives share X0..Y3.
    fn indexed_coordinate(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        let register = (offset - 0x700800) as usize;
        let kind = register / 0x40;
        let component = register & 0x2f;
        if kind > 4 || component > 4 {
            return Err(BusErrorKind::Unsupported("CGsix indexed coordinate"));
        }
        let index = ((self.fbc[1] >> 4) & 3) as usize;
        let relative = register & 0x10 != 0;
        let base = if relative {
            self.fbc[(0x80 + ((index + 3) & 3) * 16 + component) / 4]
        } else {
            self.fbc[(0xc0 + component) / 4]
        };
        let coordinate = base.wrapping_add(value);
        let count = if kind == 4 { 2 } else { 4 - kind };
        for n in 0..count {
            self.fbc[(0x80 + ((index + n) & 3) * 16 + component) / 4] = coordinate;
        }
        // X advances every primitive; rectangle Y also advances the index.
        if component == 0 || kind == 4 {
            self.fbc[1] = (self.fbc[1] & !0x30) | (((index as u32 + 1) & 3) << 4);
        }
        Ok(())
    }

    fn draw(&mut self) -> Result<(), BusErrorKind> {
        self.drawing_supported()?;
        let points: [(i64, i64); 4] = std::array::from_fn(|i| {
            (
                self.fbc[0x80 / 4 + i * 4] as i32 as i64,
                self.fbc[0x84 / 4 + i * 4] as i32 as i64,
            )
        });
        let min_x = points.iter().map(|p| p.0).min().unwrap();
        let max_x = points.iter().map(|p| p.0).max().unwrap();
        let min_y = points.iter().map(|p| p.1).min().unwrap();
        let max_y = points.iter().map(|p| p.1).max().unwrap();
        let overlapping = (self.fbc[0x108 / 4] >> 24) & 3 == 1;
        let start = *points.iter().min().unwrap();
        let end = *points.iter().max().unwrap();
        let collinear = points.iter().all(|p| {
            i128::from(p.0 - start.0) * i128::from(end.1 - start.1)
                == i128::from(p.1 - start.1) * i128::from(end.0 - start.0)
        });
        if collinear {
            // Nonoverlapping degenerate polygons have no area (section 5.2).
            if overlapping {
                self.draw_line(start, end);
            }
        } else if [
            (min_x, min_y),
            (max_x, min_y),
            (max_x, max_y),
            (min_x, max_y),
        ]
        .iter()
        .all(|p| points.contains(p))
            && (0..4).all(|i| {
                points[i].0 == points[(i + 1) % 4].0 || points[i].1 == points[(i + 1) % 4].1
            })
        {
            let extra = i64::from(overlapping);
            for y in min_y.max(0)..(max_y + extra).min(HEIGHT as i64) {
                for x in min_x.max(0)..(max_x + extra).min(WIDTH as i64) {
                    self.plot(x, y, self.pattern(x, y));
                }
            }
        } else {
            self.draw_polygon(&points, overlapping)?;
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    fn draw_line(&mut self, start: (i64, i64), end: (i64, i64)) {
        Self::line_pixels(start, end, |x, y| self.plot(x, y, self.pattern(x, y)));
    }

    fn line_pixels(mut start: (i64, i64), mut end: (i64, i64), mut pixel: impl FnMut(i64, i64)) {
        let steep = (end.1 - start.1).abs() > (end.0 - start.0).abs();
        if steep {
            start = (start.1, start.0);
            end = (end.1, end.0);
        }
        if start.0 > end.0 {
            std::mem::swap(&mut start, &mut end);
        }
        let length = end.0 - start.0;
        let limit = if steep { HEIGHT } else { WIDTH } as i64;
        // Bound work to the screen, preserving the phase of a clipped line.
        for major in start.0.max(0)..=end.0.min(limit - 1) {
            let minor = if length == 0 {
                start.1
            } else {
                let numerator = 2 * i128::from(end.1 - start.1) * i128::from(major - start.0)
                    + i128::from(length);
                start.1 + numerator.div_euclid(2 * i128::from(length)) as i64
            };
            let (x, y) = if steep {
                (minor, major)
            } else {
                (major, minor)
            };
            // Midpoint ties round toward screen right/down, independent of
            // endpoint order (section 5.2). Lines obey the same clip window.
            pixel(x, y);
        }
    }

    fn draw_polygon(
        &mut self,
        points: &[(i64, i64); 4],
        overlapping: bool,
    ) -> Result<(), BusErrorKind> {
        // Section 5.2: nonoverlap uses ideal edges, excluding right/bottom;
        // overlap is the union of the interior and balanced boundary lines.
        // Collect coverage before applying the ROP so shared vertices and
        // edge/interior intersections affect each destination pixel only once.
        let mut rows = [None; HEIGHT];
        for (y, row) in rows.iter_mut().enumerate() {
            let y = y as i64;
            let mut intersections = [(0i128, 1i128); 4];
            let mut count = 0;
            for edge in 0..4 {
                let (mut a, mut b) = (points[edge], points[(edge + 1) % 4]);
                if a.1 > b.1 {
                    std::mem::swap(&mut a, &mut b);
                }
                if y < a.1 || y >= b.1 {
                    continue;
                }
                let dy = i128::from(b.1 - a.1);
                let numerator = i128::from(a.0) * dy + i128::from(b.0 - a.0) * i128::from(y - a.1);
                intersections[count] = (numerator, dy);
                count += 1;
            }
            if count == 0 {
                continue;
            }
            if count != 2 {
                // Section 5.1 limits hardware DRAW to two active edges.
                return Err(BusErrorKind::Unsupported(
                    "CGsix polygon with more than two active edges",
                ));
            }
            let [(mut left, mut ld), (mut right, mut rd), ..] = intersections;
            if left * rd > right * ld {
                std::mem::swap(&mut left, &mut right);
                std::mem::swap(&mut ld, &mut rd);
            }
            let start = -(-left).div_euclid(ld);
            let end = if overlapping {
                right.div_euclid(rd)
            } else {
                -(-right).div_euclid(rd) - 1
            };
            if start <= end {
                *row = Some((start as i64, end as i64));
            }
        }
        if overlapping {
            for edge in 0..4 {
                Self::line_pixels(points[edge], points[(edge + 1) % 4], |x, y| {
                    if !(0..HEIGHT as i64).contains(&y) {
                        return;
                    }
                    let row = &mut rows[y as usize];
                    *row = Some(match *row {
                        Some((left, right)) => (left.min(x), right.max(x)),
                        None => (x, x),
                    });
                });
            }
        }
        for (y, row) in rows.into_iter().enumerate() {
            if let Some((left, right)) = row {
                for x in left.max(0)..=right.min(WIDTH as i64 - 1) {
                    self.plot(x, y as i64, self.pattern(x, y as i64));
                }
            }
        }
        Ok(())
    }

    fn blit(&mut self) -> Result<(), BusErrorKind> {
        self.drawing_supported()?;
        let c: [i64; 8] =
            [0x80, 0x84, 0x90, 0x94, 0xa0, 0xa4, 0xb0, 0xb4].map(|n| self.fbc[n / 4] as i32 as i64);
        let [sx, sy, ex, ey, dx, dy, fx, fy] = c;
        let w = ex - sx + 1;
        let h = ey - sy + 1;
        if w <= 0 || h <= 0 {
            return Ok(());
        }
        if w != fx - dx + 1
            || h != fy - dy + 1
            || sx < 0
            || sy < 0
            || ex >= WIDTH as i64
            || ey >= HEIGHT as i64
        {
            return Err(BusErrorKind::Unsupported("CGsix blit geometry"));
        }
        // Snapshot the source to preserve overlapping scrolls in either direction.
        let mut source = Vec::with_capacity((w * h) as usize);
        for y in sy..=ey {
            for x in sx..=ex {
                source.push(self.pixel(x as usize, y as usize));
            }
        }
        for y in 0..h {
            for x in 0..w {
                self.plot(dx + x, dy + y, source[(y * w + x) as usize]);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
}
