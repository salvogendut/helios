//! Ultra 5 reference population: two dual-rank 256 MiB DIMMs in pair 2.
//! Sun UltraSPARC-IIi User's Manual §§6.2.1, 7.2–3 and Appendix A.10.

// Installed storage has 2^17 canonical 4 KiB pages. Allocate each group
// of 512 page pointers on its first nonzero write.
const PAGES_PER_GROUP: usize = 512;
type Page = [u64; 512];
type PageGroup = [Option<Box<Page>>; PAGES_PER_GROUP];

pub(super) struct Dram {
    // The canonical key is a page of installed storage, independent of the
    // current column configuration. Untouched storage reads zero.
    groups: [Option<Box<PageGroup>>; 256],
}

impl Default for Dram {
    fn default() -> Self {
        Self {
            groups: std::array::from_fn(|_| None),
        }
    }
}

impl Dram {
    fn location(pa: u64, control: u32) -> Result<Option<(usize, usize)>, String> {
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
        Ok(Some((
            (installed >> 12) as usize,
            ((installed & 4095) >> 3) as usize,
        )))
    }

    pub(super) fn dma_bytes(
        &mut self,
        mut pa: u64,
        control: u32,
        mut bytes: &mut [u8],
        write: bool,
    ) -> Result<(), String> {
        while !bytes.is_empty() {
            let base = pa & !7;
            let start = (pa & 7) as usize;
            let count = (8 - start).min(bytes.len());
            let data = &mut bytes[..count];
            if write && count == 8 {
                self.write(base, control, u64::from_be_bytes(data.try_into().unwrap()))?;
            } else {
                let mut word = self.read(base, control)?.to_be_bytes();
                if write {
                    word[start..start + count].copy_from_slice(data);
                    self.write(base, control, u64::from_be_bytes(word))?;
                } else {
                    data.copy_from_slice(&word[start..start + count]);
                }
            }
            pa += count as u64;
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
    pub(super) fn read(&self, pa: u64, control: u32) -> Result<u64, String> {
        let Some((page, index)) = Self::location(pa, control)? else {
            return Ok(0);
        };
        let group = &self.groups[page / PAGES_PER_GROUP];
        Ok(group
            .as_ref()
            .and_then(|pages| pages[page % PAGES_PER_GROUP].as_ref())
            .map_or(0, |words| words[index]))
    }

    pub(super) fn write(&mut self, pa: u64, control: u32, value: u64) -> Result<(), String> {
        if let Some((page, index)) = Self::location(pa, control)? {
            let group = &mut self.groups[page / PAGES_PER_GROUP];
            let entry = page % PAGES_PER_GROUP;
            if value == 0 {
                if let Some(words) = group.as_mut().and_then(|pages| pages[entry].as_mut()) {
                    words[index] = 0;
                }
            } else {
                let pages = group.get_or_insert_with(|| Box::new(std::array::from_fn(|_| None)));
                let words = pages[entry].get_or_insert_with(|| Box::new([0; 512]));
                words[index] = value;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_crosses_page_groups_and_preserves_big_endian_lanes_and_aliases() {
        let mut dram = Dram::default();
        let boundary = 0x1020_0000;
        dram.write(boundary - 8, 0x1000, 0x0102_0304_0506_0708)
            .unwrap();
        dram.write(boundary, 0x1000, 0x1112_1314_1516_1718).unwrap();
        let mut bytes = [0xa1, 0xa2, 0xa3, 0xb1, 0xb2];
        dram.dma_bytes(boundary - 3, 0x1000, &mut bytes, true)
            .unwrap();
        assert_eq!(
            dram.read(boundary - 8, 0x1000).unwrap(),
            0x0102_0304_05a1_a2a3
        );
        assert_eq!(dram.read(boundary, 0x1000).unwrap(), 0xb1b2_1314_1516_1718);
        let mut readback = [0; 5];
        dram.dma_bytes(boundary - 3 + 0xc000_0000, 0x1000, &mut readback, false)
            .unwrap();
        assert_eq!(readback, bytes);
    }

    #[test]
    fn zero_accesses_leave_unpopulated_page_groups_sparse() {
        let mut dram = Dram::default();
        let last = 0x3fff_fff8;
        assert_eq!(dram.read(last, 0x1000).unwrap(), 0);
        dram.write(last, 0x1000, 0).unwrap();
        assert!(dram.groups.iter().all(Option::is_none));
        dram.write(last, 0x1000, 42).unwrap();
        dram.write(last - 4096, 0x1000, 0).unwrap();
        let group = dram.groups[255].as_ref().unwrap();
        assert_eq!(group.iter().flatten().count(), 1);
        assert_eq!(dram.read(last + 0xc000_0000, 0x1000).unwrap(), 42);
        dram.write(last, 0x1000, 0).unwrap();
        assert_eq!(dram.read(last, 0x1000).unwrap(), 0);
    }

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
