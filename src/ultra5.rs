//! Ultra 5/10 boot flash and UltraSPARC-IIi power-on reset fixture.
//!
//! The Ultra 5/10 maps its 1 MiB flash at physical `0x01ff_f000_0000`
//! (Ultra 5 Service Manual 805-7763-12, table C-14). The UltraSPARC-IIi
//! virtual RED_state vector base is `0xffff_ffff_f000_0000`. Power-on reset
//! enters at offset `0x20` (UltraSPARC IIi User's Manual 805-0087-01,
//! section 17.3.2, tables 17-3 and 17-5).
//!
//! The reset fixture executes the ROM prefix and can single-step flash at the resulting PC.

use crate::v9::{UnmappedData, V9Cpu, V9DataBus, PHYSICAL_ADDRESS_MASK};

mod board;
mod dram;
mod keyboard_uart;
mod serial;
pub use board::Ultra5ResetBoard;

pub const FLASH_BASE: u64 = 0x01ff_f000_0000;
pub const FLASH_SIZE: usize = 1 << 20;
pub const POWER_ON_RESET_OFFSET: usize = 0x20;
const DEFAULT_RSTV_VA: u64 = 0xffff_ffff_f000_0000;
pub const POWER_ON_RESET_PC: u64 = DEFAULT_RSTV_VA + POWER_ON_RESET_OFFSET as u64;
pub const POWER_ON_RESET_NPC: u64 = POWER_ON_RESET_PC + 4;
/// PCIO BAR at configuration offset 0x14, chip select 1 at reset.
pub const RESET_NVRAM_BASE: u64 = 0x01ff_f100_0000;
pub const RESET_NVRAM_SIZE: usize = 0x2000;
const M48T59_REGISTERS: usize = 0x1ff0;

/// Retained M48T59 contents for the reset fixture. The top 16 offsets
/// contain device registers and cannot be served as plain SRAM.
pub struct ResetNvram {
    image: Box<[u8]>,
}

impl ResetNvram {
    pub fn new(image: Vec<u8>) -> Result<Self, String> {
        if image.len() != RESET_NVRAM_SIZE {
            return Err(format!(
                "Ultra 5 NVRAM image is {:#x} bytes, expected {RESET_NVRAM_SIZE:#x}",
                image.len()
            ));
        }
        Ok(Self {
            image: image.into_boxed_slice(),
        })
    }
    fn sram_index(&self, pa: u64, width: usize) -> Result<usize, String> {
        let offset = pa
            .checked_sub(RESET_NVRAM_BASE)
            .filter(|&offset| offset < RESET_NVRAM_SIZE as u64)
            .ok_or_else(|| format!("unmapped Ultra 5 NVRAM address {pa:#x}"))?
            as usize;
        if offset + width > M48T59_REGISTERS {
            return Err(format!(
                "unsupported Ultra 5 M48T59 register access at {pa:#x}"
            ));
        }
        Ok(offset)
    }
}

impl V9DataBus for ResetNvram {
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        let offset = self.sram_index(pa, 8)?;
        Ok(u64::from_be_bytes(
            self.image[offset..offset + 8].try_into().unwrap(),
        ))
    }
}

/// Original Ultra 5/10 OBP 3.31.0 first instruction at the POR vector.
pub const OBP_3_31_0_FIRST_WORD: u32 = 0xc4d8_08a0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerOnReset {
    pub pc: u64,
    pub npc: u64,
    pub memory_model_tso: bool,
    pub red_state: bool,
    pub fpu_enabled: bool,
    /// Full virtual address width. PSTATE.AM is clear at power-on.
    pub address_mask_64: bool,
    pub privileged: bool,
    pub interrupts_enabled: bool,
    pub alternate_globals: bool,
    pub current_little_endian: bool,
    pub trap_little_endian: bool,
    pub interrupt_globals: bool,
    pub mmu_globals: bool,
}

impl PowerOnReset {
    pub const fn ultra_sparc_ii() -> Self {
        Self {
            pc: POWER_ON_RESET_PC,
            npc: POWER_ON_RESET_NPC,
            memory_model_tso: true,
            red_state: true,
            fpu_enabled: true,
            address_mask_64: true,
            privileged: true,
            interrupts_enabled: false,
            alternate_globals: true,
            current_little_endian: false,
            trap_little_endian: false,
            interrupt_globals: false,
            mmu_globals: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootFlash {
    image: Box<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlashFetch {
    pub pc: u64,
    pub npc: u64,
    pub physical_address: u64,
    pub flash_offset: usize,
    pub instruction: u32,
}

/// Fetch record returned by the original one-instruction checkpoint.
pub type FirstFetch = FlashFetch;

impl BootFlash {
    pub fn new(image: Vec<u8>) -> Result<Self, String> {
        if image.len() != FLASH_SIZE {
            return Err(format!(
                "Ultra 5 flash image is {:#x} bytes, expected {FLASH_SIZE:#x}",
                image.len()
            ));
        }
        Ok(Self {
            image: image.into_boxed_slice(),
        })
    }

    pub fn image(&self) -> &[u8] {
        &self.image
    }

    pub fn read_word(&self, address: u64) -> Result<u32, String> {
        let offset = address
            .checked_sub(FLASH_BASE)
            .ok_or_else(|| format!("address {address:#x} is below the Ultra 5 flash"))?;
        if offset % 4 != 0 {
            return Err(format!("misaligned Ultra 5 flash word at {address:#x}"));
        }
        let offset = usize::try_from(offset)
            .map_err(|_| format!("address {address:#x} is outside the Ultra 5 flash"))?;
        let end = offset
            .checked_add(4)
            .ok_or_else(|| format!("address {address:#x} overflows the Ultra 5 flash"))?;
        let bytes = self
            .image
            .get(offset..end)
            .ok_or_else(|| format!("address {address:#x} is outside the Ultra 5 flash"))?;
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
    }

    pub fn first_fetch(&self) -> Result<FirstFetch, String> {
        let reset = PowerOnReset::ultra_sparc_ii();
        self.fetch_reset_word(reset.pc, reset.npc)
    }

    /// Retire just the word fetched at the default power-on vector.
    pub fn execute_first_instruction(&self, cpu: &mut V9Cpu) -> Result<FirstFetch, String> {
        self.execute_reset_word_at(cpu, POWER_ON_RESET_PC)
    }

    /// Execute the two sequential flash words at POR. A second-word error retains
    /// the effects of the first instruction and leaves the second unretired.
    pub fn execute_first_two_instructions(
        &self,
        cpu: &mut V9Cpu,
    ) -> Result<[FlashFetch; 2], String> {
        let first = self.execute_first_instruction(cpu)?;
        let second = self.execute_reset_word_at(cpu, POWER_ON_RESET_NPC)?;
        Ok([first, second])
    }

    /// Retire the three sequential words at POR, stopping at the PC selected by word three.
    pub fn execute_reset_branch(&self, cpu: &mut V9Cpu) -> Result<[FlashFetch; 3], String> {
        let [first, second] = self.execute_first_two_instructions(cpu)?;
        let branch = self.execute_reset_word_at(cpu, POWER_ON_RESET_PC + 8)?;
        Ok([first, second, branch])
    }

    fn execute_reset_word_at(&self, cpu: &mut V9Cpu, pc: u64) -> Result<FlashFetch, String> {
        if (cpu.pc(), cpu.npc()) != (pc, pc + 4) {
            return Err(format!(
                "Ultra 5 reset step requires PC={pc:#x}, nPC={:#x}",
                pc + 4
            ));
        }
        self.step(cpu)
    }

    /// Retire one instruction from the current flash PC, including a branch target or delay slot.
    /// Only instruction fetches are mapped by this fixture.
    pub fn step(&self, cpu: &mut V9Cpu) -> Result<FlashFetch, String> {
        self.step_with_data(cpu, &mut UnmappedData)
    }

    /// Step original flash with a retained NVRAM image attached at its reset mapping.
    pub fn step_with_nvram(
        &self,
        cpu: &mut V9Cpu,
        nvram: &mut ResetNvram,
    ) -> Result<FlashFetch, String> {
        self.step_with_data(cpu, nvram)
    }

    fn step_with_data(
        &self,
        cpu: &mut V9Cpu,
        data: &mut dyn V9DataBus,
    ) -> Result<FlashFetch, String> {
        let fetch = self.fetch_reset_word(cpu.pc(), cpu.npc())?;
        cpu.execute_fetched(fetch.instruction, data)?;
        Ok(fetch)
    }

    fn fetch_reset_word(&self, pc: u64, npc: u64) -> Result<FlashFetch, String> {
        let physical_address = pc & PHYSICAL_ADDRESS_MASK;
        let instruction = self.read_word(physical_address)?;
        Ok(FlashFetch {
            pc,
            npc,
            physical_address,
            flash_offset: (physical_address - FLASH_BASE) as usize,
            instruction,
        })
    }
}
