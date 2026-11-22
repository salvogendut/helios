//! Ultra 5 reference population: two dual-rank 256 MiB DIMMs in pair 2.
//! Sun UltraSPARC-IIi User's Manual §§6.2.1, 7.2–3 and Appendix A.10.

use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Dram {
    // The canonical key is a page of installed storage, independent of the
    // current column configuration. Untouched storage reads zero.
    pages: BTreeMap<u32, Box<[u64; 512]>>,
}

impl Dram {
    fn location(pa: u64, control: u32) -> Result<Option<(u32, usize)>, String> {
        if pa > u64::from(u32::MAX) || pa & 7 != 0 {
            return Err(format!("unsupported Ultra 5 cacheable xword at {pa:#x}"));
        }
        // The MCU sees PA[29:0]; the top of the 4 GiB cacheable port
        // repeats the 1 GiB DRAM aperture (table 6-1).
        let pa = pa as u32;
        let rank = (pa >> 29) & 1;
        let cell = if control & (1 << 12) != 0 {
            if pa & (1 << 28) == 0 {
                // pair 0 is empty
                return Ok(None);
            }
            pa & 0x0fff_ffff
        } else {
            if (pa >> 27) & 3 != 2 {
                return Ok(None);
            }
            // On the 11-column DIMMs, CA10 is PA14. In 10-column mode
            // PA14 is also the bottom row bit (figures 7-3/7-4).
            let row = (pa >> 14) & 0x1fff;
            (row << 15) | (pa & 0x7fff)
        };
        let installed = (rank << 28) | cell;
        Ok(Some((installed >> 12, ((installed & 4095) >> 3) as usize)))
    }

    pub(super) fn read(&self, pa: u64, control: u32) -> Result<u64, String> {
        Ok(Self::location(pa, control)?
            .and_then(|(page, index)| self.pages.get(&page).map(|words| words[index]))
            .unwrap_or(0))
    }

    pub(super) fn write(&mut self, pa: u64, control: u32, value: u64) -> Result<(), String> {
        if let Some((page, index)) = Self::location(pa, control)? {
            if let Some(words) = self.pages.get_mut(&page) {
                words[index] = value;
            } else if value != 0 {
                let mut words = Box::new([0; 512]);
                words[index] = value;
                self.pages.insert(page, words);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_two_bounds_and_rank_selection_follow_the_column_mode() {
        for (mode, span) in [(0, 0x0800_0000), (0x1000, 0x1000_0000)] {
            let mut dram = Dram::default();
            for (rank, start) in [0x1000_0000, 0x3000_0000].into_iter().enumerate() {
                for (lane, addr) in [start, start + 8, start + 16, start + span - 8]
                    .into_iter()
                    .enumerate()
                {
                    let value = 0x1100 + 4 * rank as u64 + lane as u64;
                    dram.write(addr, mode, value).unwrap();
                }
                for absent in [start - 8, start + span] {
                    dram.write(absent, mode, 0xff).unwrap();
                    assert_eq!(dram.read(absent, mode).unwrap(), 0);
                }
            }
            for (rank, start) in [0x1000_0000, 0x3000_0000].into_iter().enumerate() {
                for (lane, addr) in [start, start + 8, start + 16, start + span - 8]
                    .into_iter()
                    .enumerate()
                {
                    let value = 0x1100 + 4 * rank as u64 + lane as u64;
                    assert_eq!(dram.read(addr, mode | 0xf00).unwrap(), value);
                    assert_eq!(dram.read(addr + 0xc000_0000, mode).unwrap(), value);
                }
            }
        }
    }

    #[test]
    fn column_switch_preserves_physical_cells_and_separates_ca10_from_the_low_row_bit() {
        let mut dram = Dram::default();
        let base = 0x1000_0000;
        for (offset, value) in [(0, 11), (0x4000, 22), (0x8000, 33), (0xc000, 44)] {
            dram.write(base + offset, 0x1000, value).unwrap();
        }
        assert_eq!(dram.read(base, 0).unwrap(), 11);
        assert_eq!(dram.read(base + 0x4000, 0).unwrap(), 44);
        dram.write(base + 0x8000, 0, 55).unwrap();
        assert_eq!(dram.read(base + 0x10000, 0x1000).unwrap(), 55);
        assert_eq!(dram.read(base + 0x4000, 0x1000).unwrap(), 22);
        assert_eq!(dram.read(base + 0x8000, 0x1000).unwrap(), 33);
    }
}
