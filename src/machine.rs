//! Bare-metal and sun4m harnesses with a polled console and NVRAM/RTC.
//! Timers and interrupt delivery share a deterministic functional timebase.

#[cfg(feature = "jit")]
use crate::bus::AccessContext;
#[cfg(feature = "jit-native")]
use crate::bus::Bus;
use crate::bus::{BusError, Initiator, MapError};
use crate::cpu::{Cpu, CpuConfig, CpuFault, FaultKind, Step};
use crate::devices::{cgsix::Cgsix, mxcc::MxccState, nvram::Nvram, sx::Sx};
use crate::dma::DmaError;
use crate::dram::{DramLayout, Ss5Dram};
use crate::iommu::StandaloneIommu;
use crate::mmu::MmuConfig;
#[cfg(feature = "jit")]
use crate::mmu::Width;
use crate::system_bus::{Sun4mLayout, SystemBus, SystemBusState};
use crate::topology::{
    ProcessorArbitration, ProcessorModule, ProcessorRunState, ProcessorTopology, PROCESSOR_MODULES,
};
#[cfg(any(feature = "parking-profile", feature = "jit"))]
use std::time::Instant;
use std::{
    cell::RefCell,
    fmt,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

pub const TARGET: &str = "SPARCstation 5 (sun4m), SPARC V8 / microSPARC-II";
/// Upper bound for the diagnostic bare-metal harness. Named profiles use their
/// validated DSIMM topology instead.
pub const MAX_RAM_BYTES: usize = 256 * 1024 * 1024;
pub const DEFAULT_RAM_BYTES: usize = 128 * 1024 * 1024;
/// Observed Solaris SS-20 serial-idle loop boundary from the issue #5 trace.
/// These addresses are image-specific; token issuance validates them exactly.
pub const SOLARIS_IDLE_PC: u32 = 0xf006_fa24;
pub const SOLARIS_IDLE_NPC: u32 = 0xf006_fa28;
pub const SOLARIS_IDLE_PSR: u32 = 0x4040_00e0;
pub const SOLARIS_IDLE_CWP_MASK: u32 = 0x1f;

/// The mapped SS-20 Solaris idle boundary accepts only implemented register
/// windows 0-7; every non-CWP PSR bit must match the captured value exactly.
pub const fn solaris_idle_psr_matches(psr: u32) -> bool {
    psr & !SOLARIS_IDLE_CWP_MASK == SOLARIS_IDLE_PSR & !SOLARIS_IDLE_CWP_MASK
        && psr & SOLARIS_IDLE_CWP_MASK < 8
}

pub const fn solaris_idle_boundary(pc: u32, npc: u32, psr: u32) -> bool {
    pc == SOLARIS_IDLE_PC && npc == SOLARIS_IDLE_NPC && solaris_idle_psr_matches(psr)
}
static NEXT_PARKING_OWNER: AtomicU64 = AtomicU64::new(1);
/// SS5/85 processor clock.
pub const CPU_CLOCK_HZ: u64 = 85_000_000;
pub const MAX_PROM_BYTES: usize = 1024 * 1024;
/// Default fast-mode quantum. Hardware mode selects the profile oscillator
/// with fractional nanoseconds retained across outcomes.
pub const STEP_NS: u64 = 100;

/// Fast mode retains the historical 100 ns quantum so guest timer waits need
/// fewer interpreted instructions. Hardware mode uses the profile oscillator;
/// neither mode models pipeline/cache latency or host wall-clock throttling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpuClockMode {
    #[default]
    Fast,
    Hardware,
}

/// One functional CPU cycle per completed outcome. Division happens only at
/// construction; the step path carries the fractional part using addition.
/// This does not model pipeline stalls, superscalar issue or ordinary caches.
#[derive(Debug)]
struct CycleClock {
    hardware_hz: u64,
    mode: CpuClockMode,
    hz: u64,
    whole_ns: u64,
    remainder: u64,
    phase: u64,
}

impl CycleClock {
    const fn new(hardware_hz: u64) -> Self {
        Self::with_mode(hardware_hz, CpuClockMode::Fast)
    }

    const fn with_mode(hardware_hz: u64, mode: CpuClockMode) -> Self {
        let hz = match mode {
            CpuClockMode::Fast => 1_000_000_000 / STEP_NS,
            CpuClockMode::Hardware => hardware_hz,
        };
        Self {
            hardware_hz,
            mode,
            hz,
            whole_ns: 1_000_000_000 / hz,
            remainder: 1_000_000_000 % hz,
            phase: 0,
        }
    }

    fn next_ns(&mut self) -> u64 {
        self.phase += self.remainder;
        if self.phase >= self.hz {
            self.phase -= self.hz;
            self.whole_ns + 1
        } else {
            self.whole_ns
        }
    }

    fn diagnostic_state(&self) -> ClockSnapshot {
        ClockSnapshot {
            hardware_hz: self.hardware_hz,
            mode: self.mode,
            hz: self.hz,
            whole_ns: self.whole_ns,
            remainder: self.remainder,
            phase: self.phase,
        }
    }

    fn reset(&mut self) {
        self.phase = 0;
    }

    fn select(&mut self, mode: CpuClockMode) {
        *self = Self::with_mode(self.hardware_hz, mode);
    }
}

/// Initial full-system profiles. All three run the same device models; the
/// SS-20/Axil profiles use the high sun4m physical map, a 36-bit reference MMU
/// and their own IDPROM identity. They select SuperSPARC-II identity in the
/// shared functional SPARC V8 interpreter; single-processor SS-20 only for now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MachineProfile {
    #[default]
    Ss5,
    Ss20,
    Axil320,
}

/// Video device selected for a profile construction.
///
/// The SS-20 native SX is installed in the first VSIMM (`bank 7`) slot. The
/// Axil 320 also has two VSIMM-capable RAM slots, but VSIMM construction is not
/// part of the current configurable-RAM slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstalledVideo {
    #[default]
    None,
    Sx,
}

impl InstalledVideo {
    pub const fn has_vsimm(self) -> bool {
        matches!(self, Self::Sx)
    }
}

impl MachineProfile {
    /// Selected processor oscillator: SS5/85 or a 75 MHz SuperSPARC-II
    /// module on either MBus board. Independent of host execution speed.
    pub const fn cpu_clock_hz(self) -> u64 {
        match self {
            Self::Ss5 => CPU_CLOCK_HZ,
            Self::Ss20 | Self::Axil320 => 75_000_000,
        }
    }

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

    pub const fn cpu_config(self) -> CpuConfig {
        match self {
            Self::Ss5 => CpuConfig::microsparc2(),
            Self::Ss20 | Self::Axil320 => CpuConfig::supersparc2(),
        }
    }

    pub const fn builtin_sx(self) -> bool {
        matches!(self, Self::Ss20)
    }

    pub const fn default_video(self) -> InstalledVideo {
        match self {
            Self::Ss5 | Self::Axil320 => InstalledVideo::None,
            Self::Ss20 => InstalledVideo::Sx,
        }
    }

    /// Validate and build a profile/video RAM layout before allocating guest
    /// memory. SS-20 SX reserves bank 7; an SS-5 or Axil 320 has no VSIMM in
    /// this implementation.
    pub fn ram_layout(
        self,
        ram_bytes: usize,
        video: InstalledVideo,
    ) -> Result<DramLayout, MachineError> {
        match (self, video) {
            (Self::Ss5, InstalledVideo::Sx) | (Self::Axil320, InstalledVideo::Sx) => {
                Err(MachineError::InvalidVideo {
                    profile: self,
                    video,
                })
            }
            (Self::Ss5, InstalledVideo::None) => {
                DramLayout::ss5_total(ram_bytes).map_err(|_| MachineError::InvalidRamCapacity {
                    profile: self,
                    video,
                    requested: ram_bytes,
                })
            }
            (Self::Ss20, InstalledVideo::Sx) => {
                DramLayout::ss20_total(ram_bytes, 7).map_err(|_| MachineError::InvalidRamCapacity {
                    profile: self,
                    video,
                    requested: ram_bytes,
                })
            }
            (Self::Ss20, InstalledVideo::None) => {
                DramLayout::ss20_total(ram_bytes, 8).map_err(|_| MachineError::InvalidRamCapacity {
                    profile: self,
                    video,
                    requested: ram_bytes,
                })
            }
            (Self::Axil320, InstalledVideo::None) => {
                DramLayout::axil_total(ram_bytes).map_err(|_| MachineError::InvalidRamCapacity {
                    profile: self,
                    video,
                    requested: ram_bytes,
                })
            }
        }
    }

    /// Exact RAM totals representable by this profile and video configuration.
    pub fn valid_ram_totals(self, video: InstalledVideo) -> Result<Vec<usize>, MachineError> {
        match (self, video) {
            (Self::Ss5, InstalledVideo::Sx) | (Self::Axil320, InstalledVideo::Sx) => {
                Err(MachineError::InvalidVideo {
                    profile: self,
                    video,
                })
            }
            (Self::Ss5, InstalledVideo::None) => Ok(DramLayout::valid_ss5_totals()),
            (Self::Ss20, InstalledVideo::Sx) => Ok(DramLayout::valid_ss20_totals(7)),
            (Self::Ss20, InstalledVideo::None) => Ok(DramLayout::valid_ss20_totals(8)),
            (Self::Axil320, InstalledVideo::None) => Ok(DramLayout::valid_axil_totals()),
        }
    }

    /// Validate and select an explicit MBus processor topology. The default
    /// constructors remain single-CPU; two-module construction is the
    /// synthetic-test path and does not imply firmware startup support.
    pub fn processor_topology(self, processors: usize) -> Option<ProcessorTopology> {
        match (self, processors) {
            (Self::Ss5, 1) => Some(ProcessorTopology::empty()),
            (Self::Ss20 | Self::Axil320, 1) => Some(ProcessorTopology::single()),
            (Self::Ss20 | Self::Axil320, 2) => Some(ProcessorTopology::dual()),
            _ => None,
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

    /// Allocate DRAM using the profile's default video choice. Use
    /// [`Self::ram_layout`] to validate or select another video configuration
    /// without allocating guest memory.
    pub fn dram(self, ram_bytes: usize) -> Result<Ss5Dram, MapError> {
        let layout = self
            .ram_layout(ram_bytes, self.default_video())
            .map_err(|_| MapError::InvalidRange)?;
        Ss5Dram::from_layout(layout)
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
    InvalidRamCapacity {
        profile: MachineProfile,
        video: InstalledVideo,
        requested: usize,
    },
    InvalidVideo {
        profile: MachineProfile,
        video: InstalledVideo,
    },
    InvalidPromSize,
    InvalidProcessorCount,
    Memory(MapError),
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRamSize => {
                f.write_str("RAM must be a nonzero multiple of 4 bytes, at most 256 MiB")
            }
            Self::InvalidRamCapacity {
                profile,
                video,
                requested,
            } => {
                let sockets = if matches!(profile, MachineProfile::Ss5) {
                    "eight 8/32 MiB DSIMMs, 8–256 MiB"
                } else if video.has_vsimm() {
                    "seven 16/32/64 MiB DSIMMs, 16–448 MiB (bank 7 is the VSIMM)"
                } else if matches!(profile, MachineProfile::Axil320) {
                    "eight 16/64 MiB DSIMMs, 16–512 MiB (Axil OBP 2.19 profile)"
                } else {
                    "eight 16/32/64 MiB DSIMMs, 16–512 MiB"
                };
                write!(
                    f,
                    "{} with {} video cannot use {requested} bytes; valid RAM is {sockets}",
                    profile.target(),
                    if video.has_vsimm() {
                        "SX VSIMM"
                    } else {
                        "no VSIMM"
                    }
                )
            }
            Self::InvalidVideo { profile, video } => write!(
                f,
                "{} construction does not implement {} video in this configuration",
                profile.target(),
                if video.has_vsimm() {
                    "SX VSIMM"
                } else {
                    "no VSIMM"
                }
            ),
            Self::InvalidProcessorCount => {
                f.write_str("processor count must be one for SS-5 or one/two for SS-20/Axil")
            }
            Self::Memory(error) => error.fmt(f),
            Self::InvalidPromSize => {
                f.write_str("PROM must be a nonempty raw image of at most 1 MiB")
            }
        }
    }
}

impl std::error::Error for MachineError {}

/// One independently constructed processor module. The CPU, MMU diagnostic
/// state and MXCC state are owned here, not cloned from the boot CPU.
#[derive(Debug)]
pub struct ProcessorModuleState {
    pub module: ProcessorModule,
    pub cpu: Cpu,
    pub mxcc: Rc<RefCell<MxccState>>,
    clock: CycleClock,
}

impl ProcessorModuleState {
    /// Construct a fresh module for a profile whose selected topology contains
    /// it. SS-5 and modules outside the selected profile topology return `None`.
    pub fn new(profile: MachineProfile, module: ProcessorModule) -> Option<Self> {
        let count = usize::from(module.storage_index) + 1;
        let topology = profile.processor_topology(count)?;
        if !topology.modules().any(|candidate| candidate == module) {
            return None;
        }
        let mut cpu = Cpu::new(0);
        cpu.use_cpu_config(profile.cpu_config());
        cpu.use_mmu_config(profile.mmu());
        cpu.mmu.set_initiator(Initiator::Cpu(module));
        let mxcc = Rc::new(RefCell::new(MxccState::for_module(module)));
        cpu.mmu.set_mxcc(mxcc.clone());
        Some(Self {
            module,
            cpu,
            mxcc,
            clock: CycleClock::new(profile.cpu_clock_hz()),
        })
    }

    pub fn reset(&mut self) {
        self.cpu.reset_sun4m();
        self.mxcc.borrow_mut().reset();
        self.clock.reset();
    }

    /// Install this module's interrupt and timer bank by its hardware
    /// processor number. This does not schedule the module.
    pub fn install_processor_banks(&self, bus: &mut SystemBus) -> bool {
        bus.install_processor_banks(self.module.processor)
    }

    /// Board arbitration eligibility for this module. Presence is configured
    /// separately with `SystemBus::configure_processor`.
    pub fn arbitration(&self, bus: &SystemBus) -> ProcessorArbitration {
        bus.processor_arbitration(self.module.processor)
    }

    /// Scheduler-facing state. Arbitration denial is not a retired instruction
    /// or a synthesized trap; a denied module simply must not execute. A
    /// module reset is an explicit `reset` call, not a persistent state.
    pub fn run_state(&self, bus: &SystemBus) -> ProcessorRunState {
        match self.arbitration(bus) {
            ProcessorArbitration::Absent => ProcessorRunState::Absent,
            ProcessorArbitration::Disabled => ProcessorRunState::ArbitrationDenied,
            ProcessorArbitration::Enabled if self.cpu.error_state().is_some() => {
                ProcessorRunState::CpuError
            }
            ProcessorArbitration::Enabled => ProcessorRunState::Ready,
        }
    }
}

/// Per-hardware-processor scheduler accounting. `processor` is the hardware
/// number (0 or 2), not a compact module-index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessorCounters {
    pub processor: u8,
    pub instructions: u64,
    pub traps: u64,
    pub interrupts: u64,
    pub errors: u64,
}

impl ProcessorCounters {
    const fn new(processor: u8) -> Self {
        Self {
            processor,
            instructions: 0,
            traps: 0,
            interrupts: 0,
            errors: 0,
        }
    }
}

/// Aggregate and per-processor scheduler counters plus shared virtual time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerStats {
    pub virtual_ns: u64,
    pub processor0: ProcessorCounters,
    pub processor2: ProcessorCounters,
}

impl Default for SchedulerStats {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerStats {
    fn new() -> Self {
        Self {
            virtual_ns: 0,
            processor0: ProcessorCounters::new(0),
            processor2: ProcessorCounters::new(2),
        }
    }

    pub fn for_processor(&self, processor: u8) -> Option<&ProcessorCounters> {
        match processor {
            0 => Some(&self.processor0),
            2 => Some(&self.processor2),
            _ => None,
        }
    }

    fn counters_mut(&mut self, processor: u8) -> Option<&mut ProcessorCounters> {
        match processor {
            0 => Some(&mut self.processor0),
            2 => Some(&mut self.processor2),
            _ => None,
        }
    }

    pub fn total_instructions(&self) -> u64 {
        self.processor0.instructions + self.processor2.instructions
    }

    pub fn total_traps(&self) -> u64 {
        self.processor0.traps + self.processor2.traps
    }

    pub fn total_interrupts(&self) -> u64 {
        self.processor0.interrupts + self.processor2.interrupts
    }

    pub fn total_errors(&self) -> u64 {
        self.processor0.errors + self.processor2.errors
    }
}

/// One processor's functional clock state, retained for differential checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockSnapshot {
    pub hardware_hz: u64,
    pub mode: CpuClockMode,
    pub hz: u64,
    pub whole_ns: u64,
    pub remainder: u64,
    pub phase: u64,
}

/// Shared scheduler state at a differential checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerSnapshot {
    pub virtual_ns: u64,
    pub next_ready0: u64,
    pub next_ready2: u64,
    pub eligible0: bool,
    pub eligible2: bool,
    pub last_service_ns: Option<u64>,
    pub stats: SchedulerStats,
}

/// Read-only absolute-time and deadline information. `now_ns` includes all
/// time submitted to the bus, while `deferred_ns` is the portion not yet
/// applied to device registers. Deadline queries never mutate device state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timebase {
    pub now_ns: u64,
    pub deferred_ns: u64,
    /// Earliest timed device event, or `None` if no device has a deadline.
    pub next_device_ns: Option<u64>,
    /// Earliest scheduler deadline. It is present only after the explicit
    /// `step_scheduled` execution path has claimed ownership; fresh/reset
    /// machines and the direct `step` path report `None`.
    pub next_cpu_ready_ns: Option<u64>,
    pub next_ns: Option<u64>,
}

/// An opaque, single-use permission to advance only the validated SS-20
/// Solaris serial-idle boundary. It contains no host/UI state.
#[derive(Debug)]
pub struct ParkingToken {
    owner: u64,
    reset_epoch: u64,
    virtual_ns: u64,
    deferred_ns: u64,
    mutation_epoch: u64,
    configuration_epoch: u64,
    device_digest: u64,
    arbitration: ProcessorArbitration,
    cpu: Cpu,
}

/// Errors returned by the core-only parking token API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkingError {
    /// The profile or execution shape is outside this prototype (only one CPU
    /// and the SS-20 profile are supported).
    Unsupported,
    /// The CPU is not exactly at the validated Solaris idle boundary.
    NotIdleBoundary,
    /// CPU, memory, mapping, device or reset state changed after issuance.
    Stale,
    /// A pending interrupt or pre-step service action requires the CPU first.
    ServiceRequired,
}

impl fmt::Display for ParkingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "parking token is unsupported for this machine",
            Self::NotIdleBoundary => "CPU is not at the validated idle boundary",
            Self::Stale => "parking token is stale",
            Self::ServiceRequired => "parking requires normal service first",
        })
    }
}

impl std::error::Error for ParkingError {}

/// Result of one explicit parked interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkedAdvance {
    /// Requested virtual interval.
    pub requested_ns: u64,
    /// Actual interval advanced, clipped at the earliest known event.
    pub advanced_ns: u64,
    /// True when the requested interval crossed an event boundary.
    pub clipped: bool,
    /// Earliest known event after the advancement.
    pub next_ns: Option<u64>,
    /// True when the caller must service a due device/CPU event before issuing
    /// another token.
    pub service_required: bool,
}

/// Full architectural/CPU state plus deterministic bus/device and timing state.
/// CPU snapshots are clones, so inactive register windows and the complete
/// MMU/FPU state are included. Bus/device state is represented by fingerprints.
#[derive(Debug, PartialEq, Eq)]
pub struct MachineSnapshot {
    pub cpu: Cpu,
    pub secondary_cpu: Option<Cpu>,
    pub scheduler: SchedulerSnapshot,
    pub clock: ClockSnapshot,
    pub secondary_clock: Option<ClockSnapshot>,
    pub standalone_iommu: Option<StandaloneIommu>,
    pub bus: SystemBusState,
}

impl Clone for MachineSnapshot {
    fn clone(&self) -> Self {
        Self {
            cpu: snapshot_cpu(&self.cpu),
            secondary_cpu: self.secondary_cpu.as_ref().map(snapshot_cpu),
            scheduler: self.scheduler.clone(),
            clock: self.clock.clone(),
            secondary_clock: self.secondary_clock.clone(),
            standalone_iommu: self.standalone_iommu.clone(),
            bus: self.bus.clone(),
        }
    }
}

impl MachineSnapshot {
    pub fn differences(&self, other: &Self) -> Vec<&'static str> {
        let mut differences = Vec::new();
        if self.cpu != other.cpu {
            differences.push("cpu0");
        }
        if self.secondary_cpu != other.secondary_cpu {
            differences.push("cpu2");
        }
        if self.scheduler != other.scheduler {
            differences.push("scheduler");
        }
        if self.clock != other.clock {
            differences.push("clock0");
        }
        if self.secondary_clock != other.secondary_clock {
            differences.push("clock2");
        }
        if self.standalone_iommu != other.standalone_iommu {
            differences.push("iommu");
        }
        if self.bus.layout != other.bus.layout {
            differences.push("bus.layout");
        }
        if self.bus.openbios_digest != other.bus.openbios_digest {
            differences.push("bus.openbios");
        }
        if self.bus.context != other.bus.context {
            differences.push("bus.context");
        }
        if self.bus.configured_processors != other.bus.configured_processors {
            differences.push("bus.configured_processors");
        }
        if self.bus.prom_cycles != other.bus.prom_cycles
            || self.bus.prom_phase != other.bus.prom_phase
        {
            differences.push("bus.prom");
        }
        if self.bus.logical_ns != other.bus.logical_ns {
            differences.push("bus.logical");
        }
        if self.bus.device_ns != other.bus.device_ns {
            differences.push("bus.device_ns");
        }
        if self.bus.memory_digest != other.bus.memory_digest {
            differences.push("bus.memory");
        }
        if self.bus.dram_digest != other.bus.dram_digest {
            differences.push("bus.dram");
        }
        if self.bus.devices_digest != other.bus.devices_digest {
            differences.push("bus.devices");
        }
        if self.bus.cgsix_digest != other.bus.cgsix_digest {
            differences.push("bus.cgsix");
        }
        if self.bus.sx_digest != other.bus.sx_digest {
            differences.push("bus.sx");
        }
        if self.bus.mxcc_digest != other.bus.mxcc_digest {
            differences.push("bus.mxcc");
        }
        differences
    }
}

/// One tagged scheduler outcome. `virtual_ns` is the shared device clock at
/// return, normally the earliest next eligible CPU deadline. It need not be
/// the selected instruction's completion if another CPU has an earlier
/// opportunity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledStep {
    pub processor: u8,
    pub step: Step,
    pub virtual_ns: u64,
}

/// Explicit scheduler failures. Device service failures are not CPU traps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerError {
    Cpu { processor: u8, fault: CpuFault },
    Device(DmaError),
    NoRunnable,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cpu { processor, fault } => {
                write!(f, "processor {processor} error: {fault}")
            }
            Self::Device(error) => write!(f, "device error: {error}"),
            Self::NoRunnable => f.write_str("no processor is runnable"),
        }
    }
}

impl std::error::Error for SchedulerError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SchedulerState {
    virtual_ns: u64,
    next_ready0: u64,
    next_ready2: u64,
    eligible0: bool,
    eligible2: bool,
    last_service_ns: Option<u64>,
    stats: SchedulerStats,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerState {
    fn new() -> Self {
        Self {
            virtual_ns: 0,
            next_ready0: 0,
            next_ready2: 0,
            eligible0: true,
            eligible2: false,
            last_service_ns: None,
            stats: SchedulerStats::new(),
        }
    }

    fn set_next_ready(&mut self, processor: u8, value: u64) {
        match processor {
            0 => self.next_ready0 = value,
            2 => self.next_ready2 = value,
            _ => {}
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reconcile eligibility without replaying old slots or cancelling a wait
    /// already charged to an instruction before arbitration was withdrawn.
    fn sync_eligibility(&mut self, eligible0: bool, eligible2: bool) {
        if eligible0 && !self.eligible0 {
            self.next_ready0 = self.next_ready0.max(self.virtual_ns);
        }
        if eligible2 && !self.eligible2 {
            self.next_ready2 = self.next_ready2.max(self.virtual_ns);
        }
        self.eligible0 = eligible0;
        self.eligible2 = eligible2;
    }

    fn advance_external(&mut self, ns: u64) {
        self.virtual_ns = self.virtual_ns.saturating_add(ns);
        self.next_ready0 = self.next_ready0.max(self.virtual_ns);
        self.next_ready2 = self.next_ready2.max(self.virtual_ns);
        self.last_service_ns = None;
    }

    fn earliest_ready(&self) -> Option<u64> {
        let mut earliest = None;
        if self.eligible0 {
            earliest = Some(self.next_ready0);
        }
        if self.eligible2 {
            earliest = Some(match earliest {
                Some(value) => value.min(self.next_ready2),
                None => self.next_ready2,
            });
        }
        earliest
    }

    fn record(&mut self, processor: u8, step: Step) {
        if let Some(counters) = self.stats.counters_mut(processor) {
            match step {
                Step::Executed { .. } => counters.instructions += 1,
                Step::TrapTaken { .. } => counters.traps += 1,
                Step::InterruptTaken { .. } => counters.interrupts += 1,
                Step::SoftwareReset { .. } => {}
            }
        }
    }

    fn record_error(&mut self, processor: u8) {
        if let Some(counters) = self.stats.counters_mut(processor) {
            counters.errors += 1;
        }
    }
}

#[derive(Debug)]
pub struct Machine {
    pub cpu: Cpu,
    pub bus: SystemBus,
    topology: ProcessorTopology,
    pub(crate) standalone_iommu: Option<StandaloneIommu>,
    secondary_cpu: Option<ProcessorModuleState>,
    scheduler: SchedulerState,
    clock: CycleClock,
    service_guards: bool,
    /// Whether the scheduler's deadline is the authority for `Timebase`.
    /// This is execution-path ownership, not guest architectural state.
    scheduled_clock_owned: bool,
    profile: MachineProfile,
    parking_owner: u64,
    reset_epoch: u64,
}

#[cfg(feature = "jit-native")]
fn pack_icc(icc: crate::cpu::Icc) -> u32 {
    u32::from(icc.n) | (u32::from(icc.z) << 1) | (u32::from(icc.v) << 2) | (u32::from(icc.c) << 3)
}

#[cfg(feature = "jit-native")]
fn unpack_icc(bits: u32) -> crate::cpu::Icc {
    crate::cpu::Icc {
        n: bits & 1 != 0,
        z: bits & 2 != 0,
        v: bits & 4 != 0,
        c: bits & 8 != 0,
    }
}

fn snapshot_cpu(cpu: &Cpu) -> Cpu {
    let mut snapshot = cpu.clone();
    let mxcc = snapshot.mmu.mxcc_handle();
    let state = mxcc.borrow().clone();
    snapshot.mmu.set_mxcc(Rc::new(RefCell::new(state)));
    snapshot
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

    pub fn ss20_without_sx(
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
    ) -> Result<Self, MachineError> {
        Self::with_profile_video(
            MachineProfile::Ss20,
            ram_bytes,
            prom,
            nvram,
            InstalledVideo::None,
        )
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
        Self::with_profile_video(profile, ram_bytes, prom, nvram, profile.default_video())
    }

    /// Construct a profile with an explicit video choice. This permits an
    /// SS-20 without a VSIMM to use all eight DRAM banks, including 512 MiB.
    pub fn with_profile_video(
        profile: MachineProfile,
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
        video: InstalledVideo,
    ) -> Result<Self, MachineError> {
        Self::with_profile_processors_video(profile, ram_bytes, prom, nvram, 1, video)
    }

    /// Explicit construction path for one or two independently owned processor
    /// modules. SS-5 rejects two; SS-20/Axil accept one or two. The second
    /// module starts arbitration-disabled.
    pub fn with_profile_processors(
        profile: MachineProfile,
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
        processors: usize,
    ) -> Result<Self, MachineError> {
        Self::with_profile_processors_video(
            profile,
            ram_bytes,
            prom,
            nvram,
            processors,
            profile.default_video(),
        )
    }

    /// Processor-topology plus video construction path. RAM capacity and the
    /// video/DSIMM combination are validated before any guest RAM allocation.
    pub fn with_profile_processors_video(
        profile: MachineProfile,
        ram_bytes: usize,
        prom: &[u8],
        nvram: Nvram,
        processors: usize,
        video: InstalledVideo,
    ) -> Result<Self, MachineError> {
        let topology = profile
            .processor_topology(processors)
            .ok_or(MachineError::InvalidProcessorCount)?;
        let layout = profile.ram_layout(ram_bytes, video)?;
        if prom.is_empty() || prom.len() > MAX_PROM_BYTES {
            return Err(MachineError::InvalidPromSize);
        }
        let dram = Ss5Dram::from_layout(layout).map_err(MachineError::Memory)?;
        let mut bus = SystemBus::with_layout(profile.layout());
        bus.install_ss5_dram(dram).map_err(MachineError::Memory)?;
        let mut machine = Self {
            cpu: Cpu::new(0),
            bus,
            topology,
            standalone_iommu: (!profile.layout().ncr_windows).then(StandaloneIommu::default),
            secondary_cpu: None,
            scheduler: SchedulerState::default(),
            clock: CycleClock::new(profile.cpu_clock_hz()),
            service_guards: true,
            scheduled_clock_owned: false,
            profile,
            parking_owner: NEXT_PARKING_OWNER.fetch_add(1, Ordering::Relaxed),
            reset_epoch: 0,
        };
        machine.cpu.use_cpu_config(profile.cpu_config());
        machine.cpu.use_mmu_config(profile.mmu());
        machine
            .cpu
            .mmu
            .set_initiator(Initiator::Cpu(PROCESSOR_MODULES[0]));
        if profile.mmu().reference_mmu {
            machine.bus.install_mxcc(machine.cpu.mmu.mxcc_handle());
        }
        let mut secondary = topology
            .module_for_processor(2)
            .and_then(|module| ProcessorModuleState::new(profile, module));
        if let Some(secondary) = &secondary {
            machine.bus.install_mxcc_module(secondary.mxcc.clone());
        }
        machine.install_prom(prom, nvram)?;
        if video.has_vsimm() {
            machine
                .bus
                .install_sx(Sx::new())
                .map_err(MachineError::Memory)?;
            machine.reset();
        }
        if let Some(secondary) = secondary.as_mut() {
            machine.configure_processor(2);
            machine.install_processor_banks(2);
            secondary.reset();
        }
        machine.secondary_cpu = secondary;
        Ok(machine)
    }

    pub fn ss20_with_processors(
        ram_bytes: usize,
        prom: &[u8],
        processors: usize,
    ) -> Result<Self, MachineError> {
        Self::with_profile_processors(
            MachineProfile::Ss20,
            ram_bytes,
            prom,
            MachineProfile::Ss20.default_nvram(),
            processors,
        )
    }

    pub fn axil320_with_processors(
        ram_bytes: usize,
        prom: &[u8],
        processors: usize,
    ) -> Result<Self, MachineError> {
        Self::with_profile_processors(
            MachineProfile::Axil320,
            ram_bytes,
            prom,
            MachineProfile::Axil320.default_nvram(),
            processors,
        )
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

    /// Explicit processor-module topology for this machine. Profile
    /// constructors report one populated module for SS-20/Axil, no MBus module
    /// for SS-5, and bare-metal machines report no module. Tests may build a
    /// second independent module without changing the production execution path.
    pub fn topology(&self) -> ProcessorTopology {
        self.topology
    }

    pub fn processor_modules(&self) -> impl Iterator<Item = ProcessorModule> + '_ {
        self.topology.modules()
    }

    /// The independently owned second module, if explicitly constructed.
    pub fn secondary_cpu(&self) -> Option<&ProcessorModuleState> {
        self.secondary_cpu.as_ref()
    }

    pub fn secondary_cpu_mut(&mut self) -> Option<&mut ProcessorModuleState> {
        self.secondary_cpu.as_mut()
    }

    /// Borrow the secondary module and the shared bus together for direct
    /// guest-memory/MMU diagnostics; the two owners remain disjoint.
    pub fn secondary_cpu_with_bus_mut(
        &mut self,
    ) -> Option<(&mut ProcessorModuleState, &mut SystemBus)> {
        self.secondary_cpu
            .as_mut()
            .map(|secondary| (secondary, &mut self.bus))
    }

    pub fn processor_cpu(&self, processor: u8) -> Option<&Cpu> {
        match processor {
            0 => Some(&self.cpu),
            2 => self.secondary_cpu.as_ref().map(|module| &module.cpu),
            _ => None,
        }
    }

    pub fn processor_cpu_mut(&mut self, processor: u8) -> Option<&mut Cpu> {
        match processor {
            0 => Some(&mut self.cpu),
            2 => self.secondary_cpu.as_mut().map(|module| &mut module.cpu),
            _ => None,
        }
    }

    /// Synthetic-test setup: clear boot mode and set one processor's PC/nPC.
    pub fn set_processor_entry(&mut self, processor: u8, pc: u32, npc: u32) -> bool {
        match processor {
            0 => {
                if self
                    .cpu
                    .mmu
                    .store(&mut self.bus, 0, 4, crate::mmu::Width::Word, 0)
                    .is_err()
                {
                    return false;
                }
                self.cpu.set_pc_npc(pc, npc);
                true
            }
            2 => {
                let Some(secondary) = self.secondary_cpu.as_mut() else {
                    return false;
                };
                if secondary
                    .cpu
                    .mmu
                    .store(&mut self.bus, 0, 4, crate::mmu::Width::Word, 0)
                    .is_err()
                {
                    return false;
                }
                secondary.cpu.set_pc_npc(pc, npc);
                true
            }
            _ => false,
        }
    }

    /// Install the addressable interrupt and timer banks for one hardware
    /// processor number. This does not schedule a second CPU and does not make
    /// an absent module present.
    pub fn install_processor_banks(&mut self, processor: u8) -> bool {
        self.bus.install_processor_banks(processor)
    }

    /// Configure board-level presence for arbitration without creating CPU
    /// storage or a scheduler slot.
    pub fn configure_processor(&mut self, processor: u8) -> bool {
        self.bus.configure_processor(processor)
    }

    pub fn processor_arbitration(&self, processor: u8) -> ProcessorArbitration {
        self.bus.processor_arbitration(processor)
    }

    /// Host-side AER transition for an installed processor. See
    /// `SystemBus::set_processor_arbitration`.
    pub fn set_processor_arbitration(&mut self, processor: u8, enabled: bool) -> bool {
        let changed = self.bus.set_processor_arbitration(processor, enabled);
        if changed {
            let eligible0 = self.processor_arbitration(0).is_enabled();
            let eligible2 =
                self.secondary_cpu.is_some() && self.processor_arbitration(2).is_enabled();
            self.scheduler.sync_eligibility(eligible0, eligible2);
        }
        changed
    }

    /// The standalone MBus I/O translation owner, when this profile has one.
    pub fn standalone_iommu(&self) -> Option<&StandaloneIommu> {
        self.standalone_iommu.as_ref()
    }

    /// Apply the power-on reset baseline, clearing reset-cause status.
    /// Reset CPU/MMU, timers, interrupt controller and serial. Retain RAM, ROM, NVRAM/RTC and bytes
    /// already transmitted to the host endpoint.
    pub fn reset(&mut self) {
        self.cpu.reset_sun4m();
        self.clock.reset();
        if let Some(secondary) = &mut self.secondary_cpu {
            secondary.reset();
        }
        self.bus.reset_devices();
        if let Some(iommu) = &mut self.standalone_iommu {
            iommu.reset();
        }
        self.scheduler.reset();
        self.scheduled_clock_owned = false;
        self.reset_epoch = self.reset_epoch.wrapping_add(1);
    }

    /// Reset only the local CPU/MMU and MXCC of one processor module. Board
    /// devices, AER state, the standalone IOMMU, RAM and other modules are
    /// retained. The module's scheduler readiness is moved to the current
    /// shared virtual time; its counters and the other module's state remain.
    pub fn reset_processor_module(&mut self, processor: u8) -> bool {
        if !self.bus.processor_present(processor) {
            return false;
        }
        match processor {
            0 => {
                self.cpu.reset_sun4m();
                self.clock.reset();
                self.scheduler.set_next_ready(0, self.scheduler.virtual_ns);
                true
            }
            2 => {
                let Some(secondary) = &mut self.secondary_cpu else {
                    return false;
                };
                secondary.reset();
                self.scheduler.set_next_ready(2, self.scheduler.virtual_ns);
                true
            }
            _ => false,
        }
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
            topology: ProcessorTopology::empty(),
            standalone_iommu: None,
            secondary_cpu: None,
            scheduler: SchedulerState::default(),
            clock: CycleClock::new(1_000_000_000 / STEP_NS),
            service_guards: true,
            scheduled_clock_owned: false,
            profile: MachineProfile::Ss5,
            parking_owner: NEXT_PARKING_OWNER.fetch_add(1, Ordering::Relaxed),
            reset_epoch: 0,
        })
    }

    /// Total logical device time, including time deferred by the CPU fast path.
    pub fn virtual_ns(&self) -> u64 {
        self.bus.logical_ns()
    }

    /// Apply deferred device time without moving the logical clock. Deadline
    /// queries deliberately do not call this.
    pub fn flush_device_time(&mut self) {
        self.bus.flush_device_time();
    }

    /// Read-only timebase and deadline query. The scheduler deadline is exposed
    /// only after explicit `step_scheduled` ownership; clock equality is not
    /// treated as ownership.
    pub fn timebase(&self) -> Timebase {
        let now_ns = self.bus.logical_ns();
        let next_device_ns = self.bus.next_device_deadline_ns();
        let next_cpu_ready_ns = if self.scheduled_clock_owned {
            self.scheduler.earliest_ready()
        } else {
            None
        };
        let next_ns = match (next_device_ns, next_cpu_ready_ns) {
            (Some(device), Some(cpu)) => Some(device.min(cpu)),
            (Some(device), None) => Some(device),
            (None, Some(cpu)) => Some(cpu),
            (None, None) => None,
        };
        Timebase {
            now_ns,
            deferred_ns: self.bus.deferred_ns(),
            next_device_ns,
            next_cpu_ready_ns,
            next_ns,
        }
    }

    fn idle_boundary_proof(&self) -> bool {
        // The complete captured stable path. The annulled f006fa30 slot is
        // intentionally absent: it is not executed on this path.
        const CODE: [(u32, u32); 14] = [
            (0xf006_fa24, 0xd002_20ec), // ld [%o0+0xec], %o0
            (0xf006_fa28, 0x80a2_2001), // cmp %o0, 1
            (0xf006_fa2c, 0x3280_000f), // bne,a f006fa68 (not taken)
            (0xf006_fa34, 0xd006_2058), // ld [%i0+0x58], %o0
            (0xf006_fa38, 0x80a2_2000), // cmp %o0, 0
            (0xf006_fa3c, 0x0280_0006), // be f006fa54 (taken)
            (0xf006_fa40, 0x213c_097e), // sethi ..., %l0
            (0xf006_fa54, 0xe004_21e8), // ld [%l0+0x1e8], %l0
            (0xf006_fa58, 0x9fc4_0000), // call %l0
            (0xf006_fa5c, 0x0100_0000), // nop (delay)
            (0xf007_1708, 0x81c3_e008), // retl
            (0xf007_170c, 0x0100_0000), // nop
            (0xf006_fa60, 0x10bf_fff1), // ba f006fa24
            (0xf006_fa64, 0x113c_097e), // sethi ..., %o0 (delay)
        ];
        let read_code = |pc: u32, expected: u32| {
            let physical = self
                .cpu
                .mmu
                .parking_peek(pc, true, |address| self.bus.peek_ram_word(address))?;
            (self.bus.peek_ram_word(physical) == Some(expected)).then_some(())
        };
        for (pc, expected) in CODE {
            if read_code(pc, expected).is_none() {
                return false;
            }
        }
        let read_data = |address: u32| {
            let physical = self
                .cpu
                .mmu
                .parking_peek(address, false, |address| self.bus.peek_ram_word(address))?;
            self.bus.peek_ram_word(physical)
        };
        let o0 = self.cpu.read_reg(8).wrapping_add(0xec);
        let i0 = self.cpu.read_reg(24).wrapping_add(0x58);
        o0 == 0xf025_f8ec
            && i0 == 0xf026_dea0
            && self.cpu.read_reg(15) == 0xf006_fa58
            && self.cpu.read_reg(16) == 0xf007_1708
            && read_data(o0) == Some(1)
            && read_data(i0) == Some(0)
            && read_data(0xf025_f9e8) == Some(0xf007_1708)
    }

    /// Issue a single-use token for the observed SS-20 Solaris serial-idle
    /// boundary. This is a narrow core-only prototype, not a generic idle
    /// classifier. Querying it never mutates CPU, device, RAM or MMU state.
    pub fn parking_token(&self) -> Result<ParkingToken, ParkingError> {
        #[cfg(feature = "parking-profile")]
        let started = Instant::now();
        let result = (|| {
            if self.profile != MachineProfile::Ss20 || self.secondary_cpu.is_some() {
                return Err(ParkingError::Unsupported);
            }
            if self.cpu.error_state().is_some()
                || self.software_reset_requested()
                || !solaris_idle_boundary(self.cpu.pc(), self.cpu.npc(), self.cpu.psr())
            {
                return Err(ParkingError::NotIdleBoundary);
            }
            #[cfg(feature = "parking-profile")]
            let proof = crate::parking_profile::timed(
                || self.idle_boundary_proof(),
                |ns, ok| crate::parking_profile::proof(ns, *ok),
            );
            #[cfg(not(feature = "parking-profile"))]
            let proof = self.idle_boundary_proof();
            if !proof {
                return Err(ParkingError::NotIdleBoundary);
            }
            #[cfg(feature = "parking-profile")]
            let ready = crate::parking_profile::timed(
                || self.bus.parking_ready(),
                |ns, ok| crate::parking_profile::readiness(ns, *ok),
            );
            #[cfg(not(feature = "parking-profile"))]
            let ready = self.bus.parking_ready();
            if !ready {
                return Err(ParkingError::ServiceRequired);
            }
            #[cfg(feature = "parking-profile")]
            let device_digest = crate::parking_profile::timed(
                || self.bus.parking_device_digest(),
                |ns, _| crate::parking_profile::digest(ns),
            );
            #[cfg(not(feature = "parking-profile"))]
            let device_digest = self.bus.parking_device_digest();
            #[cfg(feature = "parking-profile")]
            let arbitration = crate::parking_profile::timed(
                || self.processor_arbitration(0),
                |ns, _| crate::parking_profile::arbitration(ns),
            );
            #[cfg(not(feature = "parking-profile"))]
            let arbitration = self.processor_arbitration(0);
            #[cfg(feature = "parking-profile")]
            let cpu = crate::parking_profile::timed(
                || snapshot_cpu(&self.cpu),
                |ns, _| crate::parking_profile::snapshot_cpu(ns),
            );
            #[cfg(not(feature = "parking-profile"))]
            let cpu = snapshot_cpu(&self.cpu);
            Ok(ParkingToken {
                owner: self.parking_owner,
                reset_epoch: self.reset_epoch,
                virtual_ns: self.bus.logical_ns(),
                deferred_ns: self.bus.deferred_ns(),
                mutation_epoch: self.bus.mutation_epoch(),
                configuration_epoch: self.bus.configuration_epoch(),
                device_digest,
                arbitration,
                cpu,
            })
        })();
        #[cfg(feature = "parking-profile")]
        crate::parking_profile::token(
            crate::parking_profile::nanos(started.elapsed()),
            result.is_ok(),
        );
        result
    }

    /// Validate a token without consuming it or mutating the machine. The
    /// checks cover reset ownership, the exact boundary, CPU/MMU/FPU state,
    /// bus-visible memory/mapping/device writes, deferred time and device state.
    pub fn validate_parking_token(&self, token: &ParkingToken) -> bool {
        #[cfg(feature = "parking-profile")]
        let started = Instant::now();
        let result = self.validate_parking_token_impl(token);
        #[cfg(feature = "parking-profile")]
        crate::parking_profile::validate(crate::parking_profile::nanos(started.elapsed()), result);
        result
    }

    fn validate_parking_token_impl(&self, token: &ParkingToken) -> bool {
        if !(token.owner == self.parking_owner
            && token.reset_epoch == self.reset_epoch
            && self.profile == MachineProfile::Ss20
            && self.secondary_cpu.is_none()
            && self.cpu.error_state().is_none()
            && !self.software_reset_requested()
            && solaris_idle_boundary(self.cpu.pc(), self.cpu.npc(), self.cpu.psr())
            && self.bus.logical_ns() == token.virtual_ns
            && self.bus.deferred_ns() == token.deferred_ns
            && self.bus.mutation_epoch() == token.mutation_epoch
            && self.bus.configuration_epoch() == token.configuration_epoch)
        {
            return false;
        }
        #[cfg(feature = "parking-profile")]
        let device_digest = crate::parking_profile::timed(
            || self.bus.parking_device_digest(),
            |ns, _| crate::parking_profile::digest(ns),
        );
        #[cfg(not(feature = "parking-profile"))]
        let device_digest = self.bus.parking_device_digest();
        if device_digest != token.device_digest {
            return false;
        }
        #[cfg(feature = "parking-profile")]
        let arbitration = crate::parking_profile::timed(
            || self.processor_arbitration(0),
            |ns, _| crate::parking_profile::arbitration(ns),
        );
        #[cfg(not(feature = "parking-profile"))]
        let arbitration = self.processor_arbitration(0);
        if arbitration != token.arbitration {
            return false;
        }
        #[cfg(feature = "parking-profile")]
        let cpu = crate::parking_profile::timed(
            || snapshot_cpu(&self.cpu),
            |ns, _| crate::parking_profile::snapshot_cpu(ns),
        );
        #[cfg(not(feature = "parking-profile"))]
        let cpu = snapshot_cpu(&self.cpu);
        #[cfg(feature = "parking-profile")]
        let equal = crate::parking_profile::timed(
            || cpu == token.cpu,
            |ns, equal| crate::parking_profile::snapshot_eq(ns, *equal),
        );
        #[cfg(not(feature = "parking-profile"))]
        let equal = cpu == token.cpu;
        equal
    }

    /// Consume a validated token and advance devices by an explicit virtual
    /// interval, never crossing the earliest known event. The CPU is not
    /// stepped and no DMA/IRQ/CPU service occurs here; the caller must service
    /// the returned boundary before issuing another token.
    pub fn advance_parked(
        &mut self,
        token: ParkingToken,
        max_ns: u64,
    ) -> Result<ParkedAdvance, ParkingError> {
        #[cfg(feature = "parking-profile")]
        let started = Instant::now();
        let result = self.advance_parked_impl(token, max_ns);
        #[cfg(feature = "parking-profile")]
        crate::parking_profile::advance(
            crate::parking_profile::nanos(started.elapsed()),
            result.is_ok(),
        );
        result
    }

    fn advance_parked_impl(
        &mut self,
        token: ParkingToken,
        max_ns: u64,
    ) -> Result<ParkedAdvance, ParkingError> {
        #[cfg(feature = "parking-profile")]
        let validation_started = Instant::now();
        let valid = self.validate_parking_token_impl(&token);
        #[cfg(feature = "parking-profile")]
        crate::parking_profile::validate(
            crate::parking_profile::nanos(validation_started.elapsed()),
            valid,
        );
        if !valid {
            return Err(ParkingError::Stale);
        }
        #[cfg(feature = "parking-profile")]
        let ready = crate::parking_profile::timed(
            || self.bus.parking_ready(),
            |ns, ok| crate::parking_profile::readiness(ns, *ok),
        );
        #[cfg(not(feature = "parking-profile"))]
        let ready = self.bus.parking_ready();
        if !ready {
            return Err(ParkingError::ServiceRequired);
        }
        let before = self.bus.logical_ns();
        #[cfg(feature = "parking-profile")]
        let next = crate::parking_profile::timed(
            || self.timebase().next_ns,
            |ns, _| crate::parking_profile::deadline(ns),
        );
        #[cfg(not(feature = "parking-profile"))]
        let next = self.timebase().next_ns;
        let actual = max_ns.min(next.map_or(max_ns, |event| event.saturating_sub(before)));
        let reached_event = next.is_some_and(|event| event <= before.saturating_add(actual));
        #[cfg(feature = "parking-profile")]
        crate::parking_profile::timed(
            || self.advance_ns(actual),
            |ns, ()| crate::parking_profile::advancement(ns),
        );
        #[cfg(not(feature = "parking-profile"))]
        self.advance_ns(actual);
        #[cfg(feature = "parking-profile")]
        let after = crate::parking_profile::timed(
            || self.timebase(),
            |ns, _| crate::parking_profile::deadline(ns),
        );
        #[cfg(not(feature = "parking-profile"))]
        let after = self.timebase();
        Ok(ParkedAdvance {
            requested_ns: max_ns,
            advanced_ns: actual,
            clipped: actual < max_ns,
            next_ns: after.next_ns,
            service_required: reached_event
                || after
                    .next_ns
                    .is_some_and(|event| event <= self.bus.logical_ns()),
        })
    }

    /// Advance all device clocks between steps without executing the CPU.
    pub fn advance_ns(&mut self, ns: u64) {
        self.bus.advance_ns(ns);
        self.scheduler.advance_external(ns);
    }

    /// Whole-second advancement avoids overflow even for u64::MAX seconds.
    pub fn advance_seconds(&mut self, seconds: u64) {
        self.bus.advance_seconds(seconds);
        self.scheduler
            .advance_external(seconds.saturating_mul(1_000_000_000));
    }

    /// Attach an SBus CG6 (TurboGX) card at the profile's slot-2 aperture.
    /// The caller supplies the board's original 32 KiB FCode ROM.
    pub fn install_cgsix(&mut self, card: Cgsix) -> Result<(), MachineError> {
        self.bus.install_cgsix(card).map_err(MachineError::Memory)
    }

    /// Remove the built-in SX/VSIMM device. Useful for an SS-20 configuration
    /// without a VSIMM, leaving an optional SBus CG6 as the display.
    pub fn remove_sx(&mut self) -> Option<Sx> {
        self.bus.remove_sx()
    }

    /// Cumulative counters since MMU construction/reset. This is available
    /// only with the opt-in `perf-counters` feature.
    #[cfg(feature = "perf-counters")]
    pub fn translation_counters(&self) -> crate::mmu::TranslationCounters {
        self.cpu.mmu.translation_counters()
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

    /// Diagnostic/reference switch for device-service guards. Production
    /// defaults to the exact readiness predicates; direct service APIs remain
    /// unconditional.
    pub fn set_device_service_guards(&mut self, enabled: bool) {
        self.service_guards = enabled;
    }

    pub fn scheduler_stats(&self) -> SchedulerStats {
        let mut stats = self.scheduler.stats;
        stats.virtual_ns = self.scheduler.virtual_ns;
        stats
    }

    /// Capture full CPU/MMU/FPU state, scheduler/clock timing and a deterministic
    /// fingerprint of bus, memory and device state for differential validation.
    pub fn diagnostic_snapshot(&self) -> MachineSnapshot {
        MachineSnapshot {
            cpu: snapshot_cpu(&self.cpu),
            secondary_cpu: self
                .secondary_cpu
                .as_ref()
                .map(|module| snapshot_cpu(&module.cpu)),
            scheduler: SchedulerSnapshot {
                virtual_ns: self.scheduler.virtual_ns,
                next_ready0: self.scheduler.next_ready0,
                next_ready2: self.scheduler.next_ready2,
                eligible0: self.scheduler.eligible0,
                eligible2: self.scheduler.eligible2,
                last_service_ns: self.scheduler.last_service_ns,
                stats: self.scheduler.stats,
            },
            clock: self.clock.diagnostic_state(),
            secondary_clock: self
                .secondary_cpu
                .as_ref()
                .map(|module| module.clock.diagnostic_state()),
            standalone_iommu: self.standalone_iommu.clone(),
            bus: self.bus.diagnostic_state(),
        }
    }

    /// Selected functional timebase: 10 MHz in Fast mode; profile oscillator
    /// (85/75 MHz) in Hardware mode. Diagnostic harnesses always use 10 MHz.
    pub fn cpu_clock_hz(&self) -> u64 {
        self.clock.hz
    }

    pub fn clock_mode(&self) -> CpuClockMode {
        self.clock.mode
    }

    /// Select the functional clock policy and perform a whole-machine reset.
    /// Both CPUs use the same policy. Firmware/OS clock calibration must rerun;
    /// changing only the running timebase would leave stale guest calibration.
    /// RAM, ROM and NVRAM retain the same contents as with `reset()`.
    pub fn reset_with_clock_mode(&mut self, mode: CpuClockMode) {
        self.clock.select(mode);
        if let Some(secondary) = &mut self.secondary_cpu {
            secondary.clock.select(mode);
        }
        self.reset();
    }

    fn service_devices(&mut self) -> Result<(), DmaError> {
        let (scsi, ethernet) = if self.service_guards {
            match self.bus.devices() {
                Some(devices) => (
                    devices.esp.needs_service()
                        || devices.esp.irq()
                        || devices.scsi_dma.irq()
                        || devices.scsi_dma.request_pending(),
                    devices.ethernet.needs_service()
                        || devices.ethernet.irq()
                        || devices.ethernet_dma.irq(),
                ),
                None => (false, false),
            }
        } else {
            (true, true)
        };
        if ethernet {
            self.service_ethernet()?;
        }
        if scsi {
            self.service_scsi_batch(16)?;
        }
        if !scsi {
            if let Some(devices) = self.bus.devices_mut() {
                if !devices.scsi_dma.reset_asserted() {
                    devices.sync_interrupts();
                }
            }
        }
        Ok(())
    }

    /// Execute one deterministic processor event. Device service occurs once
    /// per distinct deadline, then the earliest next-ready eligible hardware
    /// processor runs. Ties use hardware processor order 0 before 2. Only the
    /// selected CPU is charged one oscillator cycle plus PROM wait; devices
    /// advance to the earliest next eligible deadline, never past another
    /// CPU's earlier opportunity.
    pub fn step_scheduled(&mut self) -> Result<ScheduledStep, SchedulerError> {
        self.scheduled_clock_owned = true;
        if self.software_reset_requested() {
            let step = self.software_reset(self.cpu.pc(), self.cpu.npc(), None);
            return Ok(ScheduledStep {
                processor: 0,
                step,
                virtual_ns: self.scheduler.virtual_ns,
            });
        }

        let eligible0 = self.processor_arbitration(0).is_enabled();
        let eligible2 = self.secondary_cpu.is_some() && self.processor_arbitration(2).is_enabled();
        self.scheduler.sync_eligibility(eligible0, eligible2);

        let mut candidates = [(0u8, 0u64); 2];
        let mut count = 0;
        if eligible0 {
            candidates[count] = (0, self.scheduler.next_ready0);
            count += 1;
        }
        if eligible2 {
            candidates[count] = (2, self.scheduler.next_ready2);
            count += 1;
        }
        if count == 0 {
            return Err(SchedulerError::NoRunnable);
        }
        let (processor, ready) = candidates[..count]
            .iter()
            .copied()
            .min_by_key(|&(processor, ready)| (ready, processor))
            .unwrap();
        debug_assert!(ready >= self.scheduler.virtual_ns);
        if ready > self.scheduler.virtual_ns {
            let delta = ready - self.scheduler.virtual_ns;
            self.bus.advance_ns_fast(delta);
            self.scheduler.virtual_ns = ready;
        }
        // Service each distinct scheduling boundary once. Tied processors at
        // the same deadline observe the same device snapshot.
        if self.scheduler.last_service_ns != Some(ready) {
            self.service_devices().map_err(SchedulerError::Device)?;
            self.scheduler.last_service_ns = Some(ready);
        }

        if self
            .processor_cpu(processor)
            .is_some_and(|cpu| cpu.error_state().is_some())
        {
            // The original fault event was counted when it stopped the CPU.
            // Re-selecting a latched stop is not another retired event.
            let fault = self
                .processor_cpu(processor)
                .and_then(Cpu::error_state)
                .expect("scheduler selected a faulted CPU");
            return Err(SchedulerError::Cpu { processor, fault });
        }

        let was_error = self
            .processor_cpu(processor)
            .is_some_and(|cpu| cpu.error_state().is_some());
        let pending = self.bus.interrupt_levels_for(processor);
        self.bus.begin_cpu_step();
        let result = match processor {
            0 => self.cpu.step_with_interrupts(&mut self.bus, pending),
            2 => self
                .secondary_cpu
                .as_mut()
                .expect("secondary processor")
                .cpu
                .step_with_interrupts(&mut self.bus, pending),
            _ => unreachable!("scheduler processor"),
        };
        let wait_ns = self.bus.finish_cpu_step(result.is_ok());
        let newly_faulted = !was_error
            && self
                .processor_cpu(processor)
                .is_some_and(|cpu| cpu.error_state().is_some());
        if newly_faulted {
            if let Some(devices) = self.bus.devices_mut() {
                devices.reset.latch_watchdog();
            }
        }
        let step = match result {
            Ok(step) => step,
            Err(fault) => {
                self.scheduler.record_error(processor);
                return Err(SchedulerError::Cpu { processor, fault });
            }
        };
        let cycle_ns = match processor {
            0 => self.clock.next_ns(),
            2 => self.secondary_cpu.as_mut().unwrap().clock.next_ns(),
            _ => unreachable!("scheduler processor"),
        };
        let quantum = cycle_ns + wait_ns;
        self.scheduler
            .set_next_ready(processor, ready.saturating_add(quantum));
        let executed_step = step;
        let step = if self.software_reset_requested() {
            if let Step::Executed {
                pc,
                npc,
                instruction,
            } = step
            {
                // Whole-system reset stops both processors at this store.
                // Charge its completion time before clearing device state,
                // preserving the UP path and retained RTC/serial progress.
                self.bus.advance_ns_fast(quantum);
                self.software_reset(pc, npc, Some(instruction))
            } else {
                step
            }
        } else {
            step
        };
        // A completed reset store still retires as an instruction; the
        // SoftwareReset outcome is additional trace information.
        if matches!(step, Step::SoftwareReset { .. }) {
            self.scheduler.record(processor, executed_step);
        } else {
            self.scheduler.record(processor, step);
        }
        if !matches!(step, Step::SoftwareReset { .. }) {
            if let Some(next) = self.scheduler.earliest_ready() {
                if next > self.scheduler.virtual_ns {
                    let delta = next - self.scheduler.virtual_ns;
                    self.bus.advance_ns_fast(delta);
                    self.scheduler.virtual_ns = next;
                }
            }
        }
        Ok(ScheduledStep {
            processor,
            step,
            virtual_ns: self.scheduler.virtual_ns,
        })
    }

    /// Run at most `budget` scheduled processor events. Device-only and denied
    /// processor opportunities are not counted as instructions/events.
    pub fn run_scheduled(&mut self, budget: u64) -> Result<u64, SchedulerError> {
        let mut completed = 0;
        while completed < budget {
            self.step_scheduled()?;
            completed += 1;
        }
        Ok(completed)
    }

    /// Experimental safe JIT block run. Device-equipped machines currently
    /// use the interpreter so PROM wait accounting remains exact; device-free
    /// bare-metal machines execute validated straight-line SETHI/ALU blocks.
    #[cfg(feature = "jit")]
    pub fn run_jit(&mut self, budget: u64) -> Result<crate::jit::RunStats, SchedulerError> {
        let mut stats = crate::jit::RunStats::default();
        let mut cache = crate::jit::Cache::default();
        let pc = self.cpu.pc();
        let mut force_fallback = self.cpu.npc() != pc.wrapping_add(4);
        if self.software_reset_requested() {
            let step = self.step().map_err(|fault| SchedulerError::Cpu {
                processor: 0,
                fault,
            })?;
            stats.stopped = Some(step);
            return Ok(stats);
        }
        if self.cpu.error_state().is_some() {
            let fault = self.step().unwrap_err();
            return Err(SchedulerError::Cpu {
                processor: 0,
                fault,
            });
        }
        let mut jit_eligible = self.bus.devices().is_none()
            && self.secondary_cpu.is_none()
            && self.cpu.mmu.untranslated_execution();
        while stats.executed < budget {
            if force_fallback
                || !jit_eligible
                || !self.cpu.mmu.untranslated_execution()
                || self.bus.interrupt_levels() != 0
            {
                let expected_pc = self.cpu.npc();
                let expected_npc = expected_pc.wrapping_add(4);
                match self.step() {
                    Ok(step) => {
                        stats.fallback_steps += 1;
                        if matches!(step, Step::Executed { .. }) {
                            stats.executed += 1;
                            force_fallback =
                                self.cpu.pc() != expected_pc || self.cpu.npc() != expected_npc;
                        } else {
                            stats.stopped = Some(step);
                            break;
                        }
                    }
                    Err(fault) => {
                        return Err(SchedulerError::Cpu {
                            processor: 0,
                            fault,
                        });
                    }
                }
                continue;
            }
            let pc = self.cpu.pc();
            let max_words = usize::try_from(budget - stats.executed)
                .unwrap_or(usize::MAX)
                .clamp(1, crate::jit::MAX_BLOCK_INSNS);
            let last_byte = u64::from(pc) + (max_words as u64) * 4 - 1;
            if last_byte > self.cpu.mmu.config().physical_mask
                || !self.bus.is_ram_range(
                    u64::from(pc),
                    u64::try_from(max_words * 4).unwrap_or(u64::MAX),
                )
            {
                force_fallback = true;
                continue;
            }
            let supervisor = self.cpu.psr() & crate::cpu::PSR_S != 0;
            let key = crate::jit::Key {
                pc,
                supervisor,
                mutation_epoch: self.bus.mutation_epoch(),
                mmu_generation: self.cpu.mmu.execution_generation(),
            };
            let block = if let Some(block) = cache.get(&key).cloned() {
                block
            } else {
                stats.compile_calls += 1;
                if stats.compile_calls > crate::jit::MAX_COMPILE_ATTEMPTS {
                    jit_eligible = false;
                    force_fallback = true;
                    continue;
                }
                let started = Instant::now();
                let previous = self.bus.access_context();
                self.bus
                    .set_access_context(AccessContext::new(self.cpu.mmu.initiator(), supervisor));
                let asi = if supervisor { 9 } else { 8 };
                let mut words = Vec::with_capacity(max_words);
                for index in 0..max_words {
                    let address = pc.wrapping_add((index as u32) * 4);
                    let Ok(word) = self.cpu.mmu.load(&mut self.bus, address, asi, Width::Word)
                    else {
                        break;
                    };
                    if crate::jit::decode(word).is_none() {
                        break;
                    }
                    words.push(word);
                }
                self.bus.set_access_context(previous);
                let block = crate::jit::compile(&words);
                stats.compile_ns = stats.compile_ns.saturating_add(
                    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                );
                if !block.ops.is_empty() {
                    stats.compiled_blocks += 1;
                    stats.compiled_instructions += block.ops.len() as u64;
                }
                let key = crate::jit::Key {
                    mmu_generation: self.cpu.mmu.execution_generation(),
                    ..key
                };
                cache.insert(key, block.clone());
                block
            };
            if block.ops.is_empty() {
                let expected_pc = self.cpu.npc();
                let expected_npc = expected_pc.wrapping_add(4);
                match self.step() {
                    Ok(step) => {
                        stats.fallback_steps += 1;
                        if matches!(step, Step::Executed { .. }) {
                            stats.executed += 1;
                            force_fallback =
                                self.cpu.pc() != expected_pc || self.cpu.npc() != expected_npc;
                        } else {
                            stats.stopped = Some(step);
                            break;
                        }
                    }
                    Err(fault) => {
                        return Err(SchedulerError::Cpu {
                            processor: 0,
                            fault,
                        });
                    }
                }
                continue;
            }
            let mut block_ns = 0u64;
            for op in block.ops {
                if stats.executed >= budget {
                    break;
                }
                self.cpu.execute_jit_op(op);
                block_ns = block_ns.saturating_add(self.clock.next_ns());
                stats.executed += 1;
            }
            self.bus.advance_ns_fast(block_ns);
        }
        Ok(stats)
    }

    /// Native straight-line SETHI/ALU backend. Admission is intentionally
    /// narrow: device-free, single-CPU, MMU/boot-translation-disabled ordinary
    /// RAM. Everything else uses the interpreter boundary.
    #[cfg(feature = "jit-native")]
    pub fn run_jit_native(
        &mut self,
        budget: u64,
    ) -> Result<crate::jit::NativeRunStats, SchedulerError> {
        let mut stats = crate::jit::NativeRunStats::default();
        let mut cache: helios_jit_native::Cache = helios_jit_native::Cache::new();
        let pc = self.cpu.pc();
        let mut force_fallback = self.cpu.npc() != pc.wrapping_add(4);
        if self.software_reset_requested() {
            let step = self.step().map_err(|fault| SchedulerError::Cpu {
                processor: 0,
                fault,
            })?;
            stats.stopped = Some(step);
            return Ok(stats);
        }
        if self.cpu.error_state().is_some() {
            let fault = self.step().unwrap_err();
            return Err(SchedulerError::Cpu {
                processor: 0,
                fault,
            });
        }
        let mut eligible = self.bus.devices().is_none()
            && self.secondary_cpu.is_none()
            && self.cpu.mmu.untranslated_execution();
        while stats.executed < budget {
            if force_fallback
                || !eligible
                || !self.cpu.mmu.untranslated_execution()
                || self.bus.interrupt_levels() != 0
            {
                let expected_pc = self.cpu.npc();
                let expected_npc = expected_pc.wrapping_add(4);
                match self.step() {
                    Ok(step) => {
                        stats.fallback_steps += 1;
                        if matches!(step, Step::Executed { .. }) {
                            stats.executed += 1;
                            force_fallback =
                                self.cpu.pc() != expected_pc || self.cpu.npc() != expected_npc;
                        } else {
                            stats.stopped = Some(step);
                            break;
                        }
                    }
                    Err(fault) => {
                        return Err(SchedulerError::Cpu {
                            processor: 0,
                            fault,
                        });
                    }
                }
                continue;
            }
            let pc = self.cpu.pc();
            let max_words = usize::try_from(budget - stats.executed)
                .unwrap_or(usize::MAX)
                .clamp(1, helios_jit_native::MAX_BLOCK_INSNS);
            let last_byte = u64::from(pc) + (max_words as u64) * 4 - 1;
            if last_byte > self.cpu.mmu.config().physical_mask
                || !self.bus.is_ram_range(
                    u64::from(pc),
                    u64::try_from(max_words * 4).unwrap_or(u64::MAX),
                )
            {
                force_fallback = true;
                continue;
            }
            let key = (pc, self.bus.mutation_epoch());
            if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(key) {
                stats.compile_calls += 1;
                if stats.compile_calls > crate::jit::MAX_COMPILE_ATTEMPTS {
                    eligible = false;
                    force_fallback = true;
                    continue;
                }
                let started = Instant::now();
                let mut words = Vec::with_capacity(max_words);
                for index in 0..max_words {
                    let Ok(word) = self.bus.read32(u64::from(pc) + (index as u64) * 4) else {
                        break;
                    };
                    if helios_jit_native::decode(word).is_none() {
                        break;
                    }
                    words.push(word);
                }
                let block = helios_jit_native::compile(&words);
                stats.compile_ns = stats.compile_ns.saturating_add(
                    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                );
                if block.is_some() {
                    stats.native_blocks += 1;
                    stats.native_instructions += words.len() as u64;
                }
                entry.insert(block);
            }
            let Some(block) = cache.get(&key).and_then(Option::as_ref) else {
                let expected_pc = self.cpu.npc();
                let expected_npc = expected_pc.wrapping_add(4);
                match self.step() {
                    Ok(step) => {
                        stats.fallback_steps += 1;
                        if matches!(step, Step::Executed { .. }) {
                            stats.executed += 1;
                            force_fallback =
                                self.cpu.pc() != expected_pc || self.cpu.npc() != expected_npc;
                        } else {
                            stats.stopped = Some(step);
                            break;
                        }
                    }
                    Err(fault) => {
                        return Err(SchedulerError::Cpu {
                            processor: 0,
                            fault,
                        });
                    }
                }
                continue;
            };
            if block.instructions() as u64 > budget - stats.executed {
                force_fallback = true;
                continue;
            }
            let mut state = helios_jit_native::State::default();
            for (index, slot) in state.regs.iter_mut().enumerate() {
                *slot = self.cpu.read_reg(index as u8);
            }
            state.icc = pack_icc(self.cpu.icc());
            block.execute(&mut state);
            for (index, value) in state.regs.iter().enumerate().skip(1) {
                self.cpu.write_reg(index as u8, *value);
            }
            self.cpu.set_icc(unpack_icc(state.icc));
            let count = block.instructions() as u64;
            self.cpu.set_pc_npc(
                pc.wrapping_add((count as u32).wrapping_mul(4)),
                pc.wrapping_add((count as u32).wrapping_add(1).wrapping_mul(4)),
            );
            let mut block_ns = 0u64;
            for _ in 0..count {
                block_ns = block_ns.saturating_add(self.clock.next_ns());
            }
            self.bus.advance_ns_fast(block_ns);
            stats.executed += count;
        }
        Ok(stats)
    }

    /// Single-CPU compatibility path for processor 0. Two-CPU callers must use
    /// `step_scheduled`, which tags outcomes and advances one shared device
    /// clock across independent CPU deadlines. Do not mix the two step APIs.
    pub fn step(&mut self) -> Result<Step, CpuFault> {
        if self.software_reset_requested() {
            return Ok(self.software_reset(self.cpu.pc(), self.cpu.npc(), None));
        }
        self.service_devices().map_err(|error| CpuFault {
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
        self.bus.advance_ns_fast(self.clock.next_ns() + wait_ns);
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
        self.clock.reset();
        if let Some(secondary) = &mut self.secondary_cpu {
            secondary.cpu.software_reset_sun4m();
            secondary.clock.reset();
        }
        self.bus.software_reset_devices();
        if let Some(iommu) = &mut self.standalone_iommu {
            iommu.reset();
        }
        self.scheduler.reset();
        self.scheduled_clock_owned = false;
        self.reset_epoch = self.reset_epoch.wrapping_add(1);
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
