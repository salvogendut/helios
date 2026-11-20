//! Ultra 5/10 boot flash and UltraSPARC-IIi power-on reset fixture.
//!
//! The Ultra 5/10 maps its 1 MiB flash at physical `0x01ff_f000_0000`
//! (Ultra 5 Service Manual 805-7763-12, table C-14). The UltraSPARC-IIi
//! virtual RED_state vector base is `0xffff_ffff_f000_0000`. Power-on reset
//! enters at offset `0x20` (UltraSPARC IIi User's Manual 805-0087-01,
//! section 17.3.2, tables 17-3 and 17-5).
//!
//! The reset fixture can inspect the fetch or retire one instruction on a V9 CPU.

use crate::v9::V9Cpu;

pub const FLASH_BASE: u64 = 0x01ff_f000_0000;
pub const FLASH_SIZE: usize = 1 << 20;
pub const POWER_ON_RESET_OFFSET: usize = 0x20;
const DEFAULT_RSTV_VA: u64 = 0xffff_ffff_f000_0000;
const PHYSICAL_ADDRESS_MASK: u64 = (1_u64 << 41) - 1;
pub const POWER_ON_RESET_PC: u64 = DEFAULT_RSTV_VA + POWER_ON_RESET_OFFSET as u64;
pub const POWER_ON_RESET_NPC: u64 = POWER_ON_RESET_PC + 4;

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
pub struct FirstFetch {
    pub pc: u64,
    pub npc: u64,
    pub physical_address: u64,
    pub flash_offset: usize,
    pub instruction: u32,
}

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
        if (cpu.pc(), cpu.npc()) != (POWER_ON_RESET_PC, POWER_ON_RESET_NPC) {
            return Err("Ultra 5 first instruction requires the power-on PC/nPC".into());
        }
        let fetch = self.fetch_reset_word(cpu.pc(), cpu.npc())?;
        cpu.execute_fetched(fetch.instruction, None)?;
        Ok(fetch)
    }

    fn fetch_reset_word(&self, pc: u64, npc: u64) -> Result<FirstFetch, String> {
        let physical_address = pc & PHYSICAL_ADDRESS_MASK;
        let instruction = self.read_word(physical_address)?;
        Ok(FirstFetch {
            pc,
            npc,
            physical_address,
            flash_offset: (physical_address - FLASH_BASE) as usize,
            instruction,
        })
    }
}
