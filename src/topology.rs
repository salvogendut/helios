//! Explicit MBus processor-module topology for sun4m profiles.
//!
//! Hardware processor numbers and MIDs are not compact storage indices. The
//! two single-CPU SuperSPARC modules implemented by this slice occupy MBus
//! connectors 0 and 1, with hardware processor numbers 0 and 2, MIDs 8 and 10.
//! Slots for processor numbers 1 and 3 (MIDs 9 and 11) remain absent.

pub const MAX_PROCESSOR_MODULES: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessorModule {
    pub storage_index: u8,
    pub connector: u8,
    pub processor: u8,
    pub mid: u8,
    pub mxcc_base: u64,
    pub mxcc_port: u64,
    pub mxcc_identity: u32,
}

pub const PROCESSOR_MODULES: [ProcessorModule; MAX_PROCESSOR_MODULES] = [
    ProcessorModule {
        storage_index: 0,
        connector: 0,
        processor: 0,
        mid: 8,
        mxcc_base: 0x000f_f8c0_0000,
        mxcc_port: 0x000f_f8ff_fffc,
        mxcc_identity: 0x0800_0104,
    },
    ProcessorModule {
        storage_index: 1,
        connector: 1,
        processor: 2,
        mid: 10,
        mxcc_base: 0x000f_fac0_0000,
        mxcc_port: 0x000f_faff_fffc,
        mxcc_identity: 0x0a00_0104,
    },
];

impl ProcessorModule {
    pub const fn from_storage_index(index: usize) -> Option<Self> {
        if index < MAX_PROCESSOR_MODULES {
            Some(PROCESSOR_MODULES[index])
        } else {
            None
        }
    }

    pub const fn from_processor(processor: u8) -> Option<Self> {
        match processor {
            0 => Some(PROCESSOR_MODULES[0]),
            2 => Some(PROCESSOR_MODULES[1]),
            _ => None,
        }
    }

    pub const fn from_mid(mid: u8) -> Option<Self> {
        match mid {
            8 => Some(PROCESSOR_MODULES[0]),
            10 => Some(PROCESSOR_MODULES[1]),
            _ => None,
        }
    }

    pub const fn mxcc_control_address(self) -> u64 {
        self.mxcc_base + 0xa00
    }

    pub const fn contains_mxcc_register(self, address: u64) -> bool {
        (address >= self.mxcc_port && address < self.mxcc_port + 4)
            || (address >= self.mxcc_base && address < self.mxcc_base + 0x1000)
    }

    pub const fn mxcc_register(self, address: u64) -> Option<u32> {
        if address >= self.mxcc_port && address < self.mxcc_port + 4 {
            return Some(0x01c0_0f04 | (address - self.mxcc_port) as u32);
        }
        if address < self.mxcc_base || address >= self.mxcc_base + 0x1000 {
            return None;
        }
        Some(0x01c0_0000 | (address - self.mxcc_base) as u32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessorTopology {
    modules: [Option<ProcessorModule>; MAX_PROCESSOR_MODULES],
}

impl ProcessorTopology {
    pub const fn empty() -> Self {
        Self {
            modules: [None; MAX_PROCESSOR_MODULES],
        }
    }

    pub const fn single() -> Self {
        Self {
            modules: [Some(PROCESSOR_MODULES[0]), None],
        }
    }

    pub const fn dual() -> Self {
        Self {
            modules: [Some(PROCESSOR_MODULES[0]), Some(PROCESSOR_MODULES[1])],
        }
    }

    pub const fn with_modules(modules: [Option<ProcessorModule>; MAX_PROCESSOR_MODULES]) -> Self {
        Self { modules }
    }

    pub fn modules(&self) -> impl Iterator<Item = ProcessorModule> + '_ {
        self.modules.into_iter().flatten()
    }

    pub fn len(&self) -> usize {
        self.modules.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn module_for_processor(&self, processor: u8) -> Option<ProcessorModule> {
        self.modules().find(|module| module.processor == processor)
    }

    pub fn module_for_mid(&self, mid: u8) -> Option<ProcessorModule> {
        self.modules().find(|module| module.mid == mid)
    }

    pub fn contains_mxcc_register(&self, address: u64) -> bool {
        self.modules()
            .any(|module| module.contains_mxcc_register(address))
    }
}

impl Default for ProcessorTopology {
    fn default() -> Self {
        Self::single()
    }
}

/// Board-level processor arbitration eligibility. Sun-4M 5.1.2 gives processor
/// 0 no enable bit (AER bit 0 reads one), processor 1 the split-requester MID-9
/// enable, and processors 2/3 the MID-A/MID-B enables. Configuration presence is
/// independent: enabling an unpopulated MID never creates a CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessorArbitration {
    /// No processor module occupies this hardware processor number.
    Absent,
    /// A module is configured but its AER enable bit is clear.
    Disabled,
    /// A configured module may request the bus.
    Enabled,
}

impl ProcessorArbitration {
    pub const fn is_present(self) -> bool {
        !matches!(self, Self::Absent)
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Combined module state for a future scheduler. It deliberately separates
/// board arbitration denial from a CPU that has latched error mode. A module
/// reset is an explicit host transition, not a persistent run state; a guest
/// firmware wait loop is ordinary guest code, not an emulator state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessorRunState {
    Ready,
    ArbitrationDenied,
    CpuError,
    Absent,
}
