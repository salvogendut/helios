//! UltraSPARC-IIi primary caches. Sun IIi manual §§3.1.1, 8.2 and A.7–8.
use super::{CpuFault, IntegerWidth, Translation, V9Cpu, V9DataBus, LSU_DC, LSU_IC};

const I_VALID: u32 = 1 << 28;

impl V9Cpu {
    /// Fetch and retire a translated cacheable instruction. A diagnostic access or
    /// FLUSH in the fetched instruction sees its own fill. A rejected instruction
    /// restores the previous line and replacement state, like the I-TLB Used bit.
    pub(crate) fn retire_ram_instruction(
        &mut self,
        mapping: Translation,
        bus: &mut dyn V9DataBus,
    ) -> Result<u32, CpuFault> {
        let pa = mapping.pa;
        let retire = |cpu: &mut Self, xword: u64, bus: &mut dyn V9DataBus| {
            let word = if pa & 4 == 0 {
                (xword >> 32) as u32
            } else {
                xword as u32
            };
            cpu.retire_fetch(mapping, word, bus)?;
            Ok(word)
        };
        if self.lsu_control & LSU_IC == 0 {
            return retire(self, bus.read_cacheable_xword(pa & !7)?, bus);
        }
        let index = (pa as usize >> 5) & 255;
        let ways = [index, index + 256];
        let tag = I_VALID | (pa >> 13) as u32;
        let lane = (pa as usize >> 3) & 3;
        if let Some(slot) = ways.into_iter().find(|&slot| self.icache_tags[slot] == tag) {
            return retire(self, self.icache_data[slot][lane], bus);
        }

        let mut line = [0; 4];
        for (i, word) in line.iter_mut().enumerate() {
            *word = bus.read_cacheable_xword((pa & !31) + 8 * i as u64)?;
        }
        // Prefer an invalid way, otherwise alternate on replacement.
        let next = self.icache_next[index];
        let slot = ways
            .into_iter()
            .find(|&slot| self.icache_tags[slot] & I_VALID == 0)
            .unwrap_or(index + 256 * usize::from(next));
        let old_line = std::mem::replace(&mut self.icache_data[slot], line);
        let old_tag = std::mem::replace(&mut self.icache_tags[slot], tag);
        self.icache_next[index] = slot == index;
        let result = retire(self, line[lane], bus);
        if result.is_err() {
            self.icache_data[slot] = old_line;
            self.icache_tags[slot] = old_tag;
            self.icache_next[index] = next;
        }
        result
    }

    /// Retire resident integer countdowns without passing a timer, page or cache-line
    /// boundary. Both OBP's two-word loop and the Solaris microsecond delay have
    /// no data accesses. Leave the final exit to ordinary execution.
    pub(crate) fn repeat_countdown(&mut self, budget: u64) -> u64 {
        if self.pc & 3 != 0
            || self.pc & 31 == 28
            || self.npc != self.pc.wrapping_add(4)
            || self.ccr != 0
            || self.lsu_control & LSU_IC == 0
            || budget < 2
        {
            return 0;
        }
        let Ok(fetch) = self.instruction_address() else {
            return 0;
        };
        let Some(tlb_slot) = fetch.slot.filter(|_| fetch.cacheable) else {
            return 0;
        };
        let set = (fetch.pa as usize >> 5) & 255;
        let tag = I_VALID | (fetch.pa >> 13) as u32;
        let Some(way) = [set, set + 256]
            .into_iter()
            .find(|&way| self.icache_tags[way] == tag)
        else {
            return 0;
        };
        let line = self.icache_data[way];
        let first = (fetch.pa as usize & 31) / 4;
        let word = |n: usize| {
            if n & 1 == 0 {
                (line[n / 2] >> 32) as u32
            } else {
                line[n / 2] as u32
            }
        };
        let branch = word(first);
        let sub = word(first + 1);
        let reg = ((sub >> 25) & 31) as u8;
        if !matches!(branch, 0x1468_0000 | 0x1280_0000)
            || reg == 0
            || sub != (0x80a0_2001 | (u32::from(reg) << 25) | (u32::from(reg) << 14))
        {
            return 0;
        }
        let count = self.read_reg(reg);
        if !(2..=i32::MAX as u64).contains(&count) {
            return 0;
        }
        let mut remaining = budget;
        if self.tick_compare & super::TICK_INT_DIS == 0 {
            let distance = self.tick_compare.wrapping_sub(self.tick) & !super::TICK_NPT;
            if distance != 0 {
                remaining = remaining.min((distance - 1) / super::TICK_CYCLES_PER_OUTCOME);
            }
        }
        let iterations = (remaining / 2).min(count - 1);
        let mut steps = 2 * iterations;
        let mut final_inner = count - iterations;
        // BNE . / SUBcc %o3,1,%o3 / SUBcc %o0,1,%o0 / BG .-12 /
        // ORcc %o4,0,%o3. Completing the outer branch resets CCR and %o3.
        if branch == 0x1280_0000
            && sub == 0x96a2_e001
            && first <= 3
            && [word(first + 2), word(first + 3), word(first + 4)]
                == [0x90a2_2001, 0x14bf_fffd, 0x9693_2000]
        {
            let outer = self.read_reg(8);
            let reload = self.read_reg(12);
            let first_cost = 2 * (count + 1) + 3;
            if (2..=i32::MAX as u64).contains(&outer)
                && (1..=i32::MAX as u64).contains(&reload)
                && remaining >= first_cost
            {
                let cost = 2 * (reload + 1) + 3;
                let extra = ((remaining - first_cost) / cost).min(outer - 2);
                steps = first_cost + extra * cost;
                self.write_reg(8, outer - 1 - extra);
                final_inner = reload;
            }
        }
        if steps != 0 {
            self.instruction_tlb.set_used(tlb_slot, true);
            self.write_reg(reg, final_inner);
            let elapsed = steps.wrapping_mul(super::TICK_CYCLES_PER_OUTCOME);
            self.tick = (self.tick & super::TICK_NPT)
                | (self.tick.wrapping_add(elapsed) & !super::TICK_NPT);
        }
        steps
    }

    /// Coherent PCI writes snoop both IIi primary caches (§§1.3.6 and 3.1.1).
    pub(crate) fn snoop_dma_write(&mut self, pa: u64, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let last = (pa + bytes as u64 - 1) & !15;
        for subblock in (pa & !15..=last).step_by(16) {
            self.flush_instruction(subblock);
            let set = (subblock as usize >> 5) & 255;
            let valid = 1 << ((subblock >> 4) & 1);
            for slot in [set, set + 256] {
                if self.dcache_tags[slot] >> 2 == (subblock >> 13) as u32 {
                    self.dcache_tags[slot] &= !valid;
                }
            }
        }
    }

    pub(super) fn flush_instruction(&mut self, pa: u64) {
        let index = (pa as usize >> 5) & 255;
        for slot in [index, index + 256] {
            if self.icache_tags[slot] == (I_VALID | (pa >> 13) as u32) {
                self.icache_tags[slot] &= !I_VALID;
            }
        }
    }

    pub(super) fn load_data(
        &mut self,
        bus: &mut dyn V9DataBus,
        va: u64,
        mapping: Translation,
        width: IntegerWidth,
    ) -> Result<u64, String> {
        let pa = mapping.pa;
        if !mapping.cacheable {
            return self.uncached_read(bus, pa, width);
        }
        if self.lsu_control & LSU_DC == 0 {
            return bus.read_integer(pa, width, mapping.cacheable);
        }
        let slot = (va as usize >> 5) & 511;
        let tag = (pa >> 13) as u32;
        let half = (pa as usize >> 4) & 1;
        let valid = 1 << half;
        let old = self.dcache_tags[slot];
        if old >> 2 != tag || old & valid == 0 {
            if !mapping.virtual_cacheable {
                return bus.read_integer(pa, width, true);
            }
            let base = pa & !15;
            let data = [
                bus.read_cacheable_xword(base)?,
                bus.read_cacheable_xword(base + 8)?,
            ];
            self.dcache_data[slot][2 * half..2 * half + 2].copy_from_slice(&data);
            self.dcache_tags[slot] = if old >> 2 == tag {
                old | valid
            } else {
                tag << 2 | valid
            };
        }
        let word = self.dcache_data[slot][(pa as usize >> 3) & 3];
        Ok((word >> width.lane_shift(pa)) & width.mask())
    }

    /// Call only after a successful cacheable write to the backing store.
    pub(super) fn update_data_hit(&mut self, va: u64, pa: u64, width: IntegerWidth, value: u64) {
        if self.lsu_control & LSU_DC == 0 {
            return;
        }
        let slot = (va as usize >> 5) & 511;
        let tag = self.dcache_tags[slot];
        if tag >> 2 == (pa >> 13) as u32 && tag & (1 << ((pa >> 4) & 1)) != 0 {
            let shift = width.lane_shift(pa);
            let mask = width.mask() << shift;
            let word = &mut self.dcache_data[slot][(pa as usize >> 3) & 3];
            *word = (*word & !mask) | ((value << shift) & mask);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v9::{V9Memory, LSU_DM, LSU_IM, PSTATE_PRIV};

    const NOP: u32 = 0x0100_0000;
    const SET_G1: u32 = 0x0300_0001;
    const VALID: u64 = 1 << 63;

    #[test]
    fn casa_compares_low_word_updates_selected_lane_and_checks_alignment_before_writeback() {
        let mut cpu = cpu();
        let mut mem = V9Memory::new(64);
        mem.write_cacheable_xword(8, 0x1122_3344_aabb_ccdd).unwrap();
        cpu.write_reg(1, 12);
        cpu.write_reg(2, 0x2020);
        cpu.write_reg(3, u64::MAX - 0x55443322);
        // casa [%g1] ASI_PRIMARY, %g3, %g2
        let instruction = 0xc5e0_5003;
        cpu.execute_fetched(instruction, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 0xaabbccdd);
        assert_eq!(mem.read_u64(8), 0x11223344_00002020);
        cpu.write_reg(2, 0x7777);
        cpu.execute_fetched(instruction, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 0x2020);
        assert_eq!(mem.read_u64(8), 0x11223344_00002020);
        cpu.write_reg(1, 11);
        let before = cpu.clone();
        assert!(cpu.execute_fetched(instruction, &mut mem).is_err());
        assert!(cpu == before);
    }

    #[test]
    fn counted_delay_matches_retirement_and_stops_before_timer_or_exit() {
        let mut mem = V9Memory::new(32);
        mem.load_words(0, &[0x1468_0000, 0xa2a4_6001]);
        let mut cpu = cpu();
        cpu.write_reg(17, 50);
        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();
        let mut reference = cpu.clone();
        assert_eq!(cpu.repeat_countdown(7), 6);
        for _ in 0..6 {
            reference.step(&mut mem).unwrap();
        }
        assert!(reference == cpu);
        cpu.tick_compare =
            (cpu.tick & !super::super::TICK_NPT) + 4 * super::super::TICK_CYCLES_PER_OUTCOME;
        assert_eq!(cpu.repeat_countdown(100), 2);
        assert_eq!(cpu.repeat_countdown(100), 0);
        cpu.tick_compare = super::super::TICK_INT_DIS;
        let count = cpu.read_reg(17);
        assert_eq!(cpu.repeat_countdown(u64::MAX), 2 * (count - 1));
        assert_eq!(cpu.read_reg(17), 1);
        assert_eq!(cpu.repeat_countdown(u64::MAX), 0);
        cpu.write_reg(17, 50);
        cpu.ccr = 4;
        assert_eq!(cpu.repeat_countdown(4), 0);
        cpu.ccr = 0;
        cpu.lsu_control &= !LSU_IC;
        assert_eq!(cpu.repeat_countdown(4), 0);
    }

    #[test]
    fn solaris_nested_delay_matches_single_steps_at_each_budget() {
        let mut ram = V9Memory::new(64);
        ram.load_words(
            8,
            &[
                0x9693_2000,
                0x1280_0000,
                0x96a2_e001,
                0x90a2_2001,
                0x14bf_fffd,
                0x9693_2000,
                0x0100_0000,
            ],
        );
        let mut seed = cpu();
        seed.pc = 8;
        seed.npc = 12;
        seed.write_reg(8, 17);
        seed.write_reg(12, 7);
        seed.step(&mut ram).unwrap();
        for budget in [1, 2, 3, 11, 12, 13, 18, 19, 20, 37, 38, 39, 64, 1000] {
            let mut fast = seed.clone();
            let mut slow = seed.clone();
            let count = fast.repeat_countdown(budget);
            assert!(count <= budget);
            if budget >= 19 {
                assert!(count >= 19);
            }
            for _ in 0..count {
                slow.step(&mut ram).unwrap();
            }
            assert!(fast == slow, "budget={budget} count={count}");
        }
    }

    fn cpu() -> V9Cpu {
        let mut cpu = V9Cpu::new(0);
        cpu.pstate = PSTATE_PRIV;
        cpu.lsu_control = LSU_DM | LSU_IM | LSU_DC | LSU_IC;
        for (tlb, instruction) in [(&mut cpu.instruction_tlb, true), (&mut cpu.data_tlb, false)] {
            for slot in 0..3 {
                let base = (slot as u64) << 13;
                tlb.stage_tag(base);
                tlb.write_data(slot, VALID | base | 0x36, instruction);
            }
        }
        cpu
    }

    fn fetch(cpu: &mut V9Cpu, mem: &mut V9Memory, pc: u64) -> Result<(), String> {
        cpu.pc = pc;
        cpu.npc = pc + 4;
        cpu.step(mem)
    }

    #[test]
    fn two_instruction_ways_share_a_set_and_flush_uses_the_translated_physical_tag() {
        let mut cpu = cpu();
        let mut mem = V9Memory::new(0x4020);
        for pc in [0, 0x2000] {
            mem.load_words(pc, &[NOP, NOP]);
            fetch(&mut cpu, &mut mem, pc).unwrap();
            mem.load_words(pc + 4, &[SET_G1]);
            fetch(&mut cpu, &mut mem, pc + 4).unwrap();
            assert_eq!(cpu.read_reg(1), 0);
        }
        // FLUSH VA 0x4000+7 maps to PA 7, sharing the first line.
        cpu.data_tlb.stage_tag(0x4000);
        cpu.data_tlb.write_data(2, VALID | 0x36, false);
        cpu.write_reg(2, 0x4007);
        cpu.execute_fetched(0x81d8_a000, &mut mem).unwrap();
        fetch(&mut cpu, &mut mem, 0x2004).unwrap();
        assert_eq!(cpu.read_reg(1), 0);
        fetch(&mut cpu, &mut mem, 4).unwrap();
        assert_eq!(cpu.read_reg(1), 1024);
        cpu.write_reg(1, 0);
        cpu.lsu_control &= !LSU_IC;
        fetch(&mut cpu, &mut mem, 0x2004).unwrap();
        assert_eq!(cpu.read_reg(1), 1024);
    }

    #[test]
    fn rejected_fetch_restores_evicted_line_and_replacement_state() {
        let mut cpu = cpu();
        let mut mem = V9Memory::new(0x4020);
        for pc in [0, 0x2000] {
            mem.load_words(pc, &[NOP]);
            fetch(&mut cpu, &mut mem, pc).unwrap();
        }
        cpu.pc = 0x4000;
        cpu.npc = 0x4004;
        let before = cpu.clone();
        // First all four xwords can be read but opcode zero is unsupported.
        assert!(cpu.step(&mut mem).unwrap_err().contains("unsupported"));
        assert!(cpu == before);
        // Then the final xword of the fill is outside the supplied RAM.
        let mut short = V9Memory::new(0x4018);
        short.load_words(0x4000, &[SET_G1]);
        assert!(cpu.step(&mut short).is_err());
        assert!(cpu == before);
    }

    #[test]
    fn dcache_subblocks_store_lanes_and_physical_asi_hit_without_allocating() {
        let mut cpu = cpu();
        let mut mem = V9Memory::new(64);
        mem.write_cacheable_xword(0, 0x1122_3344_5566_7788).unwrap();
        mem.write_cacheable_xword(8, 0x1234).unwrap();
        // LDX [%g1],%g2 fills both xwords of the low subblock only.
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 0x1122_3344_5566_7788);
        assert_eq!(cpu.dcache_tags[0], 1);
        mem.write_cacheable_xword(0, 0).unwrap();
        mem.write_cacheable_xword(8, 0).unwrap();
        mem.write_cacheable_xword(16, 0xaabb).unwrap();
        cpu.write_reg(1, 16);
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        assert_eq!((cpu.read_reg(2), cpu.dcache_tags[0]), (0xaabb, 3));
        cpu.write_reg(1, 8);
        cpu.execute_fetched(0xc6d8_4280, &mut mem).unwrap(); // LDXA ASI 14 -> g3
        assert_eq!(cpu.read_reg(3), 0x1234);
        cpu.write_reg(1, 1);
        cpu.write_reg(2, 0xfe);
        cpu.execute_fetched(0xc428_6000, &mut mem).unwrap(); // STB
        assert_eq!(mem.try_read_u64(0).unwrap(), 0x00fe_0000_0000_0000);
        cpu.write_reg(1, 0);
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 0x11fe_3344_5566_7788);
        cpu.write_reg(1, 32);
        cpu.execute_fetched(0xc470_6000, &mut mem).unwrap(); // STX on a miss
        cpu.execute_fetched(0xc6d8_4280, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(3), 0x11fe_3344_5566_7788);
        assert_eq!(cpu.dcache_tags[1], 0);
        cpu.lsu_control &= !LSU_DC;
        cpu.write_reg(1, 0);
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 0x00fe_0000_0000_0000);
    }

    #[test]
    fn dma_write_snoops_both_virtual_colors_and_instruction_ways() {
        let mut cpu = cpu();
        for slot in [0, 1, 256, 257] {
            cpu.dcache_tags[slot] = 3;
            cpu.icache_tags[slot] = I_VALID;
        }
        cpu.dcache_tags[257] = (1 << 2) | 3; // same index, different physical page
        cpu.snoop_dma_write(31, 2);
        assert_eq!((cpu.dcache_tags[0], cpu.dcache_tags[256]), (1, 1));
        assert_eq!((cpu.dcache_tags[1], cpu.dcache_tags[257]), (2, 7));
        for slot in [0, 1, 256, 257] {
            assert_eq!(cpu.icache_tags[slot] & I_VALID, 0);
        }
    }

    #[test]
    fn a_rejected_write_through_store_keeps_the_resident_data() {
        let mut cpu = cpu();
        let mut mem = V9Memory::new(16);
        mem.write_cacheable_xword(0, 0xfeed_1234).unwrap();
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        cpu.write_reg(2, 0xbeef);
        let before = cpu.clone();
        assert!(cpu
            .execute_fetched(0xc470_6000, &mut crate::v9::UnmappedData)
            .is_err());
        assert!(cpu == before);
        mem.write_cacheable_xword(0, 0).unwrap();
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 0xfeed_1234);
    }

    #[test]
    fn different_virtual_colors_have_separate_entries_and_failed_fills_preserve_both() {
        let mut cpu = cpu();
        let mut mem = V9Memory::new(16);
        cpu.data_tlb.stage_tag(0x2000);
        cpu.data_tlb.write_data(1, VALID | 0x36, false);
        for (address, value) in [(0, 1), (0x2000, 2)] {
            mem.write_cacheable_xword(0, value).unwrap();
            cpu.write_reg(1, address);
            cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
            assert_eq!(cpu.read_reg(2), value);
        }
        cpu.write_reg(1, 0);
        cpu.execute_fetched(0xc458_6000, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(2), 1);
        // A conflicting physical tag cannot partially displace the valid line.
        cpu.write_reg(1, 0x4000);
        let before = cpu.clone();
        let mut short = V9Memory::new(0x4008);
        assert!(cpu.execute_fetched(0xc458_6000, &mut short).is_err());
        assert!(cpu == before);
        // TLB permissions are checked even on a resident line.
        cpu.write_reg(1, 0);
        cpu.pstate = 0;
        let before = cpu.clone();
        assert!(cpu.execute_fetched(0xc458_6000, &mut mem).is_err());
        assert!(cpu == before);
    }
}
