//! Headless UltraSPARC V9 interpreter fixture. This is intentionally separate
//! from the V8 `Cpu`: it implements only the instruction subset exercised by
//! the V9 fixture and is not wired into workstation machine construction.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V9Cpu {
    regs: [u64; 32],
    pc: u64,
    npc: u64,
    ccr: u8,
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
}

impl V9Cpu {
    pub fn new(pc: u64) -> Self {
        Self {
            regs: [0; 32],
            pc,
            npc: pc.wrapping_add(4),
            ccr: 0,
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
        let word = memory.read_word(self.pc);
        let next_pc = self.npc;
        let mut next_npc = self.npc.wrapping_add(4);
        let rd = ((word >> 25) & 31) as u8;
        let rs1 = ((word >> 14) & 31) as u8;
        let op3 = (word >> 19) & 63;
        let b = if word & (1 << 13) != 0 {
            ((word << 19) as i32 >> 19) as i64 as u64
        } else {
            self.read_reg((word & 31) as u8)
        };
        match word >> 30 {
            0 if (word >> 22) & 7 == 4 => {
                self.write_reg(rd, u64::from(word & 0x3f_ffff) << 10);
            }
            0 if (word >> 22) & 7 == 2 => {
                let condition = word >> 25 & 15;
                let annul = word & (1 << 29) != 0;
                let taken = self.condition(condition);
                if taken {
                    next_npc = self
                        .pc
                        .wrapping_add((((word << 10) as i32 >> 8) as i64) as u64);
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
                    let result = self.read_reg(rs1).wrapping_add(b);
                    self.write_reg(rd, result);
                    self.set_add_cc(self.read_reg(rs1), b, result);
                }
                0x25 => self.write_reg(rd, self.read_reg(rs1) << (b & 63)),
                0x26 => self.write_reg(rd, self.read_reg(rs1) >> (b & 63)),
                0x28 if rs1 == 2 => self.write_reg(rd, u64::from(self.ccr)),
                _ => return Err(format!("unsupported V9 op3 {op3:#x}")),
            },
            3 => match op3 {
                0x0e => memory.write_u64(self.read_reg(rs1).wrapping_add(b), self.read_reg(rd)),
                0x0b => {
                    let value = memory.read_u64(self.read_reg(rs1).wrapping_add(b));
                    self.write_reg(rd, value);
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
        let low_signed = i64::from(a as i32).wrapping_add(i64::from(b as i32));
        let full_signed = (a as i64).wrapping_add(b as i64);
        self.ccr = u8::from(low & 0x8000_0000 != 0) << 3
            | u8::from(low == 0) << 2
            | u8::from(low_signed < i64::from(i32::MIN) || low_signed > i64::from(i32::MAX)) << 1
            | u8::from(low_carry)
            | u8::from(result & (1 << 63) != 0) << 7
            | u8::from(result == 0) << 6
            | u8::from(full_signed < i64::MIN || full_signed > i64::MAX) << 5
            | u8::from(wide_carry) << 4;
    }
}
