//! SPARC V8 interpreter with privileged state, floating-point transfers,
//! precise traps, external interrupts and microSPARC-II memory access.

use crate::bus::{AccessContext, Bus, BusError, BusErrorKind};
use crate::fpu::{Action as FpAction, Fpu, Operation as FpOperation};
use crate::mmu::{MemoryError, MmuConfig, MmuFault, Srmmu, Width};
use std::fmt;
#[cfg(feature = "jit-native")]
use std::{collections::HashMap, rc::Rc};

pub const WINDOW_COUNT: usize = 8;

pub const PSR_ET: u32 = 1 << 5;
pub const PSR_PS: u32 = 1 << 6;
pub const PSR_S: u32 = 1 << 7;
pub const PSR_PIL: u32 = 0xf << 8;
pub const PSR_EF: u32 = 1 << 12;
pub const PSR_DE: u32 = 1 << 15;
/// Fujitsu MB86904: implementation 0, version 4.
pub const PSR_VERSION: u32 = 0x0400_0000;
pub const SUPERSPARC_PSR_VERSION: u32 = 0x4000_0000;
const PSR_CONTROL: u32 = PSR_ET | PSR_PS | PSR_S | PSR_PIL | PSR_EF | PSR_DE;

enum Fetch {
    Word(u32),
    Event(Step),
}

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
    /// These V8 machines have no optional non-FPU coprocessor.
    CpDisabled,
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
            Self::CpDisabled => 0x24,
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

impl CpuFault {
    pub fn is_power_off(&self) -> bool {
        matches!(
            self.kind,
            FaultKind::UnsupportedMemory(MemoryError::Bus(BusError {
                kind: BusErrorKind::PowerOff,
                ..
            }))
        )
    }
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

/// Non-user-writable CPU identity exposed through PSR and FSR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuConfig {
    pub psr_version: u32,
    pub fpu_version: u32,
}

impl CpuConfig {
    /// Fujitsu MB86904 microSPARC-II: IU/FPU version 4, Meiko FPU.
    pub const fn microsparc2() -> Self {
        Self {
            psr_version: PSR_VERSION,
            fpu_version: 4 << 17,
        }
    }

    /// TI SuperSPARC-II: IU version 3, FPU version 0.
    pub const fn supersparc2() -> Self {
        Self {
            psr_version: SUPERSPARC_PSR_VERSION,
            fpu_version: 0,
        }
    }
}

impl Default for CpuConfig {
    fn default() -> Self {
        Self::microsparc2()
    }
}

const DECODE_CACHE_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheOperand {
    Register(u8),
    Immediate(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecodedInstruction {
    Sethi {
        rd: u8,
        value: u32,
    },
    Alu {
        op3: u8,
        rd: u8,
        rs1: u8,
        rhs: CacheOperand,
    },
    Branch {
        condition: u8,
        annul: bool,
        displacement: u32,
    },
}

#[derive(Clone, Copy, Debug)]
struct DecodeEntry {
    pc: u32,
    word: u32,
    instruction: DecodedInstruction,
}

#[derive(Clone, Debug)]
struct DecodeCache {
    #[cfg(feature = "jit-profile")]
    phase: crate::jit_phase::PhaseSnapshot,
    entries: [Option<DecodeEntry>; DECODE_CACHE_ENTRIES],
    enabled: bool,
    #[cfg(feature = "jit-native")]
    native_enabled: bool,
    #[cfg(feature = "jit-native")]
    interpret_span_fallback: bool,
    #[cfg(feature = "jit-native")]
    native_count: u64,
    #[cfg(feature = "jit-native")]
    native_calls: u64,
    #[cfg(feature = "jit-native")]
    native_span_calls: u64,
    #[cfg(feature = "jit-native")]
    native_words: HashMap<u32, Option<Rc<helios_jit_native::NativeBlock>>>,
    #[cfg(feature = "jit-native")]
    native_spans: HashMap<Vec<u32>, Option<Rc<helios_jit_native::NativeBlock>>>,
    #[cfg(feature = "jit-native")]
    ram_pairs: HashMap<([u32; 2], bool), Rc<helios_jit_native::WordPair>>,
}

impl Default for DecodeCache {
    fn default() -> Self {
        Self {
            entries: [None; DECODE_CACHE_ENTRIES],
            #[cfg(feature = "jit-profile")]
            phase: crate::jit_phase::PhaseSnapshot::fresh(),
            enabled: false,
            #[cfg(feature = "jit-native")]
            native_enabled: false,
            #[cfg(feature = "jit-native")]
            interpret_span_fallback: false,
            #[cfg(feature = "jit-native")]
            native_count: 0,
            #[cfg(feature = "jit-native")]
            native_calls: 0,
            #[cfg(feature = "jit-native")]
            native_span_calls: 0,
            #[cfg(feature = "jit-native")]
            native_words: HashMap::new(),
            #[cfg(feature = "jit-native")]
            native_spans: HashMap::new(),
            #[cfg(feature = "jit-native")]
            ram_pairs: HashMap::new(),
        }
    }
}

impl PartialEq for DecodeCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for DecodeCache {}

impl DecodeCache {
    fn lookup(&self, pc: u32, word: u32) -> Option<DecodedInstruction> {
        if !self.enabled {
            return None;
        }
        let entry = self.entries[(pc >> 2) as usize & (DECODE_CACHE_ENTRIES - 1)]?;
        (entry.pc == pc && entry.word == word).then_some(entry.instruction)
    }

    fn insert(&mut self, pc: u32, word: u32, instruction: DecodedInstruction) {
        if !self.enabled {
            return;
        }
        self.entries[(pc >> 2) as usize & (DECODE_CACHE_ENTRIES - 1)] = Some(DecodeEntry {
            pc,
            word,
            instruction,
        });
    }

    fn clear(&mut self) {
        self.entries.fill(None);
    }

    #[cfg(feature = "jit-native")]
    fn set_native_enabled(&mut self, enabled: bool) {
        self.native_enabled = enabled;
        if !enabled {
            self.native_words.clear();
            self.native_spans.clear();
            self.ram_pairs.clear();
        }
    }

    #[cfg(feature = "jit-native")]
    fn native_word(&mut self, word: u32) -> Option<Rc<helios_jit_native::NativeBlock>> {
        if let Some(entry) = self.native_words.get(&word) {
            #[cfg(feature = "jit-profile")]
            {
                self.phase.counts.words.hits += 1;
            }
            return entry.clone();
        }
        if self.native_words.len() >= 4096 {
            #[cfg(feature = "jit-profile")]
            {
                self.phase.counts.words.evicted_entries += self.native_words.len() as u64;
            }
            self.native_words.clear();
        }
        #[cfg(feature = "jit-profile")]
        let start = std::time::Instant::now();
        let block = helios_jit_native::compile(&[word]).map(Rc::new);
        #[cfg(feature = "jit-profile")]
        {
            let stats = &mut self.phase.counts.words;
            stats.misses += 1;
            stats.compiled += u64::from(block.is_some());
            stats.compile_ns += start.elapsed().as_nanos() as u64;
        }
        self.native_words.insert(word, block.clone());
        block
    }

    #[cfg(feature = "jit-native")]
    fn ram_pair(
        &mut self,
        words: [u32; 2],
        little: bool,
    ) -> Option<Rc<helios_jit_native::WordPair>> {
        let key = (words, little);
        if let Some(pair) = self.ram_pairs.get(&key) {
            #[cfg(feature = "jit-profile")]
            {
                self.phase.counts.spans.hits += 1;
            }
            return Some(Rc::clone(pair));
        }
        #[cfg(feature = "jit-profile")]
        let start = std::time::Instant::now();
        let compiled = helios_jit_native::compile_word_pair(words[0], words[1], little);
        #[cfg(feature = "jit-profile")]
        {
            let stats = &mut self.phase.counts.spans;
            stats.misses += 1;
            stats.compiled += u64::from(compiled.is_some());
            stats.compile_ns += start.elapsed().as_nanos() as u64;
        }
        let pair = Rc::new(compiled?);
        if self.ram_pairs.len() >= 256 {
            #[cfg(feature = "jit-profile")]
            {
                self.phase.counts.spans.evicted_entries += self.ram_pairs.len() as u64;
            }
            self.ram_pairs.clear();
        }
        self.ram_pairs.insert(key, Rc::clone(&pair));
        Some(pair)
    }

    #[cfg(feature = "jit-native")]
    fn native_span(
        &mut self,
        words: &[u32],
        with_slot: bool,
    ) -> Option<Rc<helios_jit_native::NativeBlock>> {
        if let Some(entry) = self.native_spans.get(words) {
            #[cfg(feature = "jit-profile")]
            {
                self.phase.counts.spans.hits += 1;
            }
            return entry.clone();
        }
        if self.native_spans.len() >= 256 {
            #[cfg(feature = "jit-profile")]
            {
                self.phase.counts.spans.evicted_entries += self.native_spans.len() as u64;
            }
            self.native_spans.clear();
        }
        #[cfg(feature = "jit-profile")]
        let start = std::time::Instant::now();
        let block = if with_slot {
            helios_jit_native::compile_span_with_slot(words)
        } else {
            helios_jit_native::compile_span(words)
        }
        .map(Rc::new);
        #[cfg(feature = "jit-profile")]
        {
            let stats = &mut self.phase.counts.spans;
            stats.misses += 1;
            stats.compiled += u64::from(block.is_some());
            stats.compile_ns += start.elapsed().as_nanos() as u64;
        }
        self.native_spans.insert(words.to_vec(), block.clone());
        block
    }
}

#[cfg(feature = "jit-native")]
fn pack_native_icc(icc: Icc) -> u32 {
    u32::from(icc.n) | (u32::from(icc.z) << 1) | (u32::from(icc.v) << 2) | (u32::from(icc.c) << 3)
}

#[cfg(feature = "jit-native")]
fn unpack_native_icc(bits: u32) -> Icc {
    Icc {
        n: bits & 1 != 0,
        z: bits & 2 != 0,
        v: bits & 4 != 0,
        c: bits & 8 != 0,
    }
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
    identity: CpuConfig,
    pub mmu: Srmmu,
    pub fpu: Fpu,
    decode_cache: DecodeCache,
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
            identity: CpuConfig::default(),
            mmu: Srmmu::default(),
            fpu: Fpu::default(),
            decode_cache: DecodeCache::default(),
        }
    }

    /// CPU/MMU reset for the sun4m harness. Unspecified registers are zeroed
    /// deterministically; the physical bus and its RAM/ROM remain owned by Machine.
    pub fn reset_sun4m(&mut self) {
        let config = self.mmu.config();
        let identity = self.identity;
        let mxcc = self.mmu.mxcc_handle();
        let initiator = self.mmu.initiator();
        *self = Self::new(0);
        self.wim = 1;
        self.identity = identity;
        self.mmu = Srmmu::reset_with(config);
        self.mmu.set_mxcc(mxcc);
        self.mmu.set_initiator(initiator);
        self.mmu.reset_mxcc();
        self.fpu.set_version(identity.fpu_version);
    }

    /// Select a profile's physical-address/boot-window behavior before reset.
    pub fn use_mmu_config(&mut self, config: MmuConfig) {
        let mxcc = self.mmu.mxcc_handle();
        let initiator = self.mmu.initiator();
        self.mmu = Srmmu::reset_with(config);
        self.mmu.set_mxcc(mxcc);
        self.mmu.set_initiator(initiator);
        self.mmu.reset_mxcc();
    }

    /// Select the CPU model identity before power-on reset. MMU configuration
    /// remains separate because several CPU models can share a reference MMU.
    pub fn use_cpu_config(&mut self, config: CpuConfig) {
        self.identity = config;
        self.fpu.set_version(config.fpu_version);
    }

    /// Enable or disable the bounded host-side decoded-instruction cache.
    /// The cache is not architectural state and is excluded from `Cpu` equality.
    pub fn set_decode_cache_enabled(&mut self, enabled: bool) {
        self.decode_cache.enabled = enabled;
        if !enabled {
            self.decode_cache.clear();
        }
    }

    #[cfg(feature = "jit-native")]
    pub fn set_native_execution_enabled(&mut self, enabled: bool) {
        self.decode_cache.set_native_enabled(enabled);
    }

    #[cfg(feature = "jit-native")]
    pub fn set_interpret_span_fallback(&mut self, enabled: bool) {
        self.decode_cache.interpret_span_fallback = enabled;
    }

    #[cfg(feature = "jit-native")]
    pub fn native_execution_count(&self) -> u64 {
        self.decode_cache.native_count
    }

    #[cfg(feature = "jit-profile")]
    pub fn jit_phase(&self) -> crate::jit_phase::PhaseSnapshot {
        self.decode_cache.phase
    }

    /// Actual native invocations made by the ordinary bridge/pair path.
    #[cfg(feature = "jit-native")]
    pub fn native_execution_calls(&self) -> u64 {
        self.decode_cache.native_calls
    }

    #[cfg(feature = "jit-native")]
    pub fn native_span_count(&self) -> u64 {
        self.decode_cache.native_span_calls
    }

    #[cfg(feature = "jit-native")]
    pub fn native_execution_enabled(&self) -> bool {
        self.decode_cache.native_enabled
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

    /// Synthetic-test setup only. Guest software changes PC through execution.
    pub fn set_pc_npc(&mut self, pc: u32, npc: u32) {
        self.pc = pc;
        self.npc = npc;
    }
    pub fn npc(&self) -> u32 {
        self.npc
    }
    pub fn icc(&self) -> Icc {
        self.icc
    }
    /// Apply condition codes from the optional native backend.
    #[cfg(feature = "jit-native")]
    pub fn set_icc(&mut self, icc: Icc) {
        self.icc = icc;
    }

    pub fn current_window(&self) -> usize {
        self.cwp
    }
    pub fn window_invalid_mask(&self) -> u8 {
        self.wim
    }

    pub fn psr(&self) -> u32 {
        self.identity.psr_version
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
        let previous = bus.access_context();
        let context = AccessContext::new(self.mmu.initiator(), self.psr_control & PSR_S != 0);
        bus.set_access_context(context);
        let result = self.step_inner(bus, pending);
        #[cfg(feature = "jit-profile")]
        self.decode_cache.phase.counts.boundary(result);
        bus.set_access_context(previous);
        result
    }

    #[inline(always)]
    fn fetch_instruction(&mut self, bus: &mut impl Bus, pending: u16) -> Result<Fetch, CpuFault> {
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
                return Ok(Fetch::Event(Step::InterruptTaken {
                    pc,
                    npc,
                    level,
                    vector: self.tbr,
                }));
            }
        }
        let asi = if self.psr_control & PSR_S != 0 { 9 } else { 8 };
        let instruction = match self.mmu.load(bus, self.pc, asi, Width::Word) {
            Ok(word) => word,
            Err(error) => {
                return self
                    .handle_fault(None, Self::memory_fault(error, true))
                    .map(Fetch::Event)
            }
        };
        Ok(Fetch::Word(instruction))
    }

    fn step_inner(&mut self, bus: &mut impl Bus, pending: u16) -> Result<Step, CpuFault> {
        match self.fetch_instruction(bus, pending)? {
            Fetch::Word(instruction) => self.execute_fetched(bus, instruction, true),
            Fetch::Event(event) => Ok(event),
        }
    }

    #[inline(always)]
    fn execute_fetched(
        &mut self,
        bus: &mut impl Bus,
        instruction: u32,
        _allow_native_word: bool,
    ) -> Result<Step, CpuFault> {
        let step = Step::Executed {
            pc: self.pc,
            npc: self.npc,
            instruction,
        };
        #[cfg(feature = "jit-native")]
        if _allow_native_word && self.decode_cache.native_enabled {
            if let Some(block) = self.decode_cache.native_word(instruction) {
                self.execute_native_block(&block);
                return Ok(step);
            }
        }
        let result = if self.decode_cache.enabled {
            if let Some(decoded) = self.decode_cache.lookup(self.pc, instruction) {
                self.execute_decoded(decoded);
                Ok(())
            } else if let Some(decoded) = Self::decode_plan(instruction) {
                self.decode_cache.insert(self.pc, instruction, decoded);
                self.execute_decoded(decoded);
                Ok(())
            } else {
                self.execute(bus, instruction)
            }
        } else {
            self.execute(bus, instruction)
        };
        match result {
            Ok(()) => {
                #[cfg(feature = "jit-profile")]
                self.decode_cache
                    .phase
                    .counts
                    .retirement(instruction, self.decode_cache.native_enabled);
                Ok(step)
            }
            Err(kind) => self.handle_fault(Some(instruction), kind),
        }
    }

    #[cfg(feature = "jit-native")]
    fn execute_native_block(&mut self, block: &helios_jit_native::NativeBlock) -> usize {
        let mut state = helios_jit_native::State::default();
        for (index, slot) in state.regs.iter_mut().enumerate() {
            if block.inputs() & (1u32 << index) != 0 {
                *slot = self.read_reg(index as u8);
            }
        }
        state.icc = pack_native_icc(self.icc);
        let exit = block.execute(&mut state);
        let through_branch = block.instructions() - usize::from(block.has_delay_slot());
        let slot_executed = block.has_delay_slot() && exit.is_some_and(|pair| pair.pc_delta == 4);
        let retired = through_branch + usize::from(slot_executed);
        #[cfg(feature = "jit-profile")]
        {
            self.decode_cache.phase.counts.calls_by_length[retired - 1] += 1;
        }
        self.decode_cache.native_calls = self.decode_cache.native_calls.wrapping_add(1);
        if retired >= 2 {
            self.decode_cache.native_span_calls =
                self.decode_cache.native_span_calls.wrapping_add(1);
        }
        for (index, value) in state.regs.iter().enumerate() {
            if block.outputs() & (1u32 << index) != 0 {
                self.write_reg(index as u8, *value);
            }
        }
        if block.sets_icc() {
            self.icc = unpack_native_icc(state.icc);
        }
        let sequential_pc = self.npc.wrapping_add(((through_branch - 1) as u32) * 4);
        if let Some(exit) = exit {
            // A terminal branch follows at least one straight-line instruction.
            let branch_pc = sequential_pc.wrapping_sub(4);
            self.pc = branch_pc.wrapping_add(exit.pc_delta);
            self.npc = branch_pc.wrapping_add(exit.npc_delta);
            if slot_executed {
                self.pc = self.npc;
                self.npc = self.pc.wrapping_add(4);
            }
        } else {
            self.pc = sequential_pc;
            self.npc = sequential_pc.wrapping_add(4);
        }
        self.decode_cache.native_count =
            self.decode_cache.native_count.wrapping_add(retired as u64);
        retired
    }

    /// Resolve a register operand as it will be seen by the second instruction.
    /// Only a straight-line integer prefix can reach this path.
    #[cfg(feature = "jit-native")]
    fn reg_after_prefix(&self, reg: u8, prefix: helios_jit_native::Op) -> u32 {
        use helios_jit_native::{Op, Operand};
        if reg == 0 {
            return 0;
        }
        match prefix {
            Op::Sethi { rd, value } if rd == reg => value,
            Op::Alu { rd, op3, rs1, rhs } if rd == reg => {
                let a = self.read_reg(rs1);
                let b = match rhs {
                    Operand::Immediate(value) => value,
                    Operand::Register(r) => self.read_reg(r),
                };
                match op3 & 15 {
                    0 => a.wrapping_add(b),
                    1 => a & b,
                    2 => a | b,
                    3 => a ^ b,
                    4 => a.wrapping_sub(b),
                    _ => unreachable!("unsupported native ALU prefix"),
                }
            }
            _ => self.read_reg(reg),
        }
    }

    #[cfg(feature = "jit-native")]
    fn try_ram_pair(
        &mut self,
        bus: &mut crate::system_bus::SystemBus,
        first: u32,
        fetch: crate::mmu::NativeFetch,
    ) -> Result<Option<crate::jit::NativeSpanStep>, CpuFault> {
        let last = fetch.word;
        let prefix = helios_jit_native::decode(first).expect("validated prefix");
        let base = self.reg_after_prefix(((last >> 14) & 31) as u8, prefix);
        let offset = if last & (1 << 13) != 0 {
            ((last << 19) as i32 >> 19) as u32
        } else {
            self.reg_after_prefix((last & 31) as u8, prefix)
        };
        let address = base.wrapping_add(offset);
        let asi = if self.psr_control & PSR_S != 0 {
            11
        } else {
            10
        };
        let write = last & (4 << 19) != 0;
        if !self.mmu.native_word_ready(bus, address, asi, write) {
            return Ok(None);
        }
        let Some(pair) = self
            .decode_cache
            .ram_pair([first, last], self.psr_control & PSR_DE != 0)
        else {
            return Ok(None);
        };
        let mut state = helios_jit_native::State {
            icc: pack_native_icc(self.icc),
            ..Default::default()
        };
        for (reg, value) in state.regs.iter_mut().enumerate() {
            if pair.inputs & (1 << reg) != 0 {
                *value = self.read_reg(reg as u8);
            }
        }
        // Commit the second fetch before the single architectural data request.
        self.mmu.complete_native_fetch(bus, fetch);
        let word = self
            .mmu
            .native_word(bus, address, asi, write)
            .map_err(|error| CpuFault {
                pc: self.npc,
                npc: self.npc.wrapping_add(4),
                instruction: Some(last),
                kind: Self::memory_fault(error, false),
            })?;
        pair.execute(&mut state, word);
        for (reg, value) in state.regs.iter().enumerate() {
            if pair.outputs & (1 << reg) != 0 {
                self.write_reg(reg as u8, *value);
            }
        }
        if pair.sets_icc {
            self.icc = unpack_native_icc(state.icc);
        }
        let mut out = crate::jit::NativeSpanStep::default();
        for instruction in [first, last] {
            out.push(Step::Executed {
                pc: self.pc,
                npc: self.npc,
                instruction,
            });
            self.pc = self.npc;
            self.npc = self.pc.wrapping_add(4);
        }
        out.native_instructions = 2;
        self.decode_cache.native_count = self.decode_cache.native_count.wrapping_add(2);
        self.decode_cache.native_calls = self.decode_cache.native_calls.wrapping_add(1);
        self.decode_cache.native_span_calls = self.decode_cache.native_span_calls.wrapping_add(1);
        #[cfg(feature = "jit-profile")]
        {
            self.decode_cache.phase.counts.calls_by_length[1] += 1;
        }
        Ok(Some(out))
    }

    /// Bounded translated span for the ordinary UP boundary. `span_times`
    /// contains cumulative guest nanoseconds for spans of length 2..=16.
    #[cfg(feature = "jit-native")]
    pub(crate) fn step_native_span_with_interrupts(
        &mut self,
        bus: &mut crate::system_bus::SystemBus,
        pending: u16,
        span_time: &mut dyn FnMut(usize) -> u64,
        budget: usize,
    ) -> Result<crate::jit::NativeSpanStep, CpuFault> {
        let previous = bus.access_context();
        let context = AccessContext::new(self.mmu.initiator(), self.psr_control & PSR_S != 0);
        bus.set_access_context(context);
        let result = (|| {
            let instruction = match self.fetch_instruction(bus, pending)? {
                Fetch::Word(word) => word,
                Fetch::Event(event) => {
                    let mut outcome = crate::jit::NativeSpanStep::default();
                    outcome.push(event);
                    return Ok(outcome);
                }
            };
            let asi = if self.psr_control & PSR_S != 0 { 9 } else { 8 };
            if self.decode_cache.native_enabled
                && budget >= 2
                && self.pc.checked_add(4) == Some(self.npc)
                && self.pc & !0xfff == self.npc & !0xfff
                && helios_jit_native::decode(instruction).is_some()
            {
                let limit = bus.native_span_time_limit().map(|last_ns| {
                    let mut limit = 0;
                    for count in 2..=budget.min(helios_jit_native::MAX_BLOCK_INSNS) {
                        if span_time(count) <= last_ns {
                            limit = count;
                        } else {
                            break;
                        }
                    }
                    limit
                });
                if let Some(limit) = limit.filter(|limit| *limit >= 2) {
                    let mut words = [0u32; helios_jit_native::MAX_BLOCK_INSNS];
                    let mut proofs = [None; helios_jit_native::MAX_BLOCK_INSNS];
                    let mut available = 0usize;
                    let mut slot_index = None;
                    for index in 0..limit {
                        let address = self.pc.wrapping_add((index as u32) * 4);
                        if address & !0xfff != self.pc & !0xfff {
                            break;
                        }
                        let Some(proof) = self.mmu.native_fetch_proof(bus, address, asi) else {
                            break;
                        };
                        if index == 0 && proof.word != instruction {
                            break;
                        }
                        if index == 1
                            && proof.word >> 30 == 3
                            && matches!((proof.word >> 19) & 63, 0 | 4)
                            && bus.native_pair_ready(span_time(2))
                        {
                            if let Some(out) = self.try_ram_pair(bus, instruction, proof)? {
                                return Ok(out);
                            }
                        }
                        let Some(op) = helios_jit_native::decode_span(proof.word) else {
                            break;
                        };
                        let in_slot = slot_index == Some(index);
                        if in_slot && matches!(op, helios_jit_native::Op::Branch { .. }) {
                            break;
                        }
                        words[index] = proof.word;
                        proofs[index] = Some(proof);
                        available += 1;
                        if in_slot {
                            break;
                        }
                        if matches!(op, helios_jit_native::Op::Branch { .. }) {
                            if !op.can_fuse_delay_slot() {
                                break;
                            }
                            slot_index = Some(index + 1);
                        }
                    }
                    for count in (2..=available).rev() {
                        if !bus.native_pair_ready(span_time(count)) {
                            continue;
                        }
                        let with_slot = slot_index == Some(count - 1);
                        let Some(block) = self.decode_cache.native_span(&words[..count], with_slot)
                        else {
                            continue;
                        };
                        let through_branch = count - usize::from(with_slot);
                        for proof in proofs[1..through_branch].iter().filter_map(|proof| *proof) {
                            self.mmu.complete_native_fetch(bus, proof);
                        }
                        let start_pc = self.pc;
                        let retired = self.execute_native_block(&block);
                        if with_slot && retired == count {
                            // Only ALU/SETHI can have run between the proof and
                            // this completion. Annulled slots have no fetch effects.
                            self.mmu
                                .complete_native_fetch(bus, proofs[count - 1].unwrap());
                        }
                        let mut outcome = crate::jit::NativeSpanStep::default();
                        for (index, instruction) in words.iter().take(retired).enumerate() {
                            let pc = start_pc.wrapping_add((index as u32) * 4);
                            outcome.push(Step::Executed {
                                pc,
                                npc: if with_slot && index == count - 1 {
                                    self.pc
                                } else {
                                    pc.wrapping_add(4)
                                },
                                instruction: *instruction,
                            });
                        }
                        outcome.native_instructions = retired as u8;
                        return Ok(outcome);
                    }
                }
            }
            self.execute_fetched(bus, instruction, !self.decode_cache.interpret_span_fallback)
                .map(|step| {
                    let mut outcome = crate::jit::NativeSpanStep::default();
                    outcome.push(step);
                    outcome
                })
        })();
        #[cfg(feature = "jit-profile")]
        self.decode_cache
            .phase
            .counts
            .boundary(result.map(|out| out.get(0).expect("CPU returned an empty span")));
        bus.set_access_context(previous);
        result
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
        let control_mask = if self.mmu.config().reference_mmu {
            PSR_CONTROL
        } else {
            PSR_CONTROL & !PSR_DE
        };
        self.psr_control = value & control_mask;
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
        let little = self.psr_control & PSR_DE != 0;
        match op3 {
            0x20 | 0x21 => {
                let value = self
                    .mmu
                    .load(bus, address, asi, Width::Word)
                    .map_err(|error| Self::memory_fault(error, false))?;
                let value = if little { value.swap_bytes() } else { value };
                if op3 == 0x21 {
                    self.fpu.load_fsr(value);
                } else {
                    self.fpu.write_reg(rd, value);
                }
            }
            0x23 => {
                let mut words = self
                    .mmu
                    .load_doubleword(bus, address, asi)
                    .map_err(|error| Self::memory_fault(error, false))?;
                if little {
                    words = [words[1].swap_bytes(), words[0].swap_bytes()];
                }
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
                let value = if little { value.swap_bytes() } else { value };
                self.mmu
                    .store(bus, address, asi, Width::Word, value)
                    .map_err(|error| Self::memory_fault(error, false))?;
            }
            0x27 => {
                let mut words = [self.fpu.read_reg(rd & !1), self.fpu.read_reg(rd | 1)];
                if little {
                    words = [words[1].swap_bytes(), words[0].swap_bytes()];
                }
                self.mmu
                    .store_doubleword(bus, address, asi, words)
                    .map_err(|error| Self::memory_fault(error, false))?;
            }
            0x26 => {
                if let Some(entry) = self.fpu.deferred_entry() {
                    let mut words = [entry.pc, entry.instruction];
                    if little {
                        words = [words[0].swap_bytes(), words[1].swap_bytes()];
                    }
                    self.mmu
                        .store_doubleword(bus, address, asi, words)
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

    fn decode_plan(word: u32) -> Option<DecodedInstruction> {
        let op = word >> 30;
        let rd = ((word >> 25) & 31) as u8;
        if op == 0 {
            return match (word >> 22) & 7 {
                4 => Some(DecodedInstruction::Sethi {
                    rd,
                    value: (word & 0x3f_ffff) << 10,
                }),
                2 => Some(DecodedInstruction::Branch {
                    condition: ((word >> 25) & 15) as u8,
                    annul: word & (1 << 29) != 0,
                    displacement: ((word << 10) as i32 >> 8) as u32,
                }),
                _ => None,
            };
        }
        if op != 2 {
            return None;
        }
        let op3 = (word >> 19) & 63;
        if !matches!(op3, 0x00..=0x08 | 0x0c | 0x10..=0x18 | 0x1c) {
            return None;
        }
        let rs1 = ((word >> 14) & 31) as u8;
        let rhs = if word & (1 << 13) != 0 {
            CacheOperand::Immediate(((word << 19) as i32 >> 19) as u32)
        } else {
            CacheOperand::Register((word & 31) as u8)
        };
        Some(DecodedInstruction::Alu {
            op3: op3 as u8,
            rd,
            rs1,
            rhs,
        })
    }

    #[cfg(feature = "jit")]
    pub(crate) fn execute_jit_op(&mut self, op: crate::jit::Op) {
        use crate::jit::Operand;
        match op {
            crate::jit::Op::Sethi { rd, value } => self.write_reg(rd, value),
            crate::jit::Op::Alu { op3, rd, rs1, rhs } => {
                let b = match rhs {
                    Operand::Register(index) => self.read_reg(index),
                    Operand::Immediate(value) => value,
                };
                self.execute_alu(u32::from(op3), rd, rs1, b);
            }
        }
        let next_pc = self.npc;
        self.pc = next_pc;
        self.npc = next_pc.wrapping_add(4);
    }

    fn execute_decoded(&mut self, instruction: DecodedInstruction) {
        let mut next_pc = self.npc;
        let mut next_npc = self.npc.wrapping_add(4);
        match instruction {
            DecodedInstruction::Sethi { rd, value } => self.write_reg(rd, value),
            DecodedInstruction::Alu { op3, rd, rs1, rhs } => {
                let b = match rhs {
                    CacheOperand::Register(index) => self.read_reg(index),
                    CacheOperand::Immediate(value) => value,
                };
                self.execute_alu(u32::from(op3), rd, rs1, b);
            }
            DecodedInstruction::Branch {
                condition,
                annul,
                displacement,
            } => {
                let taken = self.icc.matches(u32::from(condition));
                if taken {
                    next_npc = self.pc.wrapping_add(displacement);
                }
                if annul && (!taken || condition == 8) {
                    next_pc = next_npc;
                    next_npc = next_pc.wrapping_add(4);
                }
            }
        }
        self.pc = next_pc;
        self.npc = next_npc;
    }

    fn execute_alu(&mut self, op3: u32, rd: u8, rs1: u8, b: u32) {
        let a = self.read_reg(rs1);
        let (result, carry, overflow) = match op3 & 15 {
            0 | 8 => {
                let carry_in = u32::from(op3 & 8 != 0 && self.icc.c);
                let wide = u64::from(a) + u64::from(b) + u64::from(carry_in);
                let signed = i64::from(a as i32) + i64::from(b as i32) + i64::from(carry_in);
                (
                    wide as u32,
                    wide > u64::from(u32::MAX),
                    signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX),
                )
            }
            4 | 12 => {
                let borrow = u32::from(op3 & 8 != 0 && self.icc.c);
                let subtrahend = u64::from(b) + u64::from(borrow);
                let signed = i64::from(a as i32) - i64::from(b as i32) - i64::from(borrow);
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
                7 => return Err(FaultKind::CpDisabled),         // CBccc, including never/annul
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
                0x36 | 0x37 => return Err(FaultKind::CpDisabled), // CPop1/CPop2
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
                    self.execute_alu(op3, rd, rs1, b);
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
                    } else if rs1 < 15 {
                        // V8 B.28: reserved RDASR 1..14 has undefined data,
                        // but must not raise illegal_instruction. Choose Y
                        // readback; this does not implement the V9 CCR/TICK.
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
            3 if matches!(op3, 0x30 | 0x31 | 0x33..=0x37) => {
                // V8 B.3/B.6 and C.9: privilege precedes cp_disabled for
                // STDCQ; absence precedes alignment and all data accesses.
                if op3 == 0x36 {
                    self.require_supervisor()?;
                }
                return Err(FaultKind::CpDisabled);
            }
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
                let little = self.psr_control & PSR_DE != 0 && !matches!(asi, 8 | 9);
                if matches!(operation, 0x03 | 0x07) && rd & 1 != 0 {
                    return Err(FaultKind::IllegalInstruction);
                }
                if operation == 0x03 {
                    let mut words = self
                        .mmu
                        .load_doubleword(bus, address, asi)
                        .map_err(|error| Self::memory_fault(error, false))?;
                    if little {
                        words = [words[0].swap_bytes(), words[1].swap_bytes()];
                    }
                    self.write_reg(rd, words[0]);
                    self.write_reg(rd + 1, words[1]);
                } else if operation == 0x07 {
                    let words = if little {
                        [
                            self.read_reg(rd).swap_bytes(),
                            self.read_reg(rd + 1).swap_bytes(),
                        ]
                    } else {
                        [self.read_reg(rd), self.read_reg(rd + 1)]
                    };
                    self.mmu
                        .store_doubleword(bus, address, asi, words)
                        .map_err(|error| Self::memory_fault(error, false))?;
                } else if matches!(operation, 0x0d | 0x0f) {
                    let value = self.read_reg(rd);
                    let value = if operation == 0x0f && little {
                        value.swap_bytes()
                    } else {
                        value
                    };
                    let result = if operation == 0x0f {
                        self.mmu.swap(bus, address, asi, value)
                    } else {
                        self.mmu.load_store_byte(bus, address, asi)
                    }
                    .map_err(|error| Self::memory_fault(error, false))?;
                    let result = if operation == 0x0f && little {
                        result.swap_bytes()
                    } else {
                        result
                    };
                    self.write_reg(rd, result);
                } else if matches!(operation, 0x04..=0x06) {
                    let value = self.read_reg(rd);
                    let value = if little {
                        match width {
                            Width::Half => u32::from((value as u16).swap_bytes()),
                            Width::Word => value.swap_bytes(),
                            Width::Byte => value,
                        }
                    } else {
                        value
                    };
                    self.mmu
                        .store(bus, address, asi, width, value)
                        .map_err(|error| Self::memory_fault(error, false))?;
                } else {
                    let value = self
                        .mmu
                        .load(bus, address, asi, width)
                        .map_err(|error| Self::memory_fault(error, false))?;
                    let value = if little {
                        match width {
                            Width::Half => u32::from((value as u16).swap_bytes()),
                            Width::Word => value.swap_bytes(),
                            Width::Byte => value,
                        }
                    } else {
                        value
                    };
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
