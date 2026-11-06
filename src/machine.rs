//! Bare-metal harness and target limits. SS-5 devices and reset mapping are not
//! installed yet; no firmware boot is implied by constructing this machine.

use crate::bus::{BusError, MapError, MemoryBus};
use crate::cpu::{Cpu, CpuFault, Step};
use std::fmt;

pub const TARGET: &str = "SPARCstation 5 (sun4m), SPARC V8 / microSPARC-II";
pub const MAX_RAM_BYTES: usize = 256 * 1024 * 1024;
pub const DEFAULT_RAM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum MachineError {
    InvalidRamSize,
    Memory(MapError),
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRamSize => {
                f.write_str("RAM must be a nonzero multiple of 4 bytes, at most 256 MiB")
            }
            Self::Memory(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for MachineError {}

#[derive(Debug)]
pub struct Machine {
    pub cpu: Cpu,
    pub bus: MemoryBus,
}

impl Machine {
    pub fn bare_metal(ram_bytes: usize, entry: u32) -> Result<Self, MachineError> {
        if ram_bytes == 0 || ram_bytes % 4 != 0 || ram_bytes > MAX_RAM_BYTES {
            return Err(MachineError::InvalidRamSize);
        }
        let mut bus = MemoryBus::default();
        bus.map_ram(0, ram_bytes).map_err(MachineError::Memory)?;
        Ok(Self {
            cpu: Cpu::new(entry),
            bus,
        })
    }

    pub fn load_image(&mut self, address: u32, bytes: &[u8]) -> Result<(), BusError> {
        self.bus.load_image(u64::from(address), bytes)
    }

    pub fn step(&mut self) -> Result<Step, CpuFault> {
        self.cpu.step(&mut self.bus)
    }
}

/// Hand-encoded examples/sum.s. No guest assembler or external ROM is required.
pub fn demo_image() -> Vec<u8> {
    [
        0x8210_200au32,
        0x8410_2000,
        0x8400_8001,
        0x82a0_6001,
        0x12bf_fffe,
        0x0100_0000,
        0xc420_2100,
        0x3080_0000,
    ]
    .into_iter()
    .flat_map(u32::to_be_bytes)
    .collect()
}

/// Hand-encoded examples/trap.s. The guest installs TBA=0x1000, enables traps,
/// enters user mode, invokes TA 7, and returns through JMPL/RETT.
pub fn trap_demo_image() -> Vec<u8> {
    let main = [
        0x0300_0004u32, // sethi %hi(0x1000), %g1
        0x8198_6000,    // wr %g1, 0, %tbr
        0x0100_0000,
        0x0100_0000,
        0x0100_0000,
        0x8188_2020, // wr %g0, 0x20, %psr (ET=1, S=0)
        0x0100_0000,
        0x0100_0000,
        0x0100_0000,
        0x91d0_2007, // ta 7
        0x8600_e001, // add %g3, 1, %g3
        0x3080_0000, // ba,a .
    ];
    let handler = [0x8400_a001u32, 0x81c4_a000, 0x81cc_a004];
    let mut image = vec![0; 0x187c];
    for (address, words) in [(0, main.as_slice()), (0x1870, handler.as_slice())] {
        for (slot, word) in image[address..].chunks_exact_mut(4).zip(words) {
            slot.copy_from_slice(&word.to_be_bytes());
        }
    }
    image
}
