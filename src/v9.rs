//! Headless UltraSPARC V9 interpreter fixture. This is intentionally separate
//! from the V8 `Cpu`: it implements only the instruction subset exercised by
//! the V9 fixture and is not wired into workstation machine construction.

use crate::ultra5::PowerOnReset;
mod ecache;
mod tlb;
use ecache::Ecache;
pub(crate) use tlb::Translation;
use tlb::{Access, Tlb};

const WINDOW_COUNT: u8 = 8;
const WINDOW_MASK: u8 = WINDOW_COUNT - 1;
const GLOBAL_REGISTERS: usize = 4 * 8;
const REGISTER_COUNT: usize = GLOBAL_REGISTERS + WINDOW_COUNT as usize * 16;
const PSTATE_AG: u16 = 1;
const PSTATE_PRIV: u16 = 1 << 2;
const PSTATE_PEF: u16 = 1 << 4;
const PSTATE_RED: u16 = 1 << 5;
const PSTATE_MG: u16 = 1 << 10;
const PSTATE_IG: u16 = 1 << 11;
const GLOBAL_SELECT: u16 = PSTATE_AG | PSTATE_MG | PSTATE_IG;
const LSU_IM: u64 = 4;
const LSU_DM: u64 = 8;
pub(crate) const PHYSICAL_ADDRESS_MASK: u64 = (1 << 41) - 1;
const ASI_PHYS_USE_EC: u8 = 0x14;
const ASI_PHYS_BYPASS_EC_WITH_EBIT: u8 = 0x15;
const ASI_PHYS_BYPASS_EC_WITH_EBIT_LE: u8 = 0x1d;
const ASI_DMMU: u8 = 0x58;
const ASI_IMMU: u8 = 0x50;
const ASI_ITLB_DATA_ACCESS: u8 = 0x55;
const ASI_ITLB_TAG_READ: u8 = 0x56;
const ASI_DTLB_DATA_ACCESS: u8 = 0x5d;
const ASI_DTLB_TAG_READ: u8 = 0x5e;
const ASI_ESTATE_ERROR_EN: u8 = 0x4b;
const ASI_AFSR: u8 = 0x4c;
const ASI_UPA_CONFIG: u8 = 0x4a;
const ASI_ECACHE_TAG_DATA: u8 = 0x4e;
const ASI_ECACHE_WRITE: u8 = 0x76;
const ASI_ECACHE_READ: u8 = 0x7e;
const MAX_TL: u8 = 5;
// TI UltraSPARC-IIi mask 9.1, as reported by original Ultra 5/10 hardware.
const VERSION: u64 = 0x0017_0012_9100_0000 | ((MAX_TL as u64) << 8) | WINDOW_MASK as u64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TrapFrame {
    tpc: u64,
    tnpc: u64,
    tt: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V9Cpu {
    // Four global banks, followed by eight outs and eight locals per window.
    regs: [u64; REGISTER_COUNT],
    pc: u64,
    npc: u64,
    ccr: u8,
    cwp: u8,
    asi: u8,
    primary_context: u16,
    secondary_context: u16,
    instruction_tlb: Tlb,
    data_tlb: Tlb,
    pstate: u16,
    pil: u8,
    tba: u64,
    tl: u8,
    traps: [TrapFrame; MAX_TL as usize],
    softint: u16,
    cansave: u8,
    canrestore: u8,
    cleanwin: u8,
    otherwin: u8,
    wstate: u8,
    lsu_control: u64,
    estate_error_enable: u8,
    async_fault_status: u64,
    ecache: Ecache,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct V9Memory {
    bytes: Vec<u8>,
}

/// Data accesses are independent of the instruction source. A reset-time
/// peripheral may implement physical bypass reads without mapping virtual RAM.
pub(crate) trait V9DataBus {
    fn read_cacheable_xword(&mut self, pa: u64) -> Result<u64, String> {
        Err(format!(
            "unmapped V9 cacheable physical xword read at {pa:#x}"
        ))
    }
    fn write_cacheable_xword(&mut self, pa: u64, _value: u64) -> Result<(), String> {
        Err(format!(
            "unmapped V9 cacheable physical xword write at {pa:#x}"
        ))
    }
    fn read_cacheable_byte(&mut self, pa: u64) -> Result<u8, String> {
        let word = self.read_cacheable_xword(pa & !7)?;
        Ok((word >> ((7 - (pa & 7)) * 8)) as u8)
    }
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        Err(format!(
            "unmapped V9 physical read: ASI=0x15 PA={pa:#x} width=8"
        ))
    }
    fn read_bypass_byte(&mut self, pa: u64) -> Result<u8, String> {
        Err(format!("unmapped V9 physical byte read at {pa:#x}"))
    }
    fn write_bypass_byte(&mut self, pa: u64, value: u8) -> Result<(), String> {
        Err(format!(
            "unmapped V9 physical byte write at {pa:#x} value={value:#x}"
        ))
    }
    fn write_bypass_le_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        Err(format!(
            "unmapped V9 physical LE word write at {pa:#x} value={value:#x}"
        ))
    }
    fn write_bypass_xword(&mut self, pa: u64, _value: u64) -> Result<(), String> {
        Err(format!("unmapped V9 physical xword write at {pa:#x}"))
    }
    fn read_bypass_word(&mut self, pa: u64) -> Result<u32, String> {
        Err(format!("unmapped V9 physical word read at {pa:#x}"))
    }
    fn write_bypass_word(&mut self, pa: u64, _value: u32) -> Result<(), String> {
        Err(format!("unmapped V9 physical word write at {pa:#x}"))
    }
}

pub(crate) struct UnmappedData;
impl V9DataBus for UnmappedData {}

impl V9DataBus for V9Memory {
    fn read_cacheable_xword(&mut self, pa: u64) -> Result<u64, String> {
        self.try_read_u64(pa)
    }
    fn write_cacheable_xword(&mut self, pa: u64, value: u64) -> Result<(), String> {
        self.try_write_u64(pa, value)
    }
    fn read_cacheable_byte(&mut self, pa: u64) -> Result<u8, String> {
        self.read_bypass_byte(pa)
    }
    fn write_bypass_xword(&mut self, pa: u64, value: u64) -> Result<(), String> {
        self.try_write_u64(pa, value)
    }
    fn read_bypass_word(&mut self, pa: u64) -> Result<u32, String> {
        self.try_read_word(pa)
    }
    fn write_bypass_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        let at = self.checked_range(pa, 4, 4)?;
        self.bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        self.try_read_u64(pa)
    }
    fn read_bypass_byte(&mut self, pa: u64) -> Result<u8, String> {
        Ok(self.bytes[self.checked_range(pa, 1, 1)?])
    }
    fn write_bypass_byte(&mut self, pa: u64, value: u8) -> Result<(), String> {
        let at = self.checked_range(pa, 1, 1)?;
        self.bytes[at] = value;
        Ok(())
    }
    fn write_bypass_le_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        let at = self.checked_range(pa, 4, 4)?;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
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
            pstate: u16::from(reset.alternate_globals)
                | (u16::from(reset.interrupts_enabled) << 1)
                | (u16::from(reset.privileged) << 2)
                | (u16::from(!reset.address_mask_64) << 3)
                | (u16::from(reset.fpu_enabled) << 4)
                | (u16::from(reset.red_state) << 5)
                | (u16::from(reset.trap_little_endian) << 8)
                | (u16::from(reset.current_little_endian) << 9)
                | (u16::from(reset.mmu_globals) << 10)
                | (u16::from(reset.interrupt_globals) << 11),
            // IIi table 17-5 leaves CWP unknown at POR. Use the value observed on QEMU sun4u.
            cwp: 7,
            tl: MAX_TL,
            traps: {
                let mut frames = [TrapFrame::default(); MAX_TL as usize];
                frames[MAX_TL as usize - 1].tt = 1; // power_on_reset
                frames
            },
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
            instruction_tlb: Tlb::default(),
            data_tlb: Tlb::default(),
            pstate: 0,
            pil: 0,
            tba: 0,
            tl: 0,
            traps: [TrapFrame::default(); MAX_TL as usize],
            softint: 0,
            cansave: 0,
            canrestore: 0,
            cleanwin: 0,
            otherwin: 0,
            wstate: 0,
            lsu_control: 0,
            estate_error_enable: 0,
            async_fault_status: 0,
            ecache: Ecache::default(),
        }
    }

    fn physical_register(&self, index: u8) -> usize {
        let (window, offset) = match index {
            0..=7 => {
                let bank = match self.pstate & GLOBAL_SELECT {
                    PSTATE_AG => 1,
                    PSTATE_MG => 2,
                    PSTATE_IG => 3,
                    _ => 0,
                };
                return bank * 8 + usize::from(index);
            }
            8..=23 => (self.cwp, index - 8),
            // V9 ins alias the previous window's outs, including the 0/7 wrap.
            24..=31 => (self.cwp.wrapping_sub(1) & WINDOW_MASK, index - 24),
            _ => panic!("register index must be in 0..32"),
        };
        GLOBAL_REGISTERS + usize::from(window) * 16 + usize::from(offset)
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
        let fetch = self.instruction_address()?;
        let word = memory.try_read_word(fetch.pa)?;
        self.retire_fetch(fetch, word, memory)
    }

    pub(crate) fn instruction_address(&self) -> Result<Translation, String> {
        self.translate(self.pc, 4, Access::Fetch)
    }

    fn translate(&self, address: u64, width: u64, access: Access) -> Result<Translation, String> {
        let va = if self.pstate & (1 << 3) != 0 {
            address & 0xffff_ffff
        } else {
            address
        };
        let physical = Self::bypass_address(va, width)?;
        let enabled = match access {
            Access::Fetch => self.lsu_control & LSU_IM != 0 && self.pstate & PSTATE_RED == 0,
            _ => self.lsu_control & LSU_DM != 0,
        };
        if !enabled {
            if access == Access::Flush {
                return Err(format!("IIi FLUSH on side-effect bypass at VA={va:#x}"));
            }
            return Ok(Translation {
                pa: physical,
                cacheable: false,
                invert_endian: false,
                slot: None,
            });
        }
        let tlb = if access == Access::Fetch {
            &self.instruction_tlb
        } else {
            &self.data_tlb
        };
        let context = if self.tl == 0 {
            self.primary_context
        } else {
            0
        };
        tlb.lookup(va, context, self.pstate & PSTATE_PRIV != 0, access)
    }

    pub(crate) fn retire_fetch(
        &mut self,
        fetch: Translation,
        word: u32,
        memory: &mut dyn V9DataBus,
    ) -> Result<(), String> {
        // Make Used visible to the instruction itself. A directed write to
        // its own I-TLB slot can then replace that bit in program order.
        let previous = fetch
            .slot
            .map(|slot| (slot, self.instruction_tlb.set_used(slot, true)));
        let result = self.execute_fetched(word, memory);
        if result.is_err() {
            if let Some((slot, used)) = previous {
                self.instruction_tlb.set_used(slot, used);
            }
        }
        result
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
            1 => {
                let displacement = i64::from((word << 2) as i32);
                self.write_reg(15, self.pc);
                next_npc = self.pc.wrapping_add_signed(displacement);
            }
            2 => match op3 {
                0x02 => self.write_reg(rd, self.read_reg(rs1) | b),
                0x00 => self.write_reg(rd, self.read_reg(rs1).wrapping_add(b)),
                0x01 => self.write_reg(rd, self.read_reg(rs1) & b),
                0x03 => self.write_reg(rd, self.read_reg(rs1) ^ b),
                0x04 => self.write_reg(rd, self.read_reg(rs1).wrapping_sub(b)),
                0x05 => self.write_reg(rd, self.read_reg(rs1) & !b),
                0x09 => self.write_reg(rd, self.read_reg(rs1).wrapping_mul(b)), // MULX
                0x11 => {
                    let result = self.read_reg(rs1) & b;
                    let low = result as u32;
                    self.ccr = (u8::from((result as i64) < 0) << 7)
                        | (u8::from(result == 0) << 6)
                        | (u8::from((low as i32) < 0) << 3)
                        | (u8::from(low == 0) << 2);
                    self.write_reg(rd, result);
                }
                0x38 => {
                    if word & (1 << 13) == 0 && word & 0x1fe0 != 0 {
                        return Err(format!("unsupported V9 JMPL word {word:#x}"));
                    }
                    let target = self.read_reg(rs1).wrapping_add(b);
                    if target & 3 != 0 || !matches!((target as i64) >> 43, 0 | -1) {
                        return Err(format!("invalid V9 JMPL target {target:#x}"));
                    }
                    self.write_reg(rd, self.pc);
                    next_npc = target;
                }
                0x39 | 0x3c | 0x3d => {
                    // SAVE, RESTORE and RETURN read their operands in the old window.
                    if (word & (1 << 13) == 0 && word & 0x1fe0 != 0) || (op3 == 0x39 && rd != 0) {
                        return Err(format!("unsupported V9 window instruction {word:#x}"));
                    }
                    let sum = self.read_reg(rs1).wrapping_add(b);
                    match op3 {
                        0x3c => {
                            if self.cansave == 0 {
                                return Err("V9 window spill trap not implemented".into());
                            }
                            if self.cleanwin == self.canrestore {
                                return Err("V9 clean window trap not implemented".into());
                            }
                            self.cwp = (self.cwp + 1) & WINDOW_MASK;
                            self.cansave -= 1;
                            self.canrestore = (self.canrestore + 1) & WINDOW_MASK;
                        }
                        _ => {
                            if self.canrestore == 0 {
                                return Err("V9 window fill trap not implemented".into());
                            }
                            if op3 == 0x39 {
                                if sum & 3 != 0 || !matches!((sum as i64) >> 43, 0 | -1) {
                                    return Err(format!("invalid V9 RETURN target {sum:#x}"));
                                }
                                next_npc = sum;
                            }
                            self.cwp = self.cwp.wrapping_sub(1) & WINDOW_MASK;
                            self.canrestore -= 1;
                            self.cansave = (self.cansave + 1) & WINDOW_MASK;
                        }
                    }
                    if op3 != 0x39 {
                        self.write_reg(rd, sum);
                    }
                }
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
                0x28 if rs1 == 5 && word & 0x3fff == 0 => self.write_reg(rd, self.pc),
                0x28 if rs1 == 22 && word & 0x3fff == 0 => {
                    self.require_privilege("RD SOFTINT")?;
                    self.write_reg(rd, u64::from(self.softint));
                }
                // MEMBAR #Sync: all earlier operations in this single-CPU
                // fixture have already completed when an instruction retires.
                0x28 if word == 0x8143_e040 => {}
                0x3b if rd == 0 && (word & (1 << 13) != 0 || word & 0x1fe0 == 0) => {
                    // IIi §14.4.4. FLUSH checks the D-side mapping, without
                    // issuing a device read. Instruction stores are synchronous.
                    let access =
                        self.translate(self.read_reg(rs1).wrapping_add(b), 1, Access::Flush)?;
                    if let Some(slot) = access.slot {
                        self.data_tlb.set_used(slot, true);
                    }
                }
                0x2a => {
                    self.require_privilege("RDPR")?;
                    // V9 A.43: rs1 selects the privileged register; bits 13:0 are reserved.
                    if word & 0x3fff != 0 {
                        return Err(format!("unsupported V9 RDPR word {word:#x}"));
                    }
                    self.write_reg(rd, self.read_pr(rs1)?);
                }
                0x30 if rd == 3 => {
                    if word & (1 << 13) == 0 && word & 0x1fe0 != 0 {
                        return Err(format!("unsupported V9 WRASI word {word:#x}"));
                    }
                    self.asi = (self.read_reg(rs1) ^ b) as u8;
                }
                0x30 if rd == 22 => {
                    self.require_privilege("WR SOFTINT")?;
                    if word & (1 << 13) == 0 && word & 0x1fe0 != 0 {
                        return Err(format!("unsupported V9 WR SOFTINT word {word:#x}"));
                    }
                    let value = self.read_reg(rs1) ^ b;
                    self.softint = u16::try_from(value)
                        .map_err(|_| format!("unsupported IIi SOFTINT bits in {value:#x}"))?;
                }
                0x32 => {
                    self.require_privilege("WRPR")?;
                    let reserved_bits_set = word & (1 << 13) == 0 && word & 0x1fe0 != 0;
                    if reserved_bits_set {
                        return Err(format!("unsupported V9 WRPR word {word:#x}"));
                    }
                    self.write_pr(rd, self.read_reg(rs1) ^ b)?;
                }
                _ => return Err(format!("unsupported V9 op3 {op3:#x}")),
            },
            3 => match op3 {
                0x01 | 0x0b | 0x0e => {
                    let va = self.read_reg(rs1).wrapping_add(b);
                    let width = if op3 == 0x01 { 1 } else { 8 };
                    let kind = if op3 == 0x0e {
                        Access::Write
                    } else {
                        Access::Read
                    };
                    let mapping = self.translate(va, width, kind)?;
                    let little = mapping.invert_endian ^ (self.pstate & (1 << 9) != 0);
                    match op3 {
                        0x01 => {
                            let byte = if mapping.cacheable {
                                memory.read_cacheable_byte(mapping.pa)?
                            } else {
                                memory.read_bypass_byte(mapping.pa)?
                            };
                            self.write_reg(rd, u64::from(byte));
                        }
                        0x0b => {
                            let word = if mapping.cacheable {
                                memory.read_cacheable_xword(mapping.pa)?
                            } else {
                                memory.read_bypass_xword(mapping.pa)?
                            };
                            self.write_reg(rd, if little { word.swap_bytes() } else { word });
                        }
                        _ => {
                            let word = self.read_reg(rd);
                            let word = if little { word.swap_bytes() } else { word };
                            if mapping.cacheable {
                                memory.write_cacheable_xword(mapping.pa, word)?;
                            } else {
                                memory.write_bypass_xword(mapping.pa, word)?;
                            }
                        }
                    }
                    if let Some(slot) = mapping.slot {
                        self.data_tlb.set_used(slot, true);
                    }
                }
                0x10 | 0x11 | 0x14 | 0x15 | 0x1b | 0x1e => {
                    let asi = if word & (1 << 13) != 0 {
                        self.asi
                    } else {
                        (word >> 5) as u8
                    };
                    if asi < 0x80 && self.pstate & PSTATE_PRIV == 0 {
                        return Err(format!("privileged V9 alternate address space {asi:#x}"));
                    }
                    let address = self.read_reg(rs1).wrapping_add(b);
                    match (asi, op3) {
                        (ASI_PHYS_USE_EC, 0x1b) => {
                            let value =
                                memory.read_cacheable_xword(Self::bypass_address(address, 8)?)?;
                            self.write_reg(rd, value);
                        }
                        (ASI_PHYS_USE_EC, 0x1e) => {
                            memory.write_cacheable_xword(
                                Self::bypass_address(address, 8)?,
                                self.read_reg(rd),
                            )?;
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT | ASI_PHYS_BYPASS_EC_WITH_EBIT_LE, 0x10) => {
                            let value =
                                memory.read_bypass_word(Self::bypass_address(address, 4)?)?;
                            let value = if asi == ASI_PHYS_BYPASS_EC_WITH_EBIT_LE {
                                value.swap_bytes()
                            } else {
                                value
                            };
                            self.write_reg(rd, u64::from(value));
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT, 0x14) => {
                            memory.write_bypass_word(
                                Self::bypass_address(address, 4)?,
                                self.read_reg(rd) as u32,
                            )?;
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT | ASI_PHYS_BYPASS_EC_WITH_EBIT_LE, 0x1e) => {
                            let pa = Self::bypass_address(address, 8)?;
                            let value = self.read_reg(rd);
                            memory.write_bypass_xword(
                                pa,
                                if asi == ASI_PHYS_BYPASS_EC_WITH_EBIT_LE {
                                    value.swap_bytes()
                                } else {
                                    value
                                },
                            )?;
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT | ASI_PHYS_BYPASS_EC_WITH_EBIT_LE, 0x1b) => {
                            let pa = Self::bypass_address(address, 8)?;
                            let value = memory.read_bypass_xword(pa)?;
                            self.write_reg(
                                rd,
                                if asi == ASI_PHYS_BYPASS_EC_WITH_EBIT_LE {
                                    value.swap_bytes()
                                } else {
                                    value
                                },
                            );
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT, 0x11) => {
                            let value =
                                memory.read_bypass_byte(Self::bypass_address(address, 1)?)?;
                            self.write_reg(rd, u64::from(value));
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT, 0x15) => {
                            memory.write_bypass_byte(
                                Self::bypass_address(address, 1)?,
                                self.read_reg(rd) as u8,
                            )?;
                        }
                        (ASI_PHYS_BYPASS_EC_WITH_EBIT_LE, 0x14) => {
                            memory.write_bypass_le_word(
                                Self::bypass_address(address, 4)?,
                                self.read_reg(rd) as u32,
                            )?;
                        }
                        (0x45, 0x1b | 0x1e) => self.access_lsu_control(op3, address, rd)?,
                        (ASI_UPA_CONFIG, 0x1b) if address == 0 => {
                            self.write_reg(rd, self.ecache.upa_config());
                        }
                        (ASI_UPA_CONFIG, 0x1e) if address == 0 => {
                            self.ecache.set_upa_config(self.read_reg(rd))?;
                        }
                        (ASI_ECACHE_TAG_DATA, 0x1b) if address == 0 => {
                            self.write_reg(rd, self.ecache.tag_data());
                        }
                        (ASI_ECACHE_TAG_DATA, 0x1e) if address == 0 => {
                            self.ecache.set_tag_data(self.read_reg(rd));
                        }
                        (ASI_ECACHE_WRITE, 0x1e) => {
                            self.ecache.write(address, self.read_reg(rd))?;
                        }
                        (ASI_ECACHE_READ, 0x1b) => {
                            if let Some(value) = self.ecache.read(address)? {
                                self.write_reg(rd, value);
                            }
                        }
                        (ASI_ESTATE_ERROR_EN | ASI_AFSR, 0x1b | 0x1e) if address == 0 => {
                            if op3 == 0x1b {
                                let value = if asi == ASI_ESTATE_ERROR_EN {
                                    u64::from(self.estate_error_enable)
                                } else {
                                    self.async_fault_status
                                };
                                self.write_reg(rd, value);
                            } else if asi == ASI_ESTATE_ERROR_EN {
                                // IIi table 16-2. Bit 2 is reserved.
                                self.estate_error_enable = self.read_reg(rd) as u8 & 0x1b;
                            } else {
                                // Table 16-3. Only the sticky flags are write-one-to-clear.
                                self.async_fault_status &= !(self.read_reg(rd) & 0x0001_adf0_0000);
                            }
                        }
                        (ASI_IMMU | ASI_DMMU, 0x1b | 0x1e) if address == 0x30 => {
                            let source = self.read_reg(rd);
                            let tlb = if asi == ASI_IMMU {
                                &mut self.instruction_tlb
                            } else {
                                &mut self.data_tlb
                            };
                            if op3 == 0x1e {
                                tlb.stage_tag(source);
                            } else {
                                let tag = tlb.tag_access();
                                self.write_reg(rd, tag);
                            }
                        }
                        (ASI_ITLB_DATA_ACCESS | ASI_DTLB_DATA_ACCESS, 0x1b | 0x1e)
                        | (ASI_ITLB_TAG_READ | ASI_DTLB_TAG_READ, 0x1b) => {
                            let index = Tlb::index(address)?;
                            let source = self.read_reg(rd);
                            let instruction =
                                matches!(asi, ASI_ITLB_DATA_ACCESS | ASI_ITLB_TAG_READ);
                            let tlb = if instruction {
                                &mut self.instruction_tlb
                            } else {
                                &mut self.data_tlb
                            };
                            if op3 == 0x1e {
                                tlb.write_data(index, source, instruction);
                            } else {
                                let value = if matches!(asi, ASI_ITLB_TAG_READ | ASI_DTLB_TAG_READ)
                                {
                                    tlb.read_tag(index)
                                } else {
                                    tlb.read_data(index)
                                };
                                self.write_reg(rd, value);
                            }
                        }
                        (ASI_DMMU, 0x1b | 0x1e) => self.access_mmu_context(op3, address, rd)?,
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

    fn require_privilege(&self, operation: &str) -> Result<(), String> {
        if self.pstate & PSTATE_PRIV == 0 {
            return Err(format!("privileged V9 {operation} instruction"));
        }
        Ok(())
    }

    fn read_pr(&self, selector: u8) -> Result<u64, String> {
        Ok(match selector {
            0 => self.traps[self.trap_index()?].tpc,
            1 => self.traps[self.trap_index()?].tnpc,
            3 => u64::from(self.traps[self.trap_index()?].tt),
            5 => self.tba,
            6 => u64::from(self.pstate),
            7 => u64::from(self.tl),
            8 => u64::from(self.pil),
            9 => u64::from(self.cwp),
            10 => u64::from(self.cansave),
            11 => u64::from(self.canrestore),
            12 => u64::from(self.cleanwin),
            13 => u64::from(self.otherwin),
            14 => u64::from(self.wstate),
            31 => VERSION,
            _ => return Err(format!("unsupported V9 RDPR register {selector}")),
        })
    }

    fn trap_index(&self) -> Result<usize, String> {
        self.tl
            .checked_sub(1)
            .map(usize::from)
            .ok_or_else(|| "illegal V9 trap register access at TL=0".into())
    }

    fn write_pr(&mut self, selector: u8, value: u64) -> Result<(), String> {
        // IIi §14.2.1 truncates each window counter independently to three bits.
        let count = value as u8 & WINDOW_MASK;
        match selector {
            0 | 1 | 3 => {
                let index = self.trap_index()?;
                // IIi stores VA[43:2] and sign-extends the trap PC on read.
                let pc = (((value << 20) as i64 >> 20) as u64) & !3;
                let frame = &mut self.traps[index];
                match selector {
                    0 => frame.tpc = pc,
                    1 => frame.tnpc = pc,
                    _ => frame.tt = value as u16 & 0x1ff,
                }
            }
            5 => {
                if !matches!((value as i64) >> 43, 0 | -1) {
                    return Err(format!("unsupported IIi TBA address {value:#x}"));
                }
                self.tba = value & !0x7fff;
            }
            6 => {
                // Physical fetch, big-endian data, TSO, and interrupts disabled.
                let supported = GLOBAL_SELECT | PSTATE_PRIV | PSTATE_PEF | PSTATE_RED;
                if value & !u64::from(supported) != 0
                    || (value as u16 & GLOBAL_SELECT).count_ones() > 1
                {
                    return Err(format!("unsupported IIi PSTATE {value:#x}"));
                }
                self.pstate = value as u16;
            }
            8 => self.pil = value as u8 & 0xf,
            7 if value <= u64::from(MAX_TL) => self.tl = value as u8,
            7 => return Err(format!("unsupported IIi trap level {value:#x}")),
            9 => self.cwp = count,
            10 => self.cansave = count,
            11 => self.canrestore = count,
            12 => self.cleanwin = count,
            13 => self.otherwin = count,
            14 => self.wstate = value as u8 & 0x3f,
            _ => return Err(format!("unsupported V9 WRPR register {selector}")),
        }
        Ok(())
    }

    fn bypass_address(address: u64, width: u64) -> Result<u64, String> {
        // IIi §§4.2 and 15.10: 44-bit sign-extended VA, 41-bit physical bus.
        if !matches!((address as i64) >> 43, 0 | -1) {
            return Err(format!(
                "unsupported V9 bypass effective address {address:#x}"
            ));
        }
        let physical = address & PHYSICAL_ADDRESS_MASK;
        if physical % width != 0 {
            return Err(format!(
                "misaligned V9 physical bypass access at {physical:#x}"
            ));
        }
        Ok(physical)
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
            // IIi A.6. IC/DC are retained; IM/DM control independent TLBs.
            const CACHE_ENABLES: u64 = 0b11;
            if value & !(CACHE_ENABLES | LSU_IM | LSU_DM) != 0 {
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
    fn mulx_uses_full_width_operands_and_preserves_ccr() {
        let mut cpu = V9Cpu::new(0);
        cpu.ccr = 0xa5;
        cpu.write_reg(1, 0x8000_0000_0000_0003);
        cpu.write_reg(2, 0x1_0000_0002);
        cpu.execute_fetched(0x8248_4002, &mut UnmappedData).unwrap(); // mulx %g1,%g2,%g1
        assert_eq!(cpu.read_reg(1), 0x3_0000_0006);
        cpu.execute_fetched(0x8448_7fff, &mut UnmappedData).unwrap(); // mulx %g1,-1,%g2
        assert_eq!(cpu.read_reg(2), 0xffff_fffc_ffff_fffa);
        assert_eq!((cpu.pc(), cpu.npc(), cpu.ccr()), (8, 12, 0xa5));
    }

    #[test]
    fn physical_cacheable_xwords_require_privilege_alignment_and_a_complete_word() {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        let mut mem = V9Memory::new(20);
        const STORE: u32 = 0xc4f0_4280; // stxa %g2,[%g1] ASI 0x14
        const LOAD: u32 = 0xc6d8_4280; // ldxa [%g1] ASI 0x14,%g3
        cpu.write_reg(1, 8);
        cpu.write_reg(2, 0xfedc_ba98_7654_3210);
        cpu.execute_fetched(STORE, &mut mem).unwrap();
        assert_eq!(
            (mem.read_word(8), mem.read_word(12)),
            (0xfedc_ba98, 0x7654_3210)
        );
        cpu.execute_fetched(LOAD, &mut mem).unwrap();
        assert_eq!(cpu.read_reg(3), cpu.read_reg(2));
        cpu.write_reg(2, 0);
        for (address, pstate) in [(12, PSTATE_PRIV), (16, PSTATE_PRIV), (8, 0)] {
            cpu.pstate = pstate;
            cpu.write_reg(1, address);
            let before = cpu.clone();
            let data = mem.clone();
            for word in [LOAD, STORE] {
                assert!(cpu.execute_fetched(word, &mut mem).is_err());
                assert_eq!(cpu, before);
                assert_eq!(mem, data);
            }
        }
    }

    #[test]
    fn window_operations_read_before_rotation_and_return_slot_runs_in_caller() {
        let mut cpu = V9Cpu::new(0x100);
        cpu.cwp = 7;
        cpu.cansave = 6;
        cpu.cleanwin = 7;
        cpu.ccr = 0xa5;
        cpu.write_reg(8, 0x1c);
        cpu.write_reg(15, 0x80);
        cpu.execute_fetched(0x91e2_2004, &mut UnmappedData).unwrap(); // save %o0,4,%o0
        assert_eq!(
            (cpu.cwp, cpu.cansave, cpu.canrestore, cpu.cleanwin),
            (0, 5, 1, 7)
        );
        assert_eq!(
            (cpu.read_reg(8), cpu.read_reg(24), cpu.read_reg(31)),
            (0x20, 0x1c, 0x80)
        );
        let mut restore = cpu.clone();
        restore
            .execute_fetched(0x91ea_2008, &mut UnmappedData)
            .unwrap(); // restore %o0,8,%o0
        assert_eq!(
            (
                restore.cwp,
                restore.cansave,
                restore.canrestore,
                restore.read_reg(8)
            ),
            (7, 6, 0, 0x28)
        );
        cpu.execute_fetched(0x81cf_e008, &mut UnmappedData).unwrap(); // return %i7+8
        assert_eq!((cpu.cwp, cpu.pc, cpu.npc), (7, 0x108, 0x88));
        cpu.execute_fetched(0x8202_2001, &mut UnmappedData).unwrap(); // add %o0,1,%g1
        assert_eq!(
            (cpu.pc, cpu.npc, cpu.read_reg(1), cpu.ccr),
            (0x88, 0x8c, 0x1d, 0xa5)
        );
        assert_eq!((cpu.cansave, cpu.canrestore, cpu.cleanwin), (6, 0, 7));
    }

    #[test]
    fn unsupported_window_traps_leave_the_faulting_instruction_intact() {
        for (word, save, restore, clean, input, reason) in [
            (0x81e0_2000, 0, 0, 0, 0, "spill"),
            (0x81e0_2000, 5, 1, 1, 0, "clean"),
            (0x81e8_2000, 6, 0, 7, 0, "fill"),
            (0x81cf_e008, 6, 0, 7, 0, "fill"),
            (0x81cf_e008, 5, 1, 7, 1, "target"),
            (0x81cf_e008, 5, 1, 7, 1 << 44, "target"),
        ] {
            let mut cpu = V9Cpu::new(0x14);
            cpu.cansave = save;
            cpu.canrestore = restore;
            cpu.cleanwin = clean;
            cpu.write_reg(31, input);
            let before = cpu.clone();
            assert!(cpu
                .execute_fetched(word, &mut UnmappedData)
                .unwrap_err()
                .contains(reason));
            assert_eq!(cpu, before);
        }
    }

    #[test]
    fn revision_and_error_registers_obey_privilege_width_and_sticky_bits() {
        let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
        let ver = 0x8357_c000; // rdpr %ver,%g1
        cpu.execute_fetched(ver, &mut UnmappedData).unwrap();
        assert_eq!(cpu.read_reg(1), 0x0017_0012_9100_0507);
        let store_en = 0xc2f0_0960;
        let load_en = 0xc2d8_0960;
        let store_afsr = 0xc2f0_0980;
        let load_afsr = 0xc2d8_0980;
        cpu.write_reg(1, u64::MAX);
        cpu.execute_fetched(store_en, &mut UnmappedData).unwrap();
        cpu.execute_fetched(load_en, &mut UnmappedData).unwrap();
        assert_eq!(cpu.read_reg(1), 0x1b);
        cpu.async_fault_status = (1 << 32) | (1 << 27) | (1 << 20);
        cpu.write_reg(1, 1 << 27);
        cpu.execute_fetched(store_afsr, &mut UnmappedData).unwrap();
        for _ in 0..2 {
            cpu.execute_fetched(load_afsr, &mut UnmappedData).unwrap();
            assert_eq!(cpu.read_reg(1), (1 << 32) | (1 << 20));
        }
        cpu.write_reg(1, u64::MAX);
        cpu.execute_fetched(store_afsr, &mut UnmappedData).unwrap();
        assert_eq!(cpu.async_fault_status, 0);
        assert_eq!(cpu.estate_error_enable, 0x1b);
        cpu.write_reg(2, 8);
        for invalid in [store_en | 2, load_afsr | 2, 0xc2a0_0980, 0xbf90_2000] {
            let before = cpu.clone();
            assert!(cpu.execute_fetched(invalid, &mut UnmappedData).is_err());
            assert_eq!(cpu, before);
        }
        let mut user = V9Cpu::new(0);
        for restricted in [ver, load_en, store_en, load_afsr, store_afsr] {
            let before = user.clone();
            assert!(user
                .execute_fetched(restricted, &mut UnmappedData)
                .unwrap_err()
                .contains("privileged"));
            assert_eq!(user, before);
        }
    }

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
        expected.write_reg(2, cpu.lsu_control);
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
