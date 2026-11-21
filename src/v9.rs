//! Headless UltraSPARC V9 interpreter fixture. This is intentionally separate
//! from the V8 `Cpu`: it implements only the instruction subset exercised by
//! the V9 fixture and is not wired into workstation machine construction.

use crate::ultra5::PowerOnReset;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V9Cpu {
    regs: [u64; 32],
    pc: u64,
    npc: u64,
    ccr: u8,
    privileged: bool,
    lsu_control: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct V9Memory {
    bytes: Vec<u8>,
}

impl V9Memory {
    pub fn new(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
        }
    }

    pub fn load_words(&mut self, address: u64, words: &[u32]) {
        for (index, word) in words.iter().enumerate() {
            let at = address as usize + index * 4;
            self.bytes[at..at + 4].copy_from_slice(&word.to_be_bytes());
        }
    }

    pub fn read_word(&self, address: u64) -> u32 {
        let at = address as usize;
        u32::from_be_bytes(self.bytes[at..at + 4].try_into().unwrap())
    }

    pub fn read_u64(&self, address: u64) -> u64 {
        let at = address as usize;
        u64::from_be_bytes(self.bytes[at..at + 8].try_into().unwrap())
    }

    pub fn write_u64(&mut self, address: u64, value: u64) {
        let at = address as usize;
        self.bytes[at..at + 8].copy_from_slice(&value.to_be_bytes());
    }

    fn checked_range(&self, address: u64, len: u64, alignment: u64) -> Result<usize, String> {
        if address % alignment != 0 {
            return Err(format!("misaligned V9 access at {address:#x}"));
        }
        let end = address
            .checked_add(len)
            .ok_or_else(|| format!("V9 address overflow at {address:#x}"))?;
        if end > self.bytes.len() as u64 {
            return Err(format!("V9 access outside memory at {address:#x}"));
        }
        Ok(address as usize)
    }

    pub fn try_read_word(&self, address: u64) -> Result<u32, String> {
        let at = self.checked_range(address, 4, 4)?;
        Ok(u32::from_be_bytes(
            self.bytes[at..at + 4].try_into().unwrap(),
        ))
    }

    pub fn try_read_u64(&self, address: u64) -> Result<u64, String> {
        let at = self.checked_range(address, 8, 8)?;
        Ok(u64::from_be_bytes(
            self.bytes[at..at + 8].try_into().unwrap(),
        ))
    }

    pub fn try_write_u64(&mut self, address: u64, value: u64) -> Result<(), String> {
        let at = self.checked_range(address, 8, 8)?;
        self.bytes[at..at + 8].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
}

impl From<PowerOnReset> for V9Cpu {
    fn from(reset: PowerOnReset) -> Self {
        Self {
            npc: reset.npc,
            privileged: reset.privileged,
            // UltraSPARC-IIi User's Manual, table 17-5: all LSU_CONTROL bits clear on POR.
            lsu_control: 0,
            ..Self::new(reset.pc)
        }
    }
}

impl V9Cpu {
    /// Start the flat-memory instruction fixture in user mode.
    pub fn new(pc: u64) -> Self {
        Self {
            regs: [0; 32],
            pc,
            npc: pc.wrapping_add(4),
            ccr: 0,
            privileged: false,
            lsu_control: 0,
        }
    }

    pub fn read_reg(&self, index: u8) -> u64 {
        if index == 0 {
            0
        } else {
            self.regs[usize::from(index)]
        }
    }

    pub fn write_reg(&mut self, index: u8, value: u64) {
        if index != 0 {
            self.regs[usize::from(index)] = value;
        }
    }

    pub fn pc(&self) -> u64 {
        self.pc
    }
    pub fn npc(&self) -> u64 {
        self.npc
    }
    pub fn ccr(&self) -> u8 {
        self.ccr
    }

    pub fn step(&mut self, memory: &mut V9Memory) -> Result<(), String> {
        let word = memory.try_read_word(self.pc)?;
        self.execute_fetched(word, Some(memory))
    }

    // The reset flash supplies instruction bytes but has no board data bus yet.
    pub(crate) fn execute_fetched(
        &mut self,
        word: u32,
        memory: Option<&mut V9Memory>,
    ) -> Result<(), String> {
        let next_pc = self.npc;
        let mut next_npc = self.npc.wrapping_add(4);
        let rd = ((word >> 25) & 31) as u8;
        let rs1 = ((word >> 14) & 31) as u8;
        let op2 = (word >> 22) & 7;
        let op3 = (word >> 19) & 63;
        let b = if word & (1 << 13) != 0 {
            ((word << 19) as i32 >> 19) as i64 as u64
        } else {
            self.read_reg((word & 31) as u8)
        };
        match word >> 30 {
            0 if op2 == 4 => {
                self.write_reg(rd, u64::from(word & 0x3f_ffff) << 10);
            }
            0 if op2 == 1 || op2 == 2 => {
                let condition = word >> 25 & 15;
                let annul = word & (1 << 29) != 0;
                let displacement = if op2 == 1 {
                    // V9 A.7: BPA uses signed disp19, independent of the prediction hint.
                    let cc = (word >> 20) & 3;
                    if condition != 8 || !matches!(cc, 0 | 2) {
                        return Err(format!("unsupported V9 BPcc word {word:#x}"));
                    }
                    ((word << 13) as i32) >> 11
                } else {
                    ((word << 10) as i32) >> 8 // Bicc: signed disp22, scaled by four.
                };
                let taken = self.condition(condition);
                if taken {
                    next_npc = self.pc.wrapping_add_signed(i64::from(displacement));
                }
                if annul && (!taken || condition == 8) {
                    self.pc = next_npc;
                    self.npc = next_npc.wrapping_add(4);
                    return Ok(());
                }
            }
            2 => match op3 {
                0x02 => self.write_reg(rd, self.read_reg(rs1) | b),
                0x00 => self.write_reg(rd, self.read_reg(rs1).wrapping_add(b)),
                0x10 => {
                    let a = self.read_reg(rs1);
                    let result = a.wrapping_add(b);
                    self.write_reg(rd, result);
                    self.set_add_cc(a, b, result);
                }
                0x25..=0x27 => {
                    let a = self.read_reg(rs1);
                    let wide = word & (1 << 12) != 0;
                    let amount = if wide { b & 63 } else { b & 31 };
                    let value = match (op3, wide) {
                        (0x25, _) => a << amount,
                        (0x26, true) => a >> amount,
                        (0x26, false) => u64::from((a as u32) >> amount),
                        (0x27, true) => ((a as i64) >> amount) as u64,
                        (0x27, false) => ((a as i32) >> amount) as i64 as u64,
                        _ => unreachable!(),
                    };
                    self.write_reg(rd, value);
                }
                0x28 if rs1 == 2 => self.write_reg(rd, u64::from(self.ccr)),
                _ => return Err(format!("unsupported V9 op3 {op3:#x}")),
            },
            3 => match op3 {
                0x0e => {
                    let address = self.read_reg(rs1).wrapping_add(b);
                    let memory = memory.ok_or("V9 STX requires mapped data memory")?;
                    memory.try_write_u64(address, self.read_reg(rd))?;
                }
                0x0b => {
                    let address = self.read_reg(rs1).wrapping_add(b);
                    let memory = memory.ok_or("V9 LDX requires mapped data memory")?;
                    let value = memory.try_read_u64(address)?;
                    self.write_reg(rd, value);
                }
                0x1b | 0x1e => {
                    if word & (1 << 13) != 0 {
                        return Err("unsupported V9 access using the %asi register".into());
                    }
                    let asi = (word >> 5) & 0xff;
                    if asi != 0x45 {
                        return Err(format!("unsupported V9 alternate address space {asi:#x}"));
                    }
                    if !self.privileged {
                        return Err("privileged V9 LSU control register access".into());
                    }
                    let address = self.read_reg(rs1).wrapping_add(b);
                    if address != 0 {
                        return Err(format!(
                            "unsupported LSU control register address {address:#x}"
                        ));
                    }
                    if op3 == 0x1b {
                        self.write_reg(rd, self.lsu_control);
                    } else {
                        let value = self.read_reg(rd);
                        // IIi A.6.1: IC and DC occupy bits 0 and 1. The fixture
                        // supports their register readback and the OBP all-off write.
                        const CACHE_ENABLES: u64 = 0b11;
                        if value & !CACHE_ENABLES != 0 {
                            return Err(format!("unsupported LSU control write {value:#x}"));
                        }
                        self.lsu_control = value;
                    }
                }
                _ => return Err(format!("unsupported V9 memory op3 {op3:#x}")),
            },
            _ => return Err(format!("unsupported V9 word {word:#x}")),
        }
        self.pc = next_pc;
        self.npc = next_npc;
        Ok(())
    }

    pub fn run(&mut self, memory: &mut V9Memory, budget: usize) -> Result<(), String> {
        for _ in 0..budget {
            self.step(memory)?;
        }
        Ok(())
    }

    fn condition(&self, condition: u32) -> bool {
        let n = self.ccr & 0x8 != 0;
        let z = self.ccr & 0x4 != 0;
        let v = self.ccr & 0x2 != 0;
        let c = self.ccr & 0x1 != 0;
        let less = n ^ v;
        match condition {
            0 => false,
            1 => z,
            2 => z || less,
            3 => less,
            4 => c || z,
            5 => c,
            6 => n,
            7 => v,
            8 => true,
            9 => !z,
            10 => !z && !less,
            11 => !less,
            12 => !c && !z,
            13 => !c,
            14 => !n,
            15 => !v,
            _ => false,
        }
    }

    fn set_add_cc(&mut self, a: u64, b: u64, result: u64) {
        let low = (a as u32).wrapping_add(b as u32);
        let low_wide = u64::from(a as u32) + u64::from(b as u32);
        let low_carry = low_wide > u64::from(u32::MAX);
        let wide = u128::from(a) + u128::from(b);
        let wide_carry = wide > u128::from(u64::MAX);
        let low_signed = i64::from(a as i32) + i64::from(b as i32);
        let full_signed = i128::from(a as i64) + i128::from(b as i64);
        self.ccr = u8::from(low & 0x8000_0000 != 0) << 3
            | u8::from(low == 0) << 2
            | u8::from(low_signed < i64::from(i32::MIN) || low_signed > i64::from(i32::MAX)) << 1
            | u8::from(low_carry)
            | u8::from(result & (1 << 63) != 0) << 7
            | u8::from(result == 0) << 6
            | u8::from(full_signed < i128::from(i64::MIN) || full_signed > i128::from(i64::MAX))
                << 5
            | u8::from(wide_carry) << 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ultra5::{BootFlash, FLASH_SIZE};

    fn verify_reset_pair(flash: &BootFlash, mut cpu: V9Cpu) -> V9Cpu {
        let mut expected = cpu.clone();
        expected.pc = 0xffff_ffff_f000_0028;
        expected.npc = 0xffff_ffff_f000_002c;
        expected.regs[2] = cpu.lsu_control;
        expected.lsu_control = 0;

        let fetches = flash.execute_first_two_instructions(&mut cpu).unwrap();
        assert_eq!(cpu, expected);
        assert_eq!(fetches[0].instruction, 0xc4d8_08a0);
        assert_eq!(fetches[1].instruction, 0xc0f0_08a0);
        assert_eq!(
            (fetches[0].flash_offset, fetches[1].flash_offset),
            (0x20, 0x24)
        );
        assert_eq!(fetches[1].physical_address, 0x01ff_f000_0024);
        assert_eq!(
            (fetches[1].pc, fetches[1].npc),
            (0xffff_ffff_f000_0024, 0xffff_ffff_f000_0028)
        );
        assert!(flash.execute_first_two_instructions(&mut cpu).is_err());
        assert_eq!(cpu, expected);
        cpu
    }

    #[test]
    fn obp_ldxa_observes_the_old_lsu_bit_and_stxa_clears_it() {
        let mut image = vec![0xff; FLASH_SIZE];
        image[0x20..0x24].copy_from_slice(&0xc4d8_08a0_u32.to_be_bytes());
        image[0x24..0x28].copy_from_slice(&0xc0f0_08a0_u32.to_be_bytes());
        image[0x28..0x2c].copy_from_slice(&0x8410_2007_u32.to_be_bytes()); // mov 7, %g2
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.lsu_control = 1; // IC
        cpu.ccr = 0xa5;
        for rd in 1..32 {
            cpu.write_reg(rd, u64::MAX - u64::from(rd));
        }
        verify_reset_pair(&BootFlash::new(image).unwrap(), cpu);
    }

    #[test]
    #[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
    fn original_ultra5_obp_retires_the_first_two_instructions() {
        let image = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap();
        let flash = BootFlash::new(image).unwrap();
        let cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        verify_reset_pair(&flash, cpu.clone());
        let mut seeded = cpu;
        seeded.lsu_control = 2; // DC
        seeded.ccr = 0x44;
        let retired = verify_reset_pair(&flash, seeded);
        println!(
            "Seeded DC run after two OBP words: %g2={:#x} LSU={:#x} CCR={:#x} PC={:#x} nPC={:#x}",
            retired.read_reg(2),
            retired.lsu_control,
            retired.ccr(),
            retired.pc(),
            retired.npc()
        );
    }

    #[test]
    fn ldxa_lsu_control_preserves_nonzero_ccr() {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        cpu.ccr = 0x44; // Z is set in both ICC and XCC.
        cpu.execute_fetched(0xc4d8_08a0, None).unwrap();
        assert_eq!(cpu.ccr(), 0x44);
    }

    #[test]
    fn ldxa_returns_the_full_contents_of_lsu_control() {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        // A parity-mask bit and both cache enables distinguish a register read
        // from assigning the POR constant. No guest LSU store is required here.
        cpu.lsu_control = (1 << 40) | 3;
        cpu.execute_fetched(0xc4d8_08a0, None).unwrap();
        assert_eq!(cpu.read_reg(2), 0x100_0000_0003);
    }
}
