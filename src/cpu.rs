//! SPARC V8 interpreter with privileged state, floating-point transfers,
//! precise traps, external interrupts and microSPARC-II memory access.

use crate::bus::{Bus, BusError, BusErrorKind};
use crate::fpu::{Action as FpAction, Fpu, Operation as FpOperation};
use crate::mmu::{MemoryError, MmuFault, Srmmu, Width};
use std::fmt;

pub const WINDOW_COUNT: usize = 8;

pub const PSR_ET: u32 = 1 << 5;
pub const PSR_PS: u32 = 1 << 6;
pub const PSR_S: u32 = 1 << 7;
pub const PSR_PIL: u32 = 0xf << 8;
pub const PSR_EF: u32 = 1 << 12;
/// Fujitsu MB86904: implementation 0, version 4.
pub const PSR_VERSION: u32 = 0x0400_0000;
const PSR_CONTROL: u32 = PSR_ET | PSR_PS | PSR_S | PSR_PIL | PSR_EF;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Icc {
    pub n: bool,
    pub z: bool,
    pub v: bool,
    /// Carry for addition; borrow for subtraction.
    pub c: bool,
}

impl Icc {
    fn from_result(result: u32, overflow: bool, carry: bool) -> Self {
        Self {
            n: result >> 31 != 0,
            z: result == 0,
            v: overflow,
            c: carry,
        }
    }

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
    /// Missing emulator behavior, never delivered to the guest as an ISA trap.
    UnsupportedInstruction,
    UnsupportedMemory(MemoryError),
    /// Device engine host stop at a CPU boundary; not a guest CPU trap.
    DeviceDma(crate::dma::DmaError),
    IllegalInstruction,
    PrivilegedInstruction,
    FpDisabled,
    FpException,
    SoftwareTrap(u8),
    InstructionAccess(BusError),
    DataAccess(BusError),
    InstructionMmu(MmuFault),
    DataMmu(MmuFault),
    MisalignedJump(u32),
    WindowOverflow,
    WindowUnderflow,
    TagOverflow,
    DivisionByZero,
}

impl FaultKind {
    /// V8 trap type. MMU exceptions and physical bus errors have distinct vectors.
    pub fn trap_type(self) -> Option<u8> {
        Some(match self {
            Self::UnsupportedInstruction | Self::UnsupportedMemory(_) | Self::DeviceDma(_) => {
                return None
            }
            Self::InstructionMmu(_) => 0x01,
            Self::DataMmu(_) => 0x09,
            Self::IllegalInstruction => 0x02,
            Self::PrivilegedInstruction => 0x03,
            Self::FpDisabled => 0x04,
            Self::FpException => 0x08,
            Self::WindowOverflow => 0x05,
            Self::WindowUnderflow => 0x06,
            Self::TagOverflow => 0x0a,
            Self::DivisionByZero => 0x2a,
            Self::MisalignedJump(_) => 0x07,
            Self::InstructionAccess(e) | Self::DataAccess(e)
                if e.kind == BusErrorKind::Misaligned =>
            {
                0x07
            }
            Self::InstructionAccess(_) => 0x21,
            Self::DataAccess(_) => 0x29,
            Self::SoftwareTrap(number) => 0x80 | (number & 0x7f),
        })
    }
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

/// An execution outcome: CPU instruction/trap/interrupt, or machine-owned reset.
/// Trap entry does not execute the first handler instruction in the same step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Executed {
        pc: u32,
        npc: u32,
        instruction: u32,
    },
    TrapTaken {
        fault: CpuFault,
        trap_type: u8,
        vector: u32,
    },
    InterruptTaken {
        pc: u32,
        npc: u32,
        level: u8,
        vector: u32,
    },
    /// Emitted by Machine, never by Cpu alone. A guest store may complete and
    /// request reset in this step; None denotes a request already on the bus.
    SoftwareReset {
        pc: u32,
        npc: u32,
        instruction: Option<u32>,
    },
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
    psr_control: u32,
    tbr: u32,
    y: u32,
    error_state: Option<CpuFault>,
    pub mmu: Srmmu,
    pub fpu: Fpu,
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
            psr_control: PSR_S | PSR_PS,
            tbr: 0,
            y: 0,
            error_state: None,
            mmu: Srmmu::default(),
            fpu: Fpu::default(),
        }
    }

    /// CPU/MMU reset for the sun4m harness. Unspecified registers are zeroed
    /// deterministically; the physical bus and its RAM/ROM remain owned by Machine.
    pub fn reset_sun4m(&mut self) {
        *self = Self::new(0);
        self.wim = 1;
        self.mmu = Srmmu::reset();
    }

    /// Non-power-up reset retains integer registers, PS, CWP and TT
    /// (microSPARC-II sections 3.11.3 and 10.2). Other state uses our
    /// deterministic reset baseline.
    pub fn software_reset_sun4m(&mut self) {
        let globals = self.globals;
        let windows = self.windows;
        let cwp = self.cwp;
        let ps = self.psr_control & PSR_PS;
        let tt = self.tbr & 0xff0;
        self.reset_sun4m();
        self.globals = globals;
        self.windows = windows;
        self.cwp = cwp;
        self.psr_control = (self.psr_control & !PSR_PS) | ps;
        self.tbr = tt;
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

    pub fn psr(&self) -> u32 {
        PSR_VERSION
            | self.psr_control
            | self.cwp as u32
            | (u32::from(self.icc.n) << 23)
            | (u32::from(self.icc.z) << 22)
            | (u32::from(self.icc.v) << 21)
            | (u32::from(self.icc.c) << 20)
    }

    pub fn tbr(&self) -> u32 {
        self.tbr
    }
    pub fn y(&self) -> u32 {
        self.y
    }

    /// A precise trap with ET=0 latches the cause and prevents further execution.
    /// CPU/MMU reset or recreating the bare-metal CPU clears this latch.
    pub fn error_state(&self) -> Option<CpuFault> {
        self.error_state
    }

    /// Debug/test setup. Guest software uses WRWIM.
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
        self.step_with_interrupts(bus, 0)
    }

    /// Sample level-sensitive interrupt lines at the instruction boundary.
    /// Bit n requests level n; bit zero is ignored. The source stays asserted
    /// until its device is acknowledged. No instruction is fetched on entry.
    pub fn step_with_interrupts(
        &mut self,
        bus: &mut impl Bus,
        pending: u16,
    ) -> Result<Step, CpuFault> {
        if let Some(fault) = self.error_state {
            return Err(fault);
        }
        let pending = pending & !1;
        if pending != 0 && self.psr_control & PSR_ET != 0 {
            let level = (15 - pending.leading_zeros()) as u8;
            let pil = ((self.psr_control & PSR_PIL) >> 8) as u8;
            if level == 15 || level > pil {
                let pc = self.pc;
                let npc = self.npc;
                self.enter_trap(0x10 | level);
                return Ok(Step::InterruptTaken {
                    pc,
                    npc,
                    level,
                    vector: self.tbr,
                });
            }
        }
        let asi = if self.psr_control & PSR_S != 0 { 9 } else { 8 };
        bus.set_cpu_supervisor(self.psr_control & PSR_S != 0);
        let instruction = match self.mmu.load(bus, self.pc, asi, Width::Word) {
            Ok(word) => word,
            Err(error) => return self.handle_fault(None, Self::memory_fault(error, true)),
        };
        let step = Step::Executed {
            pc: self.pc,
            npc: self.npc,
            instruction,
        };
        match self.execute(bus, instruction) {
            Ok(()) => Ok(step),
            Err(kind) => self.handle_fault(Some(instruction), kind),
        }
    }

    fn memory_fault(error: MemoryError, fetch: bool) -> FaultKind {
        match error {
            MemoryError::Bus(error) if error.kind.is_host_stop() => {
                FaultKind::UnsupportedMemory(MemoryError::Bus(error))
            }
            MemoryError::Bus(error) if fetch => FaultKind::InstructionAccess(error),
            MemoryError::Bus(error) => FaultKind::DataAccess(error),
            MemoryError::Translation(error) if fetch => FaultKind::InstructionMmu(error),
            MemoryError::Translation(error) => FaultKind::DataMmu(error),
            other => FaultKind::UnsupportedMemory(other),
        }
    }

    fn handle_fault(
        &mut self,
        instruction: Option<u32>,
        kind: FaultKind,
    ) -> Result<Step, CpuFault> {
        let fault = self.fault(instruction, kind);
        let Some(trap_type) = kind.trap_type() else {
            return Err(fault);
        };
        if self.psr_control & PSR_ET == 0 {
            // V8 section 7.4: only a failing RETT writes TT on error-mode entry.
            if instruction.is_some_and(|word| word >> 30 == 2 && (word >> 19) & 63 == 0x39) {
                self.tbr = (self.tbr & 0xffff_f000) | (u32::from(trap_type) << 4);
            }
            self.error_state = Some(fault);
            return Err(fault);
        }
        self.enter_trap(trap_type);
        Ok(Step::TrapTaken {
            fault,
            trap_type,
            vector: self.tbr,
        })
    }

    fn enter_trap(&mut self, trap_type: u8) {
        let previous_supervisor = self.psr_control & PSR_S != 0;
        self.psr_control = (self.psr_control & !(PSR_ET | PSR_PS))
            | PSR_S
            | if previous_supervisor { PSR_PS } else { 0 };
        // Trap entry ignores WIM, including when servicing window overflow.
        self.cwp = (self.cwp + WINDOW_COUNT - 1) % WINDOW_COUNT;
        self.write_reg(17, self.pc);
        self.write_reg(18, self.npc);
        self.tbr = (self.tbr & 0xffff_f000) | (u32::from(trap_type) << 4);
        self.pc = self.tbr;
        self.npc = self.pc.wrapping_add(4);
    }

    fn require_supervisor(&self) -> Result<(), FaultKind> {
        if self.psr_control & PSR_S == 0 {
            Err(FaultKind::PrivilegedInstruction)
        } else {
            Ok(())
        }
    }

    fn write_psr(&mut self, value: u32) -> Result<(), FaultKind> {
        let cwp = (value & 31) as usize;
        if cwp >= WINDOW_COUNT {
            return Err(FaultKind::IllegalInstruction);
        }
        self.cwp = cwp;
        self.psr_control = value & PSR_CONTROL;
        self.icc = Icc {
            n: value & (1 << 23) != 0,
            z: value & (1 << 22) != 0,
            v: value & (1 << 21) != 0,
            c: value & (1 << 20) != 0,
        };
        Ok(())
    }

    fn fault(&self, instruction: Option<u32>, kind: FaultKind) -> CpuFault {
        CpuFault {
            pc: self.pc,
            npc: self.npc,
            instruction,
            kind,
        }
    }

    fn require_fpu(&self) -> Result<(), FaultKind> {
        if self.psr_control & PSR_EF == 0 {
            Err(FaultKind::FpDisabled)
        } else {
            Ok(())
        }
    }

    fn prepare_fp(&mut self, operation: FpOperation) -> Result<bool, FaultKind> {
        match self.fpu.prepare(operation) {
            FpAction::Execute => Ok(true),
            FpAction::Skip => Ok(false),
            FpAction::Trap => Err(FaultKind::FpException),
        }
    }

    fn fp_memory(
        &mut self,
        bus: &mut impl Bus,
        op3: u32,
        rd: u8,
        address: u32,
    ) -> Result<(), FaultKind> {
        if op3 == 0x26 {
            self.require_supervisor()?;
        }
        self.require_fpu()?;
        // Alignment has priority over a pending FP exception, which in turn
        // precedes data translation/access faults (V8 table 7-1).
        let alignment = if matches!(op3, 0x23 | 0x26 | 0x27) {
            8
        } else {
            4
        };
        if address % alignment != 0 {
            return Err(FaultKind::DataAccess(BusError {
                address: u64::from(address),
                kind: BusErrorKind::Misaligned,
            }));
        }
        let operation = if matches!(op3, 0x20 | 0x21 | 0x23) {
            FpOperation::Load
        } else {
            FpOperation::Store
        };
        if !self.prepare_fp(operation)? {
            return Ok(());
        }
        let asi = if self.psr_control & PSR_S != 0 {
            11
        } else {
            10
        };
        match op3 {
            0x20 | 0x21 => {
                let value = self
                    .mmu
                    .load(bus, address, asi, Width::Word)
                    .map_err(|error| Self::memory_fault(error, false))?;
                if op3 == 0x21 {
                    self.fpu.load_fsr(value);
                } else {
                    self.fpu.write_reg(rd, value);
                }
            }
            0x23 => {
                let words = self
                    .mmu
                    .load_doubleword(bus, address, asi)
                    .map_err(|error| Self::memory_fault(error, false))?;
                // V8 reserves rd[0]. Match MAME's decoder by ignoring
                // it; no optional invalid_fp_register trap is synthesized.
                self.fpu.write_reg(rd & !1, words[0]);
                self.fpu.write_reg(rd | 1, words[1]);
            }
            0x24 | 0x25 => {
                let value = if op3 == 0x25 {
                    self.fpu.fsr()
                } else {
                    self.fpu.read_reg(rd)
                };
                self.mmu
                    .store(bus, address, asi, Width::Word, value)
                    .map_err(|error| Self::memory_fault(error, false))?;
            }
            0x27 => {
                let words = [self.fpu.read_reg(rd & !1), self.fpu.read_reg(rd | 1)];
                self.mmu
                    .store_doubleword(bus, address, asi, words)
                    .map_err(|error| Self::memory_fault(error, false))?;
            }
            0x26 => {
                if let Some(entry) = self.fpu.deferred_entry() {
                    self.mmu
                        .store_doubleword(bus, address, asi, [entry.pc, entry.instruction])
                        .map_err(|error| Self::memory_fault(error, false))?;
                    self.fpu.finish_queue_store();
                } else {
                    // An empty queue traps immediately without a memory access.
                    self.fpu.empty_queue_error();
                    return Err(FaultKind::FpException);
                }
            }
            _ => unreachable!("floating-point memory opcode"),
        }
        Ok(())
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
                0 => return Err(FaultKind::IllegalInstruction), // UNIMP, including zero-filled RAM
                4 => self.write_reg(rd, (word & 0x3f_ffff) << 10), // SETHI (including NOP)
                2 | 6 => {
                    let condition = (word >> 25) & 15;
                    let taken = if (word >> 22) & 7 == 6 {
                        self.require_fpu()?;
                        self.prepare_fp(FpOperation::Branch)?;
                        self.fpu.matches(condition)
                    } else {
                        self.icc.matches(condition)
                    };
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
                0x34 | 0x35 => {
                    self.require_fpu()?;
                    if self.prepare_fp(FpOperation::Operate)? {
                        let opf = (word >> 5) & 0x1ff;
                        let rs2 = (word & 31) as u8;
                        let implemented = if op3 == 0x34 {
                            self.fpu.move_word(opf, rd, rs2)
                                || self.fpu.binary(opf, rd, rs1, rs2, self.pc, word)
                                || self.fpu.convert(opf, rd, rs2, self.pc, word)
                                || self.fpu.square_root(opf, rd, rs2, self.pc, word)
                        } else {
                            self.fpu.compare(opf, rs1, rs2, self.pc, word)
                        };
                        if !implemented {
                            return Err(FaultKind::UnsupportedInstruction);
                        }
                    }
                }
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
                        self.icc = Icc::from_result(result, overflow, carry);
                    }
                    self.write_reg(rd, result);
                }
                0x0a | 0x0b | 0x1a | 0x1b => {
                    let product = if op3 & 1 == 0 {
                        u64::from(a) * u64::from(b)
                    } else {
                        (i64::from(a as i32) * i64::from(b as i32)) as u64
                    };
                    self.y = (product >> 32) as u32;
                    self.write_reg(rd, product as u32);
                    if op3 & 0x10 != 0 {
                        self.icc = Icc::from_result(product as u32, false, false);
                    }
                }
                0x0e | 0x0f | 0x1e | 0x1f => {
                    if b == 0 {
                        return Err(FaultKind::DivisionByZero);
                    }
                    let dividend = (u64::from(self.y) << 32) | u64::from(a);
                    let (result, overflow) = if op3 & 1 == 0 {
                        let quotient = dividend / u64::from(b);
                        (
                            quotient.min(u64::from(u32::MAX)) as u32,
                            quotient > u64::from(u32::MAX),
                        )
                    } else {
                        // Widen before dividing, including i64::MIN / -1.
                        let dividend = i128::from(dividend as i64);
                        let divisor = i128::from(b as i32);
                        let quotient = dividend / divisor;
                        let min = i128::from(i32::MIN);
                        let max = i128::from(i32::MAX);
                        // microSPARC-II uses V8 method B: a negative fractional
                        // result below MIN also overflows when truncation is MIN.
                        let overflow = quotient < min
                            || quotient > max
                            || (quotient == min && dividend % divisor != 0);
                        (quotient.clamp(min, max) as i32 as u32, overflow)
                    };
                    self.write_reg(rd, result);
                    if op3 & 0x10 != 0 {
                        self.icc = Icc::from_result(result, overflow, false);
                    }
                    // V8 permits Y to be left unchanged; software must not rely
                    // on its value after division. No remainder is exposed here.
                }
                0x20..=0x23 => {
                    let ((result, carry), (_, arithmetic_overflow)) = if op3 & 1 == 0 {
                        (a.overflowing_add(b), (a as i32).overflowing_add(b as i32))
                    } else {
                        (a.overflowing_sub(b), (a as i32).overflowing_sub(b as i32))
                    };
                    let overflow = arithmetic_overflow || (a | b) & 3 != 0;
                    if op3 & 2 != 0 && overflow {
                        return Err(FaultKind::TagOverflow);
                    }
                    self.write_reg(rd, result);
                    self.icc = Icc::from_result(result, overflow, carry);
                }
                0x24 => {
                    let shifted = (a >> 1) | (u32::from(self.icc.n ^ self.icc.v) << 31);
                    let addend = if self.y & 1 != 0 { b } else { 0 };
                    let (result, carry) = shifted.overflowing_add(addend);
                    let (_, overflow) = (shifted as i32).overflowing_add(addend as i32);
                    self.y = (a << 31) | (self.y >> 1);
                    self.write_reg(rd, result);
                    self.icc = Icc::from_result(result, overflow, carry);
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
                0x28 => {
                    if word == 0x8143_c000 {
                        // STBAR: synchronous single-owner stores are already ordered.
                    } else if rs1 == 0 {
                        self.write_reg(rd, self.y);
                    } else {
                        return Err(FaultKind::UnsupportedInstruction); // implementation-specific ASR
                    }
                }
                0x29..=0x2b => {
                    self.require_supervisor()?;
                    let value = match op3 {
                        0x29 => self.psr(),
                        0x2a => u32::from(self.wim),
                        _ => self.tbr,
                    };
                    self.write_reg(rd, value);
                }
                0x30 => {
                    if rd != 0 {
                        return Err(FaultKind::UnsupportedInstruction);
                    }
                    self.y = a ^ b;
                }
                0x31..=0x33 => {
                    self.require_supervisor()?;
                    let value = a ^ b;
                    match op3 {
                        0x31 => self.write_psr(value)?,
                        0x32 => self.wim = value as u8,
                        _ => self.tbr = (value & 0xffff_f000) | (self.tbr & 0xff0),
                    }
                }
                0x38 => {
                    let target = a.wrapping_add(b);
                    if target & 3 != 0 {
                        return Err(FaultKind::MisalignedJump(target));
                    }
                    self.write_reg(rd, self.pc);
                    next_npc = target;
                }
                0x39 => {
                    self.require_supervisor()?;
                    if self.psr_control & PSR_ET != 0 {
                        return Err(FaultKind::IllegalInstruction);
                    }
                    let new_cwp = (self.cwp + 1) % WINDOW_COUNT;
                    if self.wim & (1 << new_cwp) != 0 {
                        return Err(FaultKind::WindowUnderflow);
                    }
                    let target = a.wrapping_add(b);
                    if target & 3 != 0 {
                        return Err(FaultKind::MisalignedJump(target));
                    }
                    self.cwp = new_cwp;
                    self.psr_control = (self.psr_control & !PSR_S)
                        | PSR_ET
                        | if self.psr_control & PSR_PS != 0 {
                            PSR_S
                        } else {
                            0
                        };
                    next_npc = target;
                }
                0x3a => {
                    if self.icc.matches((word >> 25) & 15) {
                        return Err(FaultKind::SoftwareTrap((a.wrapping_add(b) & 0x7f) as u8));
                    }
                }
                0x3b => {
                    // Prior writes are already visible to subsequent fetches.
                    // Also invalidate the addressed diagnostic I-cache tag.
                    self.mmu.flush_instruction(a.wrapping_add(b));
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
            3 if matches!(op3, 0x20 | 0x21 | 0x23..=0x27) => {
                self.fp_memory(bus, op3, rd, a.wrapping_add(b))?;
            }
            3 => {
                let operation = op3 & !0x10;
                let width = match operation {
                    0x00 | 0x03 | 0x04 | 0x07 | 0x0f => Width::Word,
                    0x01 | 0x05 | 0x09 | 0x0d => Width::Byte,
                    0x02 | 0x06 | 0x0a => Width::Half,
                    _ => return Err(FaultKind::UnsupportedInstruction),
                };
                let asi = if op3 & 0x10 != 0 {
                    self.require_supervisor()?;
                    if word & (1 << 13) != 0 {
                        return Err(FaultKind::IllegalInstruction);
                    }
                    ((word >> 5) & 0xff) as u8
                } else if self.psr_control & PSR_S != 0 {
                    11
                } else {
                    10
                };
                let address = a.wrapping_add(b);
                if matches!(operation, 0x03 | 0x07) && rd & 1 != 0 {
                    return Err(FaultKind::IllegalInstruction);
                }
                if operation == 0x03 {
                    let words = self
                        .mmu
                        .load_doubleword(bus, address, asi)
                        .map_err(|error| Self::memory_fault(error, false))?;
                    self.write_reg(rd, words[0]);
                    self.write_reg(rd + 1, words[1]);
                } else if operation == 0x07 {
                    let words = [self.read_reg(rd), self.read_reg(rd + 1)];
                    self.mmu
                        .store_doubleword(bus, address, asi, words)
                        .map_err(|error| Self::memory_fault(error, false))?;
                } else if matches!(operation, 0x0d | 0x0f) {
                    let value = self.read_reg(rd);
                    let result = if operation == 0x0f {
                        self.mmu.swap(bus, address, asi, value)
                    } else {
                        self.mmu.load_store_byte(bus, address, asi)
                    }
                    .map_err(|error| Self::memory_fault(error, false))?;
                    self.write_reg(rd, result);
                } else if matches!(operation, 0x04..=0x06) {
                    let value = self.read_reg(rd);
                    self.mmu
                        .store(bus, address, asi, width, value)
                        .map_err(|error| Self::memory_fault(error, false))?;
                } else {
                    let value = self
                        .mmu
                        .load(bus, address, asi, width)
                        .map_err(|error| Self::memory_fault(error, false))?;
                    self.write_reg(
                        rd,
                        match operation {
                            0x09 => value as i8 as i32 as u32,
                            0x0a => value as i16 as i32 as u32,
                            _ => value,
                        },
                    );
                }
            }
            _ => unreachable!("two-bit opcode"),
        }
        self.pc = next_pc;
        self.npc = next_npc;
        Ok(())
    }
}
