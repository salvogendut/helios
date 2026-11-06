//! Initial SPARC V8 integer interpreter. Faults stop at the instruction boundary;
//! architectural trap entry, privileged registers, ASIs and the MMU are pending.

use crate::bus::{Bus, BusError};
use std::fmt;

pub const WINDOW_COUNT: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Icc {
    pub n: bool,
    pub z: bool,
    pub v: bool,
    /// Carry for addition; borrow for subtraction.
    pub c: bool,
}

impl Icc {
    fn matches(self, condition: u32) -> bool {
        let less = self.n ^ self.v;
        match condition {
            0 => false,
            1 => self.z,
            2 => self.z || less,
            3 => less,
            4 => self.c || self.z,
            5 => self.c,
            6 => self.n,
            7 => self.v,
            8 => true,
            9 => !self.z,
            10 => !self.z && !less,
            11 => !less,
            12 => !self.c && !self.z,
            13 => !self.c,
            14 => !self.n,
            15 => !self.v,
            _ => unreachable!("four-bit condition"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    UnsupportedInstruction,
    InstructionAccess(BusError),
    DataAccess(BusError),
    MisalignedJump(u32),
    WindowOverflow,
    WindowUnderflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuFault {
    pub pc: u32,
    pub npc: u32,
    pub instruction: Option<u32>,
    pub kind: FaultKind,
}

impl fmt::Display for CpuFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} at PC={:#010x}, nPC={:#010x}",
            self.kind, self.pc, self.npc
        )?;
        if let Some(instruction) = self.instruction {
            write!(f, ", instruction={instruction:#010x}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CpuFault {}

/// Addresses before a successfully executed instruction, suitable for tracing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub pc: u32,
    pub npc: u32,
    pub instruction: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Window {
    outs: [u32; 8],
    locals: [u32; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    globals: [u32; 8],
    windows: [Window; WINDOW_COUNT],
    cwp: usize,
    wim: u8,
    icc: Icc,
    pc: u32,
    npc: u32,
}

impl Cpu {
    /// Bare-metal entry state, not the workstation's hardware reset sequence.
    /// WIM starts at zero so tests can use every window without a trap handler.
    pub fn new(entry: u32) -> Self {
        Self {
            globals: [0; 8],
            windows: [Window::default(); WINDOW_COUNT],
            cwp: 0,
            wim: 0,
            icc: Icc::default(),
            pc: entry,
            npc: entry.wrapping_add(4),
        }
    }

    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn npc(&self) -> u32 {
        self.npc
    }
    pub fn icc(&self) -> Icc {
        self.icc
    }
    pub fn current_window(&self) -> usize {
        self.cwp
    }
    pub fn window_invalid_mask(&self) -> u8 {
        self.wim
    }

    /// Debug/test setup until privileged WIM instructions are implemented.
    pub fn set_window_invalid_mask(&mut self, mask: u8) {
        self.wim = mask;
    }

    pub fn read_reg(&self, index: u8) -> u32 {
        match index {
            0 => 0,
            1..=7 => self.globals[index as usize],
            8..=15 => self.windows[self.cwp].outs[(index - 8) as usize],
            16..=23 => self.windows[self.cwp].locals[(index - 16) as usize],
            24..=31 => self.windows[(self.cwp + 1) % WINDOW_COUNT].outs[(index - 24) as usize],
            _ => panic!("register index must be in 0..32"),
        }
    }

    pub fn write_reg(&mut self, index: u8, value: u32) {
        match index {
            0 => {}
            1..=7 => self.globals[index as usize] = value,
            8..=15 => self.windows[self.cwp].outs[(index - 8) as usize] = value,
            16..=23 => self.windows[self.cwp].locals[(index - 16) as usize] = value,
            24..=31 => {
                self.windows[(self.cwp + 1) % WINDOW_COUNT].outs[(index - 24) as usize] = value
            }
            _ => panic!("register index must be in 0..32"),
        }
    }

    pub fn step(&mut self, bus: &mut impl Bus) -> Result<Step, CpuFault> {
        let instruction = bus
            .read32(u64::from(self.pc))
            .map_err(|error| self.fault(None, FaultKind::InstructionAccess(error)))?;
        let step = Step {
            pc: self.pc,
            npc: self.npc,
            instruction,
        };
        self.execute(bus, instruction)
            .map_err(|kind| self.fault(Some(instruction), kind))?;
        Ok(step)
    }

    fn fault(&self, instruction: Option<u32>, kind: FaultKind) -> CpuFault {
        CpuFault {
            pc: self.pc,
            npc: self.npc,
            instruction,
            kind,
        }
    }

    fn execute(&mut self, bus: &mut impl Bus, word: u32) -> Result<(), FaultKind> {
        let rd = ((word >> 25) & 31) as u8;
        let rs1 = ((word >> 14) & 31) as u8;
        let op3 = (word >> 19) & 63;
        let a = self.read_reg(rs1);
        let b = if word & (1 << 13) != 0 {
            ((word << 19) as i32 >> 19) as u32
        } else {
            self.read_reg((word & 31) as u8)
        };
        let mut next_pc = self.npc;
        let mut next_npc = self.npc.wrapping_add(4);
        match word >> 30 {
            0 => match (word >> 22) & 7 {
                4 => self.write_reg(rd, (word & 0x3f_ffff) << 10), // SETHI (including NOP)
                2 => {
                    let condition = (word >> 25) & 15;
                    let taken = self.icc.matches(condition);
                    let annul = word & (1 << 29) != 0;
                    if taken {
                        let displacement = ((word << 10) as i32 >> 8) as u32;
                        next_npc = self.pc.wrapping_add(displacement);
                    }
                    if annul && (!taken || condition == 8) {
                        next_pc = next_npc;
                        next_npc = next_pc.wrapping_add(4);
                    }
                }
                _ => return Err(FaultKind::UnsupportedInstruction),
            },
            1 => {
                self.write_reg(15, self.pc);
                next_npc = self.pc.wrapping_add(word << 2);
            }
            2 => match op3 {
                0x00..=0x08 | 0x0c | 0x10..=0x18 | 0x1c => {
                    let (result, carry, overflow) = match op3 & 15 {
                        0 | 8 => {
                            let carry_in = u32::from(op3 & 8 != 0 && self.icc.c);
                            let wide = u64::from(a) + u64::from(b) + u64::from(carry_in);
                            let signed =
                                i64::from(a as i32) + i64::from(b as i32) + i64::from(carry_in);
                            (
                                wide as u32,
                                wide > u64::from(u32::MAX),
                                signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX),
                            )
                        }
                        4 | 12 => {
                            let borrow = u32::from(op3 & 8 != 0 && self.icc.c);
                            let subtrahend = u64::from(b) + u64::from(borrow);
                            let signed =
                                i64::from(a as i32) - i64::from(b as i32) - i64::from(borrow);
                            (
                                a.wrapping_sub(b).wrapping_sub(borrow),
                                u64::from(a) < subtrahend,
                                signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX),
                            )
                        }
                        1 => (a & b, false, false),
                        2 => (a | b, false, false),
                        3 => (a ^ b, false, false),
                        5 => (a & !b, false, false),
                        6 => (a | !b, false, false),
                        7 => (!(a ^ b), false, false),
                        _ => unreachable!("validated ALU opcode"),
                    };
                    if op3 & 0x10 != 0 {
                        self.icc = Icc {
                            n: result >> 31 != 0,
                            z: result == 0,
                            v: overflow,
                            c: carry,
                        };
                    }
                    self.write_reg(rd, result);
                }
                0x25..=0x27 => {
                    let amount = b & 31;
                    let result = match op3 {
                        0x25 => a << amount,
                        0x26 => a >> amount,
                        _ => ((a as i32) >> amount) as u32,
                    };
                    self.write_reg(rd, result);
                }
                0x38 => {
                    let target = a.wrapping_add(b);
                    if target & 3 != 0 {
                        return Err(FaultKind::MisalignedJump(target));
                    }
                    self.write_reg(rd, self.pc);
                    next_npc = target;
                }
                0x3c | 0x3d => {
                    let saving = op3 == 0x3c;
                    let new_cwp =
                        (self.cwp + if saving { WINDOW_COUNT - 1 } else { 1 }) % WINDOW_COUNT;
                    if self.wim & (1 << new_cwp) != 0 {
                        return Err(if saving {
                            FaultKind::WindowOverflow
                        } else {
                            FaultKind::WindowUnderflow
                        });
                    }
                    self.cwp = new_cwp;
                    self.write_reg(rd, a.wrapping_add(b));
                }
                _ => return Err(FaultKind::UnsupportedInstruction),
            },
            3 => {
                let address = u64::from(a.wrapping_add(b));
                let value = self.read_reg(rd);
                match op3 {
                    0x00 => self.write_reg(rd, bus.read32(address).map_err(FaultKind::DataAccess)?),
                    0x01 => self.write_reg(
                        rd,
                        u32::from(bus.read8(address).map_err(FaultKind::DataAccess)?),
                    ),
                    0x02 => self.write_reg(
                        rd,
                        u32::from(bus.read16(address).map_err(FaultKind::DataAccess)?),
                    ),
                    0x09 => self.write_reg(
                        rd,
                        bus.read8(address).map_err(FaultKind::DataAccess)? as i8 as i32 as u32,
                    ),
                    0x0a => self.write_reg(
                        rd,
                        bus.read16(address).map_err(FaultKind::DataAccess)? as i16 as i32 as u32,
                    ),
                    0x04 => bus.write32(address, value).map_err(FaultKind::DataAccess)?,
                    0x05 => bus
                        .write8(address, value as u8)
                        .map_err(FaultKind::DataAccess)?,
                    0x06 => bus
                        .write16(address, value as u16)
                        .map_err(FaultKind::DataAccess)?,
                    _ => return Err(FaultKind::UnsupportedInstruction),
                }
            }
            _ => unreachable!("two-bit opcode"),
        }
        self.pc = next_pc;
        self.npc = next_npc;
        Ok(())
    }
}
