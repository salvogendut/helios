//! Compare and vblank interrupts are latched independently of their enables.
//! Use the programmed line length and pixel PLL for noninterlaced modes.
use super::Mach64;

impl Mach64 {
    fn line_ns(&self) -> u64 {
        let c = usize::from(self.registers[0x90] & 3);
        let fb = u64::from(self.pll[7 + c]);
        let reference = u64::from(self.pll[2]);
        let div = (self.pll[6] >> (2 * c) & 3) | ((self.pll[0xb] >> (4 + c) & 1) << 2);
        let post = [1u64, 2, 4, 8, 3, 0, 6, 12][usize::from(div)];
        let clocks = u64::from((self.reg(0) & 0x1ff) + 1) * 8;
        if fb == 0 || reference == 0 || post == 0 {
            return 0;
        }
        (clocks * 1_000_000_000 * reference * post).div_ceil(2 * 14_318_184 * fb)
    }

    fn scan_lines(&self) -> u64 {
        u64::from((self.reg(8) & 0x7ff) + 1)
    }
    fn schedule_raster(&mut self, now: u64) {
        self.raster_period = self.line_ns();
        let lines = self.scan_lines();
        let line = self.raster_period;
        let enabled = self.reg(0x1c) & (1 << 25) != 0;
        let deadline = |n: u64| {
            if !enabled || line == 0 || n >= lines {
                return u64::MAX;
            }
            let frame = line * lines;
            let pos = now % frame;
            let offset = n * line;
            now.saturating_add(if offset > pos {
                offset - pos
            } else {
                frame - pos + offset
            })
        };
        self.raster_events = [
            deadline(u64::from((self.reg(8) >> 16 & 0x7ff) + 1)),
            deadline(u64::from(self.reg(0x10) & 0x7ff)),
        ];
    }
    pub(in crate::ultra5) fn raster_deadline(&self) -> u64 {
        self.raster_events[0].min(self.raster_events[1])
    }
    pub(in crate::ultra5) fn video_interrupt(&mut self, now: u64) -> bool {
        if now >= self.raster_deadline() {
            for (i, t) in self.raster_events.into_iter().enumerate() {
                if now >= t {
                    self.registers[0x18] |= [4, 16][i];
                }
            }
            self.schedule_raster(now);
        }
        let bits = self.registers[0x18];
        bits & (bits >> 1) & 10 != 0
    }
    pub(in crate::ultra5) fn read_at<const N: usize>(
        &mut self,
        off: u32,
        now: u64,
    ) -> Result<[u8; N], String> {
        if (0x7ffc10..0x7ffc14).contains(&off) || (0x7ffc18..0x7ffc1c).contains(&off) {
            self.video_interrupt(now);
            let line = now.checked_div(self.raster_period).unwrap_or(0) % self.scan_lines();
            let current = line as u16;
            self.registers[0x12..0x14].copy_from_slice(&current.to_le_bytes());
            let blank = (u32::from(current) > (self.reg(8) >> 16 & 0x7ff)) as u8;
            self.registers[0x18] =
                (self.registers[0x18] & !0x61) | blank | ((current as u8 & 1) << 5);
        }
        self.read(off)
    }
    pub(in crate::ultra5) fn write_at<const N: usize>(
        &mut self,
        off: u32,
        data: [u8; N],
        now: u64,
    ) -> Result<(), String> {
        let raster = matches!(off,0x7ffc00..=0x7ffc13 | 0x7ffc18..=0x7ffc1f | 0x7ffc90..=0x7ffc93);
        if raster {
            self.video_interrupt(now);
        }
        self.write(off, data)?;
        if raster {
            self.schedule_raster(now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Mach64;

    #[test]
    fn compare_and_blank_latch_rearm_and_acknowledge_independently() {
        const REGS: u32 = 0x7ffc00;
        let mut c = Mach64::default();
        c.pll[2] = 1;
        c.pll[7] = 1;
        let write =
            |chip: &mut Mach64, i, v: u32, t| chip.write_at(REGS + i, v.to_le_bytes(), t).unwrap();
        write(&mut c, 8, (1 << 16) | 3, 0); // 4 total lines, 2 visible
        write(&mut c, 0x10, 1, 0);
        write(&mut c, 0x1c, 1 << 25, 0);
        assert_eq!(c.raster_deadline(), 280);
        assert!(!c.video_interrupt(280)); // masked compare still latches
        assert_eq!(c.read_at::<1>(REGS + 0x18, 280).unwrap()[0] & 0x14, 0x10);
        write(&mut c, 0x18, 0xa, 280);
        assert!(c.video_interrupt(280));
        write(&mut c, 0x18, 0x1a, 280);
        assert!(!c.video_interrupt(280));
        assert_eq!(
            c.read_at::<2>(REGS + 0x12, 560).unwrap(),
            2u16.to_le_bytes()
        );
        assert!(c.video_interrupt(560));
        assert_eq!(c.read_at::<1>(REGS + 0x18, 560).unwrap()[0] & 0x15, 5);
        write(&mut c, 0x18, 0xe, 560);
        assert_eq!(c.raster_deadline(), 1400);
        assert!(!c.video_interrupt(1399));
        assert!(c.video_interrupt(1400));
    }
}
