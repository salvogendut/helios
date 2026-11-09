//! Bare-metal and sun4m harnesses with a polled console and NVRAM/RTC.
//! Timers and interrupt delivery share a deterministic functional timebase.

use crate::bus::{BusError, MapError};
use crate::cpu::{Cpu, CpuFault, FaultKind, Step};
use crate::devices::{nvram::Nvram, sx::Sx};
use crate::dram::Ss5Dram;
use crate::mmu::MmuConfig;
use crate::system_bus::{Sun4mLayout, SystemBus};
use std::fmt;

pub const TARGET: &str = "SPARCstation 5 (sun4m), SPARC V8 / microSPARC-II";
pub const MAX_RAM_BYTES: usize = 256 * 1024 * 1024;
pub const DEFAULT_RAM_BYTES: usize = 128 * 1024 * 1024;
/// Nominal SS5/85 hardware clock, independent of the interpreter's time quantum.
pub const CPU_CLOCK_HZ: u64 = 85_000_000;
pub const MAX_PROM_BYTES: usize = 1024 * 1024;
/// Base functional quantum; physical PROM reads add documented EBus wait time.
/// This is not a cycle-accurate microSPARC pipeline model.
pub const STEP_NS: u64 = 100;

/// Initial full-system profiles. All three run the same device models; the
/// SS-20/Axil profiles use the high sun4m physical map, a 36-bit reference MMU
/// and their own IDPROM identity. The CPU model remains the tested
/// microSPARC-II interpreter; single-processor SS-20 only for now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MachineProfile {
    #[default]
    Ss5,
    Ss20,
    Axil320,
}

impl MachineProfile {
    pub const fn target(self) -> &'static str {
        match self {
            Self::Ss5 => TARGET,
            Self::Ss20 => "SPARCstation 20 (sun4m, single processor), SPARC V8 profile",
            Self::Axil320 => "Axil 320 (SPARCstation 20 clone), SPARC V8 profile",
        }
    }

    pub const fn layout(self) -> Sun4mLayout {
        match self {
            Self::Ss5 => Sun4mLayout::ss5(),
            Self::Ss20 => Sun4mLayout::ss20(),
            Self::Axil320 => Sun4mLayout::axil320(),
        }
    }

    pub const fn mmu(self) -> MmuConfig {
        match self {
            Self::Ss5 => MmuConfig::ss5(),
            Self::Ss20 | Self::Axil320 => MmuConfig::ss20(),
        }
    }

    pub const fn nvram_machine_type(self) -> u8 {
        match self {
            Self::Ss5 => 0x80,
            Self::Ss20 | Self::Axil320 => 0x72,
        }
    }

    pub const fn default_mac(self) -> [u8; 6] {
        match self {
            Self::Ss5 | Self::Ss20 => [0x08, 0x00, 0x20, 0xc0, 0xff, 0xee],
            Self::Axil320 => [0x00, 0x00, 0x3c, 0x13, 0xde, 0xad],
        }
    }

    pub const fn default_serial(self) -> [u8; 3] {
        [0xc0, 0xff, 0xee]
    }

    pub fn dram(self, ram_bytes: usize) -> Result<Ss5Dram, MapError> {
        match self {
            Self::Ss5 => Ss5Dram::from_total(ram_bytes),
            Self::Ss20 | Self::Axil320 => Ss5Dram::from_ss20_total(ram_bytes),
        }
    }

    pub fn default_nvram(self) -> Nvram {
        Nvram::with_machine_type(
            self.nvram_machine_type(),
            self.default_mac(),
            self.default_serial(),
        )
    }
}

#[derive(Debug)]
pub enum MachineError {
    InvalidRamSize,
    InvalidDramSize,
    InvalidPromSize,
    Memory(MapError),
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRamSize => {
                f.write_str("RAM must be a nonzero multiple of 4 bytes, at most 256 MiB")
            }
            Self::InvalidDramSize => {
                f.write_str("RAM must fit in eight 8 MiB or 32 MiB DSIMMs (8–256 MiB)")
            }
            Self::Memory(error) => error.fmt(f),
            Self::InvalidPromSize => {
                f.write_str("PROM must be a nonempty raw image of at most 1 MiB")
            }
        }
    }
}

impl std::error::Error for MachineError {}

#[derive(Debug)]
pub struct Machine {
    pub cpu: Cpu,
    pub bus: SystemBus,
}

impl Machine {
    /// Hardware SS-5 memory layout; low slots receive 32 MiB then 8 MiB DSIMMs.
    pub fn ss5(ram_bytes: usize, prom: &[u8]) -> Result<Self, MachineError> {
        Self::ss5_with_nvram(ram_bytes, prom, MachineProfile::Ss5.default_nvram())
    }

    pub fn ss5_with_nvram(
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
    ) -> Result<Self, MachineError> {
        Self::with_profile(MachineProfile::Ss5, ram_bytes, prom, nvram)
    }

    pub fn ss20(ram_bytes: usize, prom: &[u8]) -> Result<Self, MachineError> {
        Self::ss20_with_nvram(ram_bytes, prom, MachineProfile::Ss20.default_nvram())
    }

    pub fn ss20_with_nvram(
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
    ) -> Result<Self, MachineError> {
        Self::with_profile(MachineProfile::Ss20, ram_bytes, prom, nvram)
    }

    pub fn axil320(ram_bytes: usize, prom: &[u8]) -> Result<Self, MachineError> {
        Self::axil320_with_nvram(ram_bytes, prom, MachineProfile::Axil320.default_nvram())
    }

    pub fn axil320_with_nvram(
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
    ) -> Result<Self, MachineError> {
        Self::with_profile(MachineProfile::Axil320, ram_bytes, prom, nvram)
    }

    pub fn with_profile(
        profile: MachineProfile,
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
    ) -> Result<Self, MachineError> {
        let dram = profile.dram(ram_bytes).map_err(|error| match error {
            MapError::InvalidRange => MachineError::InvalidDramSize,
            error => MachineError::Memory(error),
        })?;
        let mut bus = SystemBus::with_layout(profile.layout());
        bus.install_ss5_dram(dram).map_err(MachineError::Memory)?;
        let mut machine = Self {
            cpu: Cpu::new(0),
            bus,
        };
        machine.cpu.use_mmu_config(profile.mmu());
        machine.install_prom(prom, nvram)?;
        if profile != MachineProfile::Ss5 {
            machine
                .bus
                .install_sx(Sx::new())
                .map_err(MachineError::Memory)?;
            machine.reset();
        }
        Ok(machine)
    }

    /// Load a raw PROM into the profile's read-only alias (padded with 0xff).
    /// Install the polling serial console and a fresh deterministic NVRAM/RTC.
    pub fn sun4m(ram_bytes: usize, prom: &[u8]) -> Result<Self, MachineError> {
        Self::sun4m_with_nvram(ram_bytes, prom, Nvram::default())
    }

    pub fn sun4m_with_nvram(
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
    ) -> Result<Self, MachineError> {
        let mut machine = Self::bare_metal(ram_bytes, 0)?;
        machine.install_prom(prom, nvram)?;
        Ok(machine)
    }

    fn install_prom(&mut self, prom: &[u8], nvram: Nvram) -> Result<(), MachineError> {
        if prom.is_empty() || prom.len() > MAX_PROM_BYTES {
            return Err(MachineError::InvalidPromSize);
        }
        let mut image = vec![0xff; self.bus.prom_aperture() as usize];
        image[..prom.len()].copy_from_slice(prom);
        let prom_base = self.bus.prom_base();
        self.bus
            .map_rom(prom_base, &image)
            .map_err(MachineError::Memory)?;
        self.bus
            .install_sun4m(nvram)
            .map_err(MachineError::Memory)?;
        self.reset();
        Ok(())
    }

    /// Apply the power-on reset baseline, clearing reset-cause status.
    /// Reset CPU/MMU, timers, interrupt controller and serial. Retain RAM, ROM, NVRAM/RTC and bytes
    /// already transmitted to the host endpoint.
    pub fn reset(&mut self) {
        self.cpu.reset_sun4m();
        self.bus.reset_devices();
    }

    pub fn bare_metal(ram_bytes: usize, entry: u32) -> Result<Self, MachineError> {
        if ram_bytes == 0 || ram_bytes % 4 != 0 || ram_bytes > MAX_RAM_BYTES {
            return Err(MachineError::InvalidRamSize);
        }
        let mut bus = SystemBus::default();
        bus.map_ram(0, ram_bytes).map_err(MachineError::Memory)?;
        Ok(Self {
            cpu: Cpu::new(entry),
            bus,
        })
    }

    /// Advance all device clocks between steps without executing the CPU.
    pub fn advance_ns(&mut self, ns: u64) {
        self.bus.advance_ns(ns);
    }

    /// Whole-second advancement avoids overflow even for u64::MAX seconds.
    pub fn advance_seconds(&mut self, seconds: u64) {
        self.bus.advance_seconds(seconds);
    }

    pub fn framebuffer_pixels(&self, output: &mut [u32]) -> bool {
        if let Some(card) = self.bus.cgsix() {
            card.pixels(output);
            true
        } else if let Some(card) = self.bus.sx() {
            card.pixels(output);
            true
        } else {
            false
        }
    }

    pub fn mouse_motion(&mut self, dx: i32, dy: i32) {
        if let Some(devices) = self.bus.devices_mut() {
            devices.mouse.motion(dx, dy);
        }
    }

    pub fn mouse_button(&mut self, button: u8, down: bool) {
        if let Some(devices) = self.bus.devices_mut() {
            devices.mouse.button(button, down);
        }
    }

    pub fn load_image(&mut self, address: u32, bytes: &[u8]) -> Result<(), BusError> {
        self.bus.load_image(u64::from(address), bytes)
    }

    pub fn step(&mut self) -> Result<Step, CpuFault> {
        if self.software_reset_requested() {
            return Ok(self.software_reset(self.cpu.pc(), self.cpu.npc(), None));
        }
        self.service_ethernet().map_err(|error| CpuFault {
            pc: self.cpu.pc(),
            npc: self.cpu.npc(),
            instruction: None,
            kind: FaultKind::DeviceDma(error),
        })?;
        self.service_scsi().map_err(|error| CpuFault {
            pc: self.cpu.pc(),
            npc: self.cpu.npc(),
            instruction: None,
            kind: FaultKind::DeviceDma(error),
        })?;
        let pending = self.bus.interrupt_levels();
        let was_error = self.cpu.error_state().is_some();
        self.bus.begin_cpu_step();
        let result = self.cpu.step_with_interrupts(&mut self.bus, pending);
        let wait_ns = self.bus.finish_cpu_step(result.is_ok());
        if !was_error && self.cpu.error_state().is_some() {
            if let Some(devices) = self.bus.devices_mut() {
                devices.reset.latch_watchdog();
            }
        }
        let step = result?;
        self.bus.advance_ns(STEP_NS + wait_ns);
        if self.software_reset_requested() {
            if let Step::Executed {
                pc,
                npc,
                instruction,
            } = step
            {
                return Ok(self.software_reset(pc, npc, Some(instruction)));
            }
        }
        Ok(step)
    }

    fn software_reset_requested(&self) -> bool {
        self.bus
            .devices()
            .is_some_and(|devices| devices.reset.requested())
    }

    fn software_reset(&mut self, pc: u32, npc: u32, instruction: Option<u32>) -> Step {
        self.cpu.software_reset_sun4m();
        self.bus.software_reset_devices();
        Step::SoftwareReset {
            pc,
            npc,
            instruction,
        }
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

/// Synthetic reset PROM from examples/serial.s. Checks IDPROM, writes HELIOS,
/// echoes one input byte and a newline, then sets g7=1 and loops.
pub fn serial_demo_image() -> Vec<u8> {
    let words = [
        0x031c4400u32, // sethi %hi(0x71100000), %g1
        0x82106004,    // or %g1, 4, %g1
        0x84102009,    // or %g0, 0x9, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x841020c0,    // or %g0, 0xc0, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x84102004,    // or %g0, 0x4, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x84102044,    // or %g0, 0x44, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x8410200e,    // or %g0, 0xe, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x84102003,    // or %g0, 0x3, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x84102003,    // or %g0, 0x3, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x841020c1,    // or %g0, 0xc1, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x84102005,    // or %g0, 0x5, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x841020ea,    // or %g0, 0xea, %g2
        0xc4286000,    // stb %g2, [%g1]
        0x091c4807,    // sethi %hi(0x71201fd8), %g4
        0x881123d8,    // or %g4, 0x3d8, %g4
        0xc6092001,    // ldub [%g4+1], %g3
        0x8a102000,    // or %g0, 0x0, %g5
        0x8c102010,    // or %g0, 0x10, %g6
        0xc4092000,    // ldub [%g4], %g2
        0x8a194002,    // xor %g5, %g2, %g5
        0x88012001,    // add %g4, 1, %g4
        0x8ca1a001,    // subcc %g6, 1, %g6
        0x12bffffc,    // bne checksum
        0x01000000,    // nop
        0x88102200,    // or %g0, 0x200, %g4
        0x8c102006,    // or %g0, 0x6, %g6
        0xc4086000,    // ldub [%g1], %g2
        0x8088a004,    // andcc %g2, 4, %g0
        0x02bffffe,    // be transmit
        0x01000000,    // nop
        0xc4890120,    // lduba [%g4] 0x09, %g2
        0xc4286002,    // stb %g2, [%g1+2]
        0x88012001,    // add %g4, 1, %g4
        0x8ca1a001,    // subcc %g6, 1, %g6
        0x12bffff8,    // bne transmit
        0x01000000,    // nop
        0xc4086000,    // ldub [%g1], %g2
        0x8088a001,    // andcc %g2, 1, %g0
        0x02bffffe,    // be receive
        0x01000000,    // nop
        0xc4086002,    // ldub [%g1+2], %g2
        0xc4286002,    // stb %g2, [%g1+2]
        0x8410200a,    // or %g0, 0xa, %g2
        0xc4286002,    // stb %g2, [%g1+2]
        0x8e102001,    // or %g0, 0x1, %g7
        0x30800000,    // ba,a done
    ];
    let mut image = vec![0; 0x206];
    for (slot, word) in image.chunks_exact_mut(4).zip(words) {
        slot.copy_from_slice(&word.to_be_bytes());
    }
    image[0x200..].copy_from_slice(b"HELIOS");
    image
}

/// Synthetic reset PROM from examples/timer.s. Acknowledges level 10 and 14
/// timer interrupts and returns with JMPL/RETT to the interrupted work loop.
pub fn timer_demo_image() -> Vec<u8> {
    let mut image = vec![0; 0x11e8];
    for (address, words) in [
        (
            0x0,
            &[
                0x0500_0004u32, // sethi %hi(0x1000), %g2
                0x8198_a000,    // wr %g2, 0, %tbr
                0x8190_2000,    // wr %g0, 0, %wim
                0x0100_0000,    // nop
                0x0100_0000,    // nop
                0x0100_0000,    // nop
                0x031c_7440,    // sethi %hi(0x71d10000), %g1
                0x8410_2c00,    // or %g0, 0xc00, %g2
                0xc420_6000,    // st %g2, [%g1]
                0x091c_7400,    // sethi %hi(0x71d00000), %g4
                0x0500_0006,    // sethi %hi(0x1800), %g2
                0xc421_2000,    // st %g2, [%g4]
                0x0d1c_7840,    // sethi %hi(0x71e10000), %g6
                0x0520_0200,    // sethi %hi(0x80080000), %g2
                0xc421_a008,    // st %g2, [%g6+8]
                0x8188_20a0,    // wr %g0, 0xa0, %psr
                0x0100_0000,    // nop
                0x0100_0000,    // nop
                0x0100_0000,    // nop
                0x8600_e001,    // add %g3, 1, %g3
                0x10bf_ffff,    // ba work
                0x0100_0000,    // nop
            ][..],
        ),
        (
            0x200,
            &[
                0xe000_6000u32, // ld [%g1], %l0
                0xc020_6000,    // st %g0, [%g1]
                0x8a01_6001,    // add %g5, 1, %g5
                0x81c4_6000,    // jmpl %l1, %g0
                0x81cc_a000,    // rett %l2
            ][..],
        ),
        (
            0x240,
            &[
                0xe001_2000u32, // ld [%g4], %l0
                0xc021_2000,    // st %g0, [%g4]
                0x8e01_e001,    // add %g7, 1, %g7
                0x81c4_6000,    // jmpl %l1, %g0
                0x81cc_a000,    // rett %l2
            ][..],
        ),
        (
            0x11a0,
            &[
                0x10bf_fc18u32, // ba system_handler
                0x0100_0000,    // nop
            ][..],
        ),
        (
            0x11e0,
            &[
                0x10bf_fc18u32, // ba processor_handler
                0x0100_0000,    // nop
            ][..],
        ),
    ] {
        for (slot, word) in image[address..].chunks_exact_mut(4).zip(words) {
            slot.copy_from_slice(&word.to_be_bytes());
        }
    }
    image
}
