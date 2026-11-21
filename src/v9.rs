//! Headless UltraSPARC V9 interpreter fixture. This is intentionally separate
//! from the V8 `Cpu`: it implements only the instruction subset exercised by
//! the V9 fixture and is not wired into workstation machine construction.

use crate::ultra5::PowerOnReset;

const WINDOW_COUNT: u8 = 8;
const WINDOW_MASK: u8 = WINDOW_COUNT - 1;
const REGISTER_COUNT: usize = 8 + WINDOW_COUNT as usize * 16;
pub(crate) const PHYSICAL_ADDRESS_MASK: u64 = (1 << 41) - 1;
const ASI_PHYS_BYPASS_EC_WITH_EBIT: u8 = 0x15;
const ASI_DMMU: u8 = 0x58;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V9Cpu {
    // Globals, followed by eight outs and eight locals per physical window.
    regs: [u64; REGISTER_COUNT],
    pc: u64,
    npc: u64,
    ccr: u8,
    cwp: u8,
    asi: u8,
    primary_context: u16,
    secondary_context: u16,
    privileged: bool,
    lsu_control: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct V9Memory {
    bytes: Vec<u8>,
}

/// Data accesses are independent of the instruction source. A reset-time
/// peripheral may implement physical bypass reads without mapping virtual RAM.
pub(crate) trait V9DataBus {
    fn load_xword(&mut self, _va: u64) -> Result<u64, String> {
        Err("V9 LDX requires mapped data memory".into())
    }
    fn store_xword(&mut self, _va: u64, _value: u64) -> Result<(), String> {
        Err("V9 STX requires mapped data memory".into())
    }
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        Err(format!(
            "unmapped V9 physical read: ASI=0x15 PA={pa:#x} width=8"
        ))
    }
}

pub(crate) struct UnmappedData;
impl V9DataBus for UnmappedData {}

impl V9DataBus for V9Memory {
    fn load_xword(&mut self, va: u64) -> Result<u64, String> {
        self.try_read_u64(va)
    }
    fn store_xword(&mut self, va: u64, value: u64) -> Result<(), String> {
        self.try_write_u64(va, value)
    }
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        self.try_read_u64(pa)
    }
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
            // IIi table 17-5 leaves CWP unknown at POR. Use the value observed on QEMU sun4u.
            cwp: 7,
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
            regs: [0; REGISTER_COUNT],
            pc,
            npc: pc.wrapping_add(4),
            ccr: 0,
            cwp: 0,
            asi: 0,
            primary_context: 0,
            secondary_context: 0,
            privileged: false,
            lsu_control: 0,
        }
    }

    fn physical_register(&self, index: u8) -> usize {
        let (window, offset) = match index {
            0..=7 => return usize::from(index),
            8..=23 => (self.cwp, index - 8),
            // V9 ins alias the previous window's outs, including the 0/7 wrap.
            24..=31 => (self.cwp.wrapping_sub(1) & WINDOW_MASK, index - 24),
            _ => panic!("register index must be in 0..32"),
        };
        8 + usize::from(window) * 16 + usize::from(offset)
    }

    pub fn read_reg(&self, index: u8) -> u64 {
        if index == 0 {
            0
        } else {
            self.regs[self.physical_register(index)]
        }
    }

    pub fn write_reg(&mut self, index: u8, value: u64) {
        if index != 0 {
            let register = self.physical_register(index);
            self.regs[register] = value;
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
    pub fn current_window(&self) -> u8 {
        self.cwp
    }

    pub fn step(&mut self, memory: &mut V9Memory) -> Result<(), String> {
        let word = memory.try_read_word(self.pc)?;
        self.execute_fetched(word, memory)
    }

    pub(crate) fn execute_fetched(
        &mut self,
        word: u32,
        memory: &mut dyn V9DataBus,
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
                let (displacement, flags) = if op2 == 1 {
                    let cc = (word >> 20) & 3;
                    let flags = match cc {
                        0 => self.ccr,
                        2 => self.ccr >> 4,
                        _ => return Err(format!("unsupported V9 BPcc word {word:#x}")),
                    };
                    (((word << 13) as i32) >> 11, flags)
                } else {
                    (((word << 10) as i32) >> 8, self.ccr)
                };
                let taken = Self::condition(condition, flags);
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
                0x14 => {
                    let a = self.read_reg(rs1);
                    let icc = Self::sub_flags(u64::from(a as u32), u64::from(b as u32), 1 << 31);
                    let xcc = Self::sub_flags(a, b, 1 << 63);
                    self.write_reg(rd, a.wrapping_sub(b));
                    self.ccr = (xcc << 4) | icc;
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
                0x28 if rs1 == 3 && word & 0x3fff == 0 => self.write_reg(rd, u64::from(self.asi)),
                // MEMBAR #Sync: all earlier operations in this single-CPU
                // fixture have already completed when an instruction retires.
                0x28 if word == 0x8143_e040 => {}
                0x2a => {
                    if !self.privileged {
                        return Err("privileged V9 RDPR instruction".into());
                    }
                    // V9 A.43: rs1 selects the privileged register; bits 13:0 are reserved.
                    if rs1 != 9 || word & 0x3fff != 0 {
                        return Err(format!("unsupported V9 RDPR word {word:#x}"));
                    }
                    self.write_reg(rd, u64::from(self.cwp));
                }
                0x30 if rd == 3 => {
                    if word & (1 << 13) == 0 && word & 0x1fe0 != 0 {
                        return Err(format!("unsupported V9 WRASI word {word:#x}"));
                    }
                    self.asi = (self.read_reg(rs1) ^ b) as u8;
                }
                0x32 => {
                    if !self.privileged {
                        return Err("privileged V9 WRPR instruction".into());
                    }
                    let reserved_bits_set = word & (1 << 13) == 0 && word & 0x1fe0 != 0;
                    if rd != 9 || reserved_bits_set {
                        return Err(format!("unsupported V9 WRPR word {word:#x}"));
                    }
                    // IIi §14.2.1 implements only the low three bits of CWP.
                    self.cwp = (self.read_reg(rs1) ^ b) as u8 & WINDOW_MASK;
                }
                _ => return Err(format!("unsupported V9 op3 {op3:#x}")),
            },
            3 => match op3 {
                0x0e => {
                    let address = self.read_reg(rs1).wrapping_add(b);
                    memory.store_xword(address, self.read_reg(rd))?;
                }
                0x0b => {
                    let address = self.read_reg(rs1).wrapping_add(b);
                    let value = memory.load_xword(address)?;
                    self.write_reg(rd, value);
                }
                0x1b | 0x1e => {
                    let asi = if word & (1 << 13) != 0 {
                        self.asi
                    } else {
                        (word >> 5) as u8
                    };
                    if asi < 0x80 && !self.privileged {
                        return Err(format!("privileged V9 alternate address space {asi:#x}"));
                    }
                    let address = self.read_reg(rs1).wrapping_add(b);
                    match asi {
                        ASI_PHYS_BYPASS_EC_WITH_EBIT if op3 == 0x1b => {
                            let value = Self::read_physical_bypass(address, memory)?;
                            self.write_reg(rd, value);
                        }
                        0x45 => self.access_lsu_control(op3, address, rd)?,
                        ASI_DMMU => self.access_mmu_context(op3, address, rd)?,
                        _ => {
                            return Err(format!("unsupported V9 alternate address space {asi:#x}"))
                        }
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

    fn read_physical_bypass(address: u64, memory: &mut dyn V9DataBus) -> Result<u64, String> {
        // IIi §§4.2 and 15.10: 44-bit sign-extended VA, 41-bit physical bus.
        if !matches!((address as i64) >> 43, 0 | -1) {
            return Err(format!(
                "unsupported V9 bypass effective address {address:#x}"
            ));
        }
        let physical = address & PHYSICAL_ADDRESS_MASK;
        if physical % 8 != 0 {
            return Err(format!(
                "misaligned V9 physical bypass read at {physical:#x}"
            ));
        }
        memory.read_bypass_xword(physical)
    }

    fn access_lsu_control(&mut self, op3: u32, address: u64, rd: u8) -> Result<(), String> {
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
        Ok(())
    }

    fn access_mmu_context(&mut self, op3: u32, address: u64, rd: u8) -> Result<(), String> {
        let source = self.read_reg(rd);
        let context = match address {
            0x08 => &mut self.primary_context,
            0x10 => &mut self.secondary_context,
            _ if address & 7 != 0 => {
                return Err(format!("misaligned V9 D-MMU access at {address:#x}"))
            }
            _ => return Err(format!("unsupported V9 D-MMU register at {address:#x}")),
        };
        if op3 == 0x1b {
            let value = u64::from(*context);
            self.write_reg(rd, value);
        } else {
            // IIi §15.9.3 defines bits 12:0; software must write reserved bits as zero.
            if source > 0x1fff {
                return Err(format!(
                    "unsupported V9 MMU context reserved bits in {source:#x}"
                ));
            }
            *context = source as u16;
        }
        Ok(())
    }

    pub fn run(&mut self, memory: &mut V9Memory, budget: usize) -> Result<(), String> {
        for _ in 0..budget {
            self.step(memory)?;
        }
        Ok(())
    }

    fn condition(condition: u32, flags: u8) -> bool {
        let n = flags & 0x8 != 0;
        let z = flags & 0x4 != 0;
        let v = flags & 0x2 != 0;
        let c = flags & 0x1 != 0;
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

    fn sub_flags(a: u64, b: u64, sign: u64) -> u8 {
        let result = a.wrapping_sub(b);
        let overflow = (a ^ b) & (a ^ result) & sign != 0;
        (u8::from(result & sign != 0) << 3)
            | (u8::from(a == b) << 2)
            | (u8::from(overflow) << 1)
            | u8::from(a < b)
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

    #[test]
    fn rdpr_reads_the_live_cwp_preserving_all_other_state() {
        let mut memory = V9Memory::new(4);
        for cwp in 0..8 {
            for rd in 0u8..32 {
                let mut cpu = V9Cpu::from(PowerOnReset {
                    pc: 0,
                    npc: 0x48, // RDPR in a delay slot follows the pending branch.
                    ..PowerOnReset::ultra_sparc_ii()
                });
                cpu.cwp = cwp;
                cpu.ccr = 0xa5;
                cpu.lsu_control = 2;
                for i in 1..32 {
                    cpu.write_reg(i, u64::MAX - u64::from(i));
                }
                memory.load_words(0, &[0x8152_4000 | (u32::from(rd) << 25)]);
                let mut expected = cpu.clone();
                expected.write_reg(rd, u64::from(cwp));
                expected.pc = 0x48;
                expected.npc = 0x4c;
                cpu.step(&mut memory).unwrap();
                assert_eq!(cpu, expected);
            }
        }
    }

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
        cpu.execute_fetched(0xc4d8_08a0, &mut UnmappedData).unwrap();
        assert_eq!(cpu.ccr(), 0x44);
    }

    #[test]
    fn ldxa_returns_the_full_contents_of_lsu_control() {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        // A parity-mask bit and both cache enables distinguish a register read
        // from assigning the POR constant. No guest LSU store is required here.
        cpu.lsu_control = (1 << 40) | 3;
        cpu.execute_fetched(0xc4d8_08a0, &mut UnmappedData).unwrap();
        assert_eq!(cpu.read_reg(2), 0x100_0000_0003);
    }
}
