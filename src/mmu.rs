//! microSPARC-II SRMMU. See docs/memory.md for the supported control spaces,
//! implementation choices, and differences from generic SPARC Reference MMUs.

use crate::bus::{Bus, BusError, BusErrorKind, IoTlbFlush, MemoryRequest, MemoryRequestKind};
use crate::cache::CacheDiagnostics;
use crate::devices::io_control::IoMapping;
use crate::devices::mxcc::MxccState;
use crate::dma::{DmaError, DmaFault};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const PCR_EN: u32 = 1;
pub const PCR_NF: u32 = 2;
pub const PCR_BM: u32 = 1 << 14;
pub const PROM_BASE: u64 = 0x7000_0000;
const PCR_VERSION: u32 = 0x0400_0000;

/// Address-translation identity of one sun4m CPU/profile. The microSPARC-II
/// reports 31-bit physical addresses and a 1 MiB boot PROM window; the
/// SuperSPARC reference MMU used by the SS-20/Axil reports 36 bits and a
/// 512 KiB boot window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MmuConfig {
    pub prom_base: u64,
    pub boot_mask: u32,
    pub physical_mask: u64,
    pub pte_physical_mask: u32,
    pub pte_table_mask: u32,
    pub reference_mmu: bool,
    /// Boot-mode control bit: bit 14 on microSPARC-II, bit 13 on SuperSPARC.
    pub boot_mode_bit: u32,
    pub control_mask: u32,
    pub context_table_mask: u32,
    pub context_mask: u32,
    pub fault_status_mask: u32,
    pub replacement_mask: u32,
    pub mmu_version: u32,
}

impl MmuConfig {
    pub const fn ss5() -> Self {
        Self {
            prom_base: PROM_BASE,
            boot_mask: 0x0fff_ffff,
            physical_mask: 0x7fff_ffff,
            pte_physical_mask: 0x07ff_ff00,
            pte_table_mask: 0x07ff_fff0,
            reference_mmu: false,
            boot_mode_bit: PCR_BM,
            control_mask: 0x003e_ff03,
            context_table_mask: 0x00ff_ffc0,
            context_mask: 0x0000_00ff,
            fault_status_mask: 0x0001_6fff,
            replacement_mask: 0x01f1_ffff,
            mmu_version: PCR_VERSION,
        }
    }

    #[allow(clippy::unusual_byte_groupings)]
    pub const fn ss20() -> Self {
        Self {
            prom_base: 0xf_f000_0000,
            boot_mask: 0x0007_ffff,
            physical_mask: 0xffff_ffff,
            pte_physical_mask: 0xffff_ff00,
            pte_table_mask: 0xffff_fff0,
            reference_mmu: true,
            boot_mode_bit: 1 << 13,
            control_mask: 0x00ff_ffff,
            context_table_mask: 0xffff_ffc0,
            context_mask: 0x0000_ffff,
            fault_status_mask: 0xffff_ffff,
            replacement_mask: 0xffff_ffff,
            mmu_version: 0x0800_0000,
        }
    }
}
/// IOPTE physical page bits are 26:8. The reference-MMU config's PTE mask also
/// covers CPU PTE bits 31:28, which translate_io sets only for TLB diagnostics.
const IOPTE_PHYSICAL_MASK: u32 = 0x07ff_ff00;
const REFERENCED: u32 = 1 << 5;
const MODIFIED: u32 = 1 << 6;
const TLB_ENTRIES: usize = 64;
const FAST_TLB_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    fn store_kind(self) -> MemoryRequestKind {
        match self {
            Self::Byte => MemoryRequestKind::WriteByte,
            Self::Half => MemoryRequestKind::WriteHalf,
            Self::Word => MemoryRequestKind::WriteWord,
        }
    }
    fn alignment(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FaultType {
    Invalid = 1,
    Protection = 2,
    Privilege = 3,
    Translation = 4,
    AccessBus = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MmuFault {
    pub address: u32,
    pub kind: FaultType,
    pub level: u8,
    /// SFSR AT encoding: supervisor | instruction << 1 | write << 2.
    pub access_type: u8,
    /// Physical failure during translation, when applicable. This preserves
    /// SBus cause bits through page walks and fault-priority arbitration.
    pub bus_error: Option<BusErrorKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryOperation {
    LoadDoubleword,
    StoreDoubleword,
    Swap,
    LoadStoreByte,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    Bus(BusError),
    Translation(MmuFault),
    /// Unimplemented emulator behavior, not a guest hardware exception.
    UnsupportedAsi {
        asi: u8,
        width: Width,
        write: bool,
    },
    UnsupportedRegister(u32),
    UnsupportedControl(u32),
    UnsupportedRootPte,
    UnsupportedTlbDiagnostic {
        address: u32,
        value: u32,
    },
    /// A diagnostic entry without a page-table origin needs a hardware rewalk
    /// to set M; that path is not implemented yet.
    UnsupportedTlbModifiedUpdate,
    UnsupportedOperation {
        asi: u8,
        operation: MemoryOperation,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Access {
    address: u32,
    asi: u8,
    write: bool,
}

struct Translation {
    physical: u64,
    cacheable: bool,
}

impl Access {
    fn instruction(self) -> bool {
        matches!(self.asi, 8 | 9)
    }

    fn supervisor(self) -> bool {
        self.asi & 1 != 0 || self.asi == 0x20 || self.asi == 3
    }

    fn fault(self, kind: FaultType, level: u8) -> MmuFault {
        MmuFault {
            address: self.address,
            kind,
            level,
            access_type: u8::from(self.supervisor())
                | (u8::from(self.instruction()) << 1)
                | (u8::from(self.write) << 2),
            bus_error: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entry {
    virtual_base: u32,
    offset_mask: u32,
    context: u32,
    level: u8,
    pte_address: Option<u64>,
    pte: u32,
    upper: u32,
    lower: u32,
}

impl Entry {
    fn matches(self, address: u32, context: u32, supervisor: bool, mask: u32) -> bool {
        let global = self.lower & 8 != 0;
        self.upper & 0xc == 8
            && self.lower & 6 == 0
            && address & !self.offset_mask == self.virtual_base
            && ((context & mask) == (self.context & mask) || (supervisor && global))
    }

    fn retag(&mut self, upper: u32) {
        self.upper = upper;
        self.context = upper >> 4;
        (self.level, self.offset_mask) = match upper & 7 {
            3 => (1, 0x00ff_ffff),
            1 => (2, 0x0003_ffff),
            0 => (3, 0xfff),
            _ => (0, u32::MAX),
        };
        self.virtual_base = upper & !self.offset_mask;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FastTlbEntry {
    generation: u64,
    tag: u64,
    supervisor: bool,
    index: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FastTlb {
    entries: [FastTlbEntry; FAST_TLB_ENTRIES],
}

impl Default for FastTlb {
    fn default() -> Self {
        Self {
            entries: [FastTlbEntry::default(); FAST_TLB_ENTRIES],
        }
    }
}

impl FastTlb {
    fn get(&self, address: u32, context: u32, supervisor: bool, generation: u64) -> Option<usize> {
        let slot = &self.entries[((address >> 12) as usize) & (FAST_TLB_ENTRIES - 1)];
        (slot.generation == generation
            && slot.supervisor == supervisor
            && slot.tag == Self::tag(address, context))
        .then_some(usize::from(slot.index))
    }

    fn put(&mut self, address: u32, context: u32, supervisor: bool, generation: u64, index: usize) {
        self.entries[((address >> 12) as usize) & (FAST_TLB_ENTRIES - 1)] = FastTlbEntry {
            generation,
            tag: Self::tag(address, context),
            supervisor,
            index: index as u8,
        };
    }

    fn tag(address: u32, context: u32) -> u64 {
        u64::from(address & !0xfff) | (u64::from(context) << 32)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srmmu {
    config: MmuConfig,
    control: u32,
    context_table: u32,
    context: u32,
    fault_status: u32,
    fault_address: u32,
    tlb: [Entry; TLB_ENTRIES],
    tlb_generation: u64,
    tlb_diagnostic: Option<(u32, u32)>,
    fast_instruction: FastTlb,
    fast_data: FastTlb,
    trcr: u32,
    caches: CacheDiagnostics,
    /// SuperSPARC MMU breakpoint/counter registers (ASI 0x38, 0x49..=0x4c).
    breakpoints: [u64; 4],
    counter_value: u32,
    counter_control: u32,
    counter_status: u32,
    breakpoint_action: u32,
    /// SuperSPARC MXCC state shared with the physical module-control alias.
    mxcc: Rc<RefCell<MxccState>>,
    /// SuperSPARC II I/D-TLB diagnostic storage (ASI 0x05/0x06), keyed by
    /// the full diagnostic address so entry/tag writes never alias.
    itlb_diag: HashMap<u32, u32>,
    dtlb_diag: HashMap<u32, u32>,
    /// Diagnostic view of the SuperSPARC II cached level-2 PTPs (ASI 5/6
    /// SEL 5/6). The reference firmware checks these after a 16 MiB TLB fill.
    itlb_ptp2: Option<(u32, u32)>,
    dtlb_ptp2: [Option<(u32, u32)>; 4],
    dtlb_ptp2_next: usize,
}

impl Default for Srmmu {
    /// Translation and boot mode disabled, for the bare-metal harness.
    fn default() -> Self {
        Self {
            config: MmuConfig::ss5(),
            control: 0,
            context_table: 0,
            context: 0,
            fault_status: 0,
            fault_address: 0,
            tlb: [Entry::default(); TLB_ENTRIES],
            tlb_generation: 1,
            tlb_diagnostic: None,
            fast_instruction: FastTlb::default(),
            fast_data: FastTlb::default(),
            // Functional reset choice: all 64 entries initially replaceable.
            trcr: 63 << 7,
            caches: CacheDiagnostics::default(),
            breakpoints: [0; 4],
            counter_value: 0,
            counter_control: 0,
            counter_status: 0,
            breakpoint_action: 0,
            mxcc: Rc::new(RefCell::new(MxccState::default())),
            itlb_diag: HashMap::new(),
            dtlb_diag: HashMap::new(),
            itlb_ptp2: None,
            dtlb_ptp2: [None; 4],
            dtlb_ptp2_next: 0,
        }
    }
}

impl Srmmu {
    fn request(
        &self,
        bus: &mut impl Bus,
        physical: u64,
        kind: MemoryRequestKind,
        cpu: bool,
        cacheable: bool,
    ) {
        bus.observe_memory_request(
            physical,
            MemoryRequest {
                kind,
                cpu,
                boot_mode: self.boot_mode(),
                cacheable,
            },
        );
    }

    fn finish_store(
        &mut self,
        bus: &mut impl Bus,
        access: Access,
        bytes: u8,
        result: Result<(), MemoryError>,
    ) -> Result<(), MemoryError> {
        if let Err(MemoryError::Bus(error)) = result {
            if matches!(
                error.kind,
                BusErrorKind::SBusTimeout | BusErrorKind::SBusError
            ) {
                if bytes == 8 {
                    return Err(MemoryError::Bus(BusError {
                        kind: BusErrorKind::Unsupported("doubleword SBus fault completion"),
                        ..error
                    }));
                }
                if bus.capture_sbus_write_error(error, bytes) {
                    return Ok(());
                }
            }
        }
        self.finish(access, result)
    }
    pub fn reset() -> Self {
        Self::reset_with(MmuConfig::ss5())
    }

    pub fn reset_with(config: MmuConfig) -> Self {
        let mut mmu = Self {
            config,
            control: config.boot_mode_bit,
            ..Self::default()
        };
        mmu.caches.set_reference(config.reference_mmu);
        mmu
    }

    pub fn config(&self) -> MmuConfig {
        self.config
    }

    pub fn control(&self) -> u32 {
        self.config.mmu_version | self.control
    }

    pub fn boot_mode(&self) -> bool {
        self.control & self.config.boot_mode_bit != 0
    }

    /// Non-destructive debugger views. ASI 4 also has destructive read aliases.
    pub fn fault_status(&self) -> u32 {
        self.fault_status
    }

    pub fn fault_address(&self) -> u32 {
        self.fault_address
    }

    /// Unprivileged FLUSH: no translation, bus access or TLB invalidation.
    pub(crate) fn flush_instruction(&mut self, address: u32) {
        self.caches.flush_instruction(address);
    }

    fn flush_matching(&mut self, address: u32) {
        // Diagnostic entries use the low page index in their tag word (SEL 0).
        // A demap invalidates the cached PTE's V bit, not the whole register.
        let mask = match (address >> 8) & 7 {
            0 => 0xffff_f000,
            1 => 0xfffc_0000,
            _ => 0xff00_0000,
        };
        for table in [&mut self.itlb_diag, &mut self.dtlb_diag] {
            let bases: Vec<u32> = table
                .keys()
                .filter(|key| *key & 0xfff == 0)
                .copied()
                .collect();
            for base in bases {
                if table.get(&base).copied().unwrap_or(0) & mask == address & mask {
                    if let Some(value) = table.get_mut(&(base + 0x200)) {
                        *value &= !0x20;
                    }
                }
            }
        }
        for entry in &mut self.tlb {
            if entry.upper & mask == address & mask {
                entry.upper &= !8;
            }
        }
        let tag_mask = address & 0xfffc_0000;
        for entry in &mut self.dtlb_ptp2 {
            if entry.is_some_and(|(tag, _)| tag & 0xfffc_0000 == tag_mask) {
                *entry = None;
            }
        }
        if self
            .itlb_ptp2
            .is_some_and(|(tag, _)| tag & 0xfffc_0000 == tag_mask)
        {
            self.itlb_ptp2 = None;
        }
        self.refresh_tlb_diagnostic();
    }

    fn flush_context(&mut self) {
        // SEL 1 carries the tag context; SEL 2's V bit is cleared for the
        // matching entries. Entries of other contexts survive.
        let context = self.context;
        for table in [&mut self.itlb_diag, &mut self.dtlb_diag] {
            let bases: Vec<u32> = table
                .keys()
                .filter(|key| *key & 0xfff == 0)
                .copied()
                .collect();
            for base in bases {
                if table.get(&(base + 0x100)).copied().unwrap_or(0) & 0xff == (context & 0xff) {
                    if let Some(value) = table.get_mut(&(base + 0x200)) {
                        *value &= !0x20;
                    }
                }
            }
        }
        for entry in &mut self.tlb {
            if (entry.upper >> 4) & 0xff == context {
                entry.upper &= !8;
            }
        }
        self.refresh_tlb_diagnostic();
    }

    fn flush(&mut self) {
        for entry in &mut self.tlb {
            entry.upper &= !8;
        }
        if self.config.reference_mmu {
            for table in [&mut self.itlb_diag, &mut self.dtlb_diag] {
                let keys: Vec<u32> = table
                    .keys()
                    .filter(|key| *key & 0xfff == 0x200)
                    .copied()
                    .collect();
                for key in keys {
                    if let Some(value) = table.get_mut(&key) {
                        *value &= !0x20;
                    }
                }
            }
        }
        self.refresh_tlb_diagnostic();
    }

    fn sync_io_flushes(&mut self, bus: &mut impl Bus) {
        let mut changed = false;
        while let Some(flush) = bus.take_io_tlb_flush() {
            for entry in &mut self.tlb {
                let matches = match flush {
                    IoTlbFlush::All => true,
                    IoTlbFlush::Page(address) => entry.upper & !0xfff == address,
                };
                if entry.lower & 4 != 0 && matches {
                    entry.upper &= !8;
                    changed = true;
                }
            }
        }
        if changed {
            self.refresh_tlb_diagnostic();
        }
    }

    fn insert(&mut self, entry: Entry) -> usize {
        let wrap = ((self.trcr >> 7) & 63) as usize;
        let limit = if self.trcr & (1 << 13) != 0 {
            wrap.min(47)
        } else {
            wrap
        };
        let counter = (self.trcr & 63) as usize;
        let mut index = if entry.lower & 4 != 0 && self.trcr & (1 << 13) != 0 {
            48 + (counter & 15)
        } else {
            counter % (limit + 1)
        };
        // Locked PTP slots redirect leaf fills to locations 3, 4 and 5.
        if index < 3 && self.trcr & (1 << (14 + index)) != 0 {
            index += 3;
        }
        self.tlb[index] = entry;
        // Functional scheduling: advance on fills, not on individual CPU clocks.
        if self.trcr & (1 << 6) == 0 {
            self.trcr = (self.trcr & !63) | ((counter + 1) % (limit + 1)) as u32;
        }
        self.tlb_generation = self.tlb_generation.wrapping_add(1);
        let diagnostic = entry.upper & 8 != 0 && (entry.lower & 2 != 0 || entry.upper & 7 == 2);
        if diagnostic {
            self.tlb_diagnostic = Some((0x300 | (index as u32 * 4), entry.upper));
        } else if self.tlb_diagnostic.is_some() {
            self.tlb_diagnostic = self.find_tlb_diagnostic();
        }
        index
    }

    fn diagnostic_read(&self, address: u32) -> Result<u32, MemoryError> {
        let entry = self.tlb[((address >> 2) & 63) as usize];
        match address >> 8 {
            0 => Ok(if entry.lower & 2 != 0 {
                (entry.pte & 0xe7ff_fff0) | 1
            } else if entry.lower & 4 != 0 {
                (entry.pte & 0xe7ff_ff04) | 2
            } else {
                (entry.pte & 0xe7ff_ffdc) | 0x22
            }),
            1 => Ok(entry.lower),
            3 => Ok(entry.upper),
            _ => Err(MemoryError::UnsupportedRegister(address)),
        }
    }

    fn diagnostic_write(&mut self, address: u32, value: u32) -> Result<(), MemoryError> {
        let slot = &mut self.tlb[((address >> 2) & 63) as usize];
        let mut entry = *slot;
        match address >> 8 {
            0 => entry.pte = value & 0xe7ff_fffc,
            1 => entry.lower = value & 0x3ff,
            3 => entry.retag(value),
            _ => return Err(MemoryError::UnsupportedRegister(address)),
        }
        // RAM tests can install arbitrary patterns with translation disabled.
        // Unsupported active types are checked at lookup, not on diagnostic writes.
        entry.pte_address = None;
        *slot = entry;
        self.refresh_tlb_diagnostic();
        Ok(())
    }

    fn find_tlb_diagnostic(&self) -> Option<(u32, u32)> {
        self.tlb.iter().enumerate().find_map(|(index, entry)| {
            (entry.upper & 8 != 0 && (entry.lower & 2 != 0 || entry.upper & 7 == 2))
                .then_some((0x300 | (index as u32 * 4), entry.upper))
        })
    }

    fn refresh_tlb_diagnostic(&mut self) {
        self.tlb_generation = self.tlb_generation.wrapping_add(1);
        self.tlb_diagnostic = self.find_tlb_diagnostic();
    }

    fn check_tlb_diagnostic(&self) -> Result<(), MemoryError> {
        match self.tlb_diagnostic {
            Some((address, value)) => Err(MemoryError::UnsupportedTlbDiagnostic { address, value }),
            None => Ok(()),
        }
    }

    fn record(&mut self, fault: MmuFault) {
        fn priority(kind: u32, access: u32) -> u8 {
            if kind == FaultType::Translation as u32 {
                3
            } else if access & 2 == 0 {
                2
            } else {
                1
            }
        }
        let old_kind = (self.fault_status >> 2) & 7;
        let old_priority = priority(old_kind, (self.fault_status >> 5) & 7);
        let new_priority = priority(fault.kind as u32, u32::from(fault.access_type));
        if old_kind != 0 && old_priority > new_priority {
            return;
        }
        let valid_address = fault.access_type & 2 == 0 || fault.kind == FaultType::Translation;
        let status = match fault.bus_error {
            Some(BusErrorKind::SBusTimeout) => 1 << 11,
            Some(BusErrorKind::SBusError) => 1 << 10,
            _ => 0,
        };
        self.fault_status = status
            | (u32::from(fault.level) << 8)
            | (u32::from(fault.access_type) << 5)
            | ((fault.kind as u32) << 2)
            | (u32::from(valid_address) << 1)
            | u32::from(old_kind != 0 && old_priority == new_priority);
        if valid_address {
            self.fault_address = fault.address;
        }
    }

    fn finish<T: Default>(
        &mut self,
        access: Access,
        result: Result<T, MemoryError>,
    ) -> Result<T, MemoryError> {
        let fault = match result {
            Err(MemoryError::Translation(fault)) => fault,
            Err(MemoryError::Bus(error))
                if matches!(
                    error.kind,
                    BusErrorKind::Unmapped
                        | BusErrorKind::ReadOnly
                        | BusErrorKind::SBusTimeout
                        | BusErrorKind::SBusError
                ) =>
            {
                MmuFault {
                    bus_error: Some(error.kind),
                    ..access.fault(FaultType::AccessBus, 0)
                }
            }
            _ => return result,
        };
        self.record(fault);
        if self.control & PCR_NF != 0 && !access.instruction() {
            // Failed reads have an implementation-dependent value. Never issue
            // a denied access, including stores, merely because NF is set.
            Ok(T::default())
        } else {
            result
        }
    }

    pub fn load(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
        width: Width,
    ) -> Result<u32, MemoryError> {
        self.sync_io_flushes(bus);
        Self::aligned(address, width)?;
        match asi {
            3 if width == Width::Word => return self.probe(bus, address),
            4 if width == Width::Word => return self.read_register(address),
            5 if width == Width::Word && self.config.reference_mmu => {
                if let Some(value) = self.read_ptp2(address, true) {
                    return Ok(value);
                }
                return Ok(*self.itlb_diag.get(&address).unwrap_or(&0));
            }
            6 if width == Width::Word => {
                return if self.config.reference_mmu {
                    let value = self
                        .read_ptp2(address, false)
                        .unwrap_or_else(|| *self.dtlb_diag.get(&address).unwrap_or(&0));
                    Ok(value)
                } else {
                    self.diagnostic_read(address)
                };
            }
            0x0c..=0x0e if width == Width::Word => {
                return Ok(self.caches.read(address, asi, width));
            }
            0x0f => return Ok(self.caches.read(address, asi, width)),
            0x30..=0x32 | 0x36 | 0x37 | 0x39 if width == Width::Word => return Ok(0),
            0x38 | 0x49..=0x4c if width == Width::Word => {
                return Ok(self.reference_diagnostic_read(address, asi));
            }
            2 if width == Width::Word => {
                return Ok(self.mxcc.borrow().read_word(address));
            }
            8..=11 | 0x20 => {}
            0x21..=0x2f if self.config.reference_mmu => {}
            _ => {
                return Err(MemoryError::UnsupportedAsi {
                    asi,
                    width,
                    write: false,
                })
            }
        }
        let access = Access {
            address,
            asi,
            write: false,
        };
        let result = self.translate(bus, access).and_then(|translation| {
            let physical = translation.physical;
            self.request(
                bus,
                physical,
                MemoryRequestKind::Read,
                true,
                translation.cacheable,
            );
            match width {
                Width::Byte => bus.read8(physical).map(u32::from),
                Width::Half => bus.read16(physical).map(u32::from),
                Width::Word => bus.read32(physical),
            }
            .map_err(MemoryError::Bus)
        });
        self.finish(access, result)
    }

    pub fn store(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
        width: Width,
        value: u32,
    ) -> Result<(), MemoryError> {
        self.sync_io_flushes(bus);
        Self::aligned(address, width)?;
        match asi {
            3 if width == Width::Word => {
                match (address >> 8) & 15 {
                    0..=2 => self.flush_matching(address),
                    3 => self.flush_context(),
                    4 => self.flush(),
                    _ => {}
                }
                return Ok(());
            }
            4 if width == Width::Word => return self.write_register(address, value),
            5 if width == Width::Word && self.config.reference_mmu => {
                self.write_ptp2(address, value, true);
                self.itlb_diag.insert(address, value);
                if (address >> 8) & 7 <= 3 {
                    self.install_diag_entry(address, true);
                }
                return Ok(());
            }
            6 if width == Width::Word => {
                if self.config.reference_mmu {
                    self.write_ptp2(address, value, false);
                    self.dtlb_diag.insert(address, value);
                    if (address >> 8) & 7 <= 3 {
                        self.install_diag_entry(address, false);
                    }
                    return Ok(());
                }
                return self.diagnostic_write(address, value);
            }
            0x0c..=0x0e if width == Width::Word => {
                self.caches.write(address, asi, width, value);
                return Ok(());
            }
            0x0f => {
                self.caches.write(address, asi, width, value);
                return Ok(());
            }
            0x30..=0x32 | 0x36 | 0x37 | 0x39 if width == Width::Word => return Ok(()),
            0x38 | 0x49..=0x4c if width == Width::Word => {
                self.reference_diagnostic_write(address, asi, u64::from(value));
                return Ok(());
            }
            2 if width == Width::Word => {
                self.mxcc
                    .borrow_mut()
                    .write_word(bus, address, value)
                    .map_err(MemoryError::Bus)?;
                return Ok(());
            }
            0x10..=0x14 if width == Width::Word => {
                self.caches.flush(address, self.context as u8, asi);
                return Ok(());
            }
            8..=11 | 0x20 => {}
            0x21..=0x2f if self.config.reference_mmu => {}
            _ => {
                return Err(MemoryError::UnsupportedAsi {
                    asi,
                    width,
                    write: true,
                })
            }
        }
        let access = Access {
            address,
            asi,
            write: true,
        };
        let result = self.translate(bus, access).and_then(|translation| {
            let physical = translation.physical;
            self.request(
                bus,
                physical,
                width.store_kind(),
                true,
                translation.cacheable,
            );
            match width {
                Width::Byte => bus.write8(physical, value as u8),
                Width::Half => bus.write16(physical, value as u16),
                Width::Word => bus.write32(physical, value),
            }
            .map_err(MemoryError::Bus)
        });
        self.finish_store(bus, access, width.alignment() as u8, result)
    }

    pub fn load_doubleword(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
    ) -> Result<[u32; 2], MemoryError> {
        self.sync_io_flushes(bus);
        Self::aligned_to(address, 8)?;
        if asi == 0x0f || (self.config.reference_mmu && matches!(asi, 0x0c..=0x0e)) {
            return Ok(self.caches.read_doubleword(address, asi));
        }
        if self.config.reference_mmu && matches!(asi, 5 | 6) {
            let table = if asi == 5 {
                &self.itlb_diag
            } else {
                &self.dtlb_diag
            };
            return Ok([
                *table.get(&address).unwrap_or(&0),
                *table.get(&(address + 4)).unwrap_or(&0),
            ]);
        }
        if matches!(asi, 0x30..=0x32) {
            return Ok([0; 2]);
        }
        if asi == 0x38 {
            let value = self.breakpoints[((address >> 8) & 3) as usize];
            return Ok([(value >> 32) as u32, value as u32]);
        }
        if asi == 2 {
            let value = self.mxcc_read(address);
            return Ok([(value >> 32) as u32, value as u32]);
        }
        self.memory_space(asi, MemoryOperation::LoadDoubleword)?;
        let access = Access {
            address,
            asi,
            write: false,
        };
        // Eight-byte alignment guarantees that both words occupy the same page.
        let result = self.translate(bus, access).and_then(|translation| {
            self.request(
                bus,
                translation.physical,
                MemoryRequestKind::Read,
                true,
                translation.cacheable,
            );
            bus.read_doubleword(translation.physical)
                .map_err(MemoryError::Bus)
        });
        self.finish(access, result)
    }

    pub fn store_doubleword(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
        words: [u32; 2],
    ) -> Result<(), MemoryError> {
        self.sync_io_flushes(bus);
        Self::aligned_to(address, 8)?;
        if asi == 0x0f || (self.config.reference_mmu && matches!(asi, 0x0c..=0x0e)) {
            self.caches.write_doubleword(address, asi, words);
            return Ok(());
        }
        if self.config.reference_mmu && matches!(asi, 5 | 6) {
            let table = if asi == 5 {
                &mut self.itlb_diag
            } else {
                &mut self.dtlb_diag
            };
            table.insert(address, words[0]);
            table.insert(address + 4, words[1]);
            return Ok(());
        }
        if matches!(asi, 0x30..=0x32 | 0x36 | 0x37) {
            return Ok(());
        }
        if asi == 0x38 {
            self.breakpoints[((address >> 8) & 3) as usize] =
                (u64::from(words[0]) << 32) | u64::from(words[1]);
            return Ok(());
        }
        if asi == 2 {
            self.mxcc_write(
                bus,
                address,
                (u64::from(words[0]) << 32) | u64::from(words[1]),
            )?;
            return Ok(());
        }
        self.memory_space(asi, MemoryOperation::StoreDoubleword)?;
        let access = Access {
            address,
            asi,
            write: true,
        };
        let result = self.translate(bus, access).and_then(|translation| {
            self.request(
                bus,
                translation.physical,
                MemoryRequestKind::WriteDoubleword,
                true,
                translation.cacheable,
            );
            bus.write_doubleword(translation.physical, words)
                .map_err(MemoryError::Bus)
        });
        self.finish_store(bus, access, 8, result)
    }

    pub fn swap(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
        value: u32,
    ) -> Result<u32, MemoryError> {
        Self::aligned(address, Width::Word)?;
        self.exchange(bus, address, asi, Some(value))
    }

    pub fn load_store_byte(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
    ) -> Result<u32, MemoryError> {
        self.exchange(bus, address, asi, None)
    }

    fn exchange(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
        value: Option<u32>,
    ) -> Result<u32, MemoryError> {
        self.sync_io_flushes(bus);
        self.memory_space(
            asi,
            if value.is_some() {
                MemoryOperation::Swap
            } else {
                MemoryOperation::LoadStoreByte
            },
        )?;
        // All writable data permissions also permit reading; instruction-space
        // writes require both execute and write. Check before issuing the bus
        // transaction, and classify the access as a store for SFSR/R/M.
        let access = Access {
            address,
            asi,
            write: true,
        };
        let result = self.translate(bus, access).and_then(|translation| {
            let physical = translation.physical;
            self.request(
                bus,
                physical,
                MemoryRequestKind::Read,
                true,
                translation.cacheable,
            );
            match value {
                Some(value) => bus.swap32(physical, value),
                None => bus.ldstub8(physical).map(u32::from),
            }
            .map_err(MemoryError::Bus)
            .map(|old| {
                let kind = if value.is_some() {
                    MemoryRequestKind::WriteWord
                } else {
                    MemoryRequestKind::WriteByte
                };
                self.request(bus, physical, kind, true, translation.cacheable);
                old
            })
        });
        self.finish(access, result)
    }

    fn memory_space(&self, asi: u8, operation: MemoryOperation) -> Result<(), MemoryError> {
        if matches!(asi, 8..=11 | 0x20)
            || (self.config.reference_mmu && (0x21..=0x2f).contains(&asi))
        {
            Ok(())
        } else {
            Err(MemoryError::UnsupportedOperation { asi, operation })
        }
    }

    fn aligned(address: u32, width: Width) -> Result<(), MemoryError> {
        Self::aligned_to(address, width.alignment())
    }

    fn aligned_to(address: u32, alignment: u32) -> Result<(), MemoryError> {
        if address % alignment != 0 {
            Err(MemoryError::Bus(BusError {
                address: u64::from(address),
                kind: BusErrorKind::Misaligned,
            }))
        } else {
            Ok(())
        }
    }

    fn mxcc_read(&self, address: u32) -> u64 {
        self.mxcc.borrow().read_u64(address)
    }

    fn mxcc_write(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        value: u64,
    ) -> Result<(), MemoryError> {
        self.mxcc
            .borrow_mut()
            .write_u64(bus, address, value)
            .map_err(MemoryError::Bus)
    }

    /// Shared state for the CPU-module physical control window.
    pub fn mxcc_handle(&self) -> Rc<RefCell<MxccState>> {
        self.mxcc.clone()
    }

    pub fn set_mxcc(&mut self, mxcc: Rc<RefCell<MxccState>>) {
        self.mxcc = mxcc;
    }

    pub fn reset_mxcc(&mut self) {
        self.mxcc.borrow_mut().reset();
    }

    /// Diagnostic TLB writes install a real translation on the reference
    /// profile. POST's TLB_HIT test writes an entry through ASI 5/6 and then
    /// performs data accesses through it.
    fn install_diag_entry(&mut self, address: u32, instruction: bool) {
        let base = (address >> 12) << 12;
        let high_index = ((address >> 12) & 0x3f) as usize;
        let index = if high_index != 0 {
            high_index
        } else {
            ((address >> 2) & 0x3f) as usize
        };
        let table = if instruction {
            &self.itlb_diag
        } else {
            &self.dtlb_diag
        };
        if (address >> 8) & 7 == 2 {
            if let Some(&pte) = table.get(&(base + 0x200)).or_else(|| table.get(&address)) {
                if pte & 0x20 == 0 {
                    self.tlb[index].upper &= !8;
                    return;
                }
            }
        }
        let (Some(&tag), Some(&context), Some(&pte)) = (
            table.get(&base),
            table.get(&(base + 0x100)),
            table.get(&(base + 0x200)),
        ) else {
            return;
        };
        if pte & 0x20 == 0 {
            self.tlb[index].upper &= !8;
            return;
        }
        let (mask, level_tag, level) = match pte & 3 {
            1 => (0x00ff_ffff, 3, 1),
            2 => (0x0003_ffff, 1, 2),
            3 => (0x0000_0fff, 0, 3),
            _ => return,
        };
        let permissions = [0x120, 0x1b0, 0x360, 0x3f0, 0x240, 0x130, 0x068, 0x078];
        let acc = ((pte >> 2) & 7) as usize;
        let virtual_base = tag & !mask;
        let entry = Entry {
            virtual_base,
            offset_mask: mask,
            context,
            level,
            pte_address: None,
            pte,
            upper: virtual_base | ((context & 0xff) << 4) | 8 | level_tag,
            lower: permissions[acc] | u32::from(pte & 0x40 != 0),
        };
        self.tlb[index] = entry;
    }

    /// Mirror normal TLB fills into the ASI 5/6 diagnostic view so the POST
    /// miss test can read the cached PTE and tag after a table walk.
    fn diag_fill(table: &mut HashMap<u32, u32>, key: u32, value: u32) {
        if table.get(&key).copied().unwrap_or(0) == 0 {
            table.insert(key, value);
        }
    }

    fn record_fill(&mut self, index: usize, access: Access, entry: Entry) {
        if !self.config.reference_mmu || entry.lower & 4 != 0 {
            return;
        }
        let base = (index as u32) << 12;
        let tag = entry.virtual_base;
        let context = entry.context;
        let pte = (entry.pte & !0x40) | 0x20;
        let table = if access.instruction() {
            &mut self.itlb_diag
        } else {
            &mut self.dtlb_diag
        };
        Self::diag_fill(table, base, tag);
        Self::diag_fill(table, base + 0x100, context);
        Self::diag_fill(table, base + 0x200, pte);
    }

    /// POST reads the D-TLB cached PTEs at consecutive low diagnostic offsets,
    /// indexed by the virtual 256 KiB region.
    fn record_diag_view(&mut self, access: Access, entry: Entry) {
        if !self.config.reference_mmu || access.instruction() || entry.lower & 4 != 0 {
            return;
        }
        let index = (access.address >> 18) & 0x3f;
        let low = index * 4;
        let tag = access.address & 0xfffc_0000;
        let context = entry.context;
        let pte = (entry.pte & !0x40) | 0x20;
        let table = &mut self.dtlb_diag;
        Self::diag_fill(table, index << 12, tag);
        Self::diag_fill(table, (index << 12) + 0x100, context);
        Self::diag_fill(table, (index << 12) + 0x200, pte);
        Self::diag_fill(table, low, tag);
        Self::diag_fill(table, 0x100 + low, context);
        Self::diag_fill(table, 0x200 + low, pte);
    }

    /// SuperSPARC II SEL 5/6 expose the cached level-2 PTP and its virtual
    /// tag. POST checks four D-TLB slots after a 16 MiB fill.
    fn read_ptp2(&self, address: u32, instruction: bool) -> Option<u32> {
        match (address >> 8) & 7 {
            5 => {
                let entry = if instruction {
                    self.itlb_ptp2
                } else {
                    self.dtlb_ptp2[((address >> 12) & 3) as usize]
                };
                entry.map(|(_, value)| value)
            }
            6 => {
                let entry = if instruction {
                    self.itlb_ptp2
                } else {
                    self.dtlb_ptp2[((address >> 12) & 3) as usize]
                };
                entry.map(|(tag, _)| tag)
            }
            _ => None,
        }
    }

    fn write_ptp2(&mut self, address: u32, value: u32, instruction: bool) {
        let sel = (address >> 8) & 7;
        if !matches!(sel, 5 | 6) {
            return;
        }
        let slot = ((address >> 12) & 3) as usize;
        if instruction {
            let mut entry = self.itlb_ptp2.unwrap_or((0, 0));
            if sel == 5 {
                entry.1 = value;
            } else {
                entry.0 = value;
            }
            self.itlb_ptp2 = Some(entry);
        } else {
            let mut entry = self.dtlb_ptp2[slot].unwrap_or((0, 0));
            if sel == 5 {
                entry.1 = value;
            } else {
                entry.0 = value;
            }
            self.dtlb_ptp2[slot] = Some(entry);
        }
    }

    /// Record the level-2 PTP for this virtual 256 KiB region. The PTP2 stores
    /// the level-1 descriptor that points at the level-2 table; the tag is the
    /// level-2 virtual address. This is a diagnostic view only.
    fn fill_ptp2(&mut self, bus: &mut impl Bus, access: Access, entry: Entry) {
        if !self.config.reference_mmu || entry.level != 1 {
            return;
        }
        let Some(pte_address) = entry.pte_address else {
            return;
        };
        let l1_base = pte_address - u64::from((access.address >> 24) * 4);
        let index = (access.address >> 18) & 0x3f;
        let Ok(word) = bus.read32(l1_base + u64::from(index * 4)) else {
            return;
        };
        if word & 3 != 1 {
            return;
        }
        let tag = access.address & 0xfffc_0000;
        if access.instruction() {
            self.itlb_ptp2 = Some((tag, word));
            return;
        }
        if let Some(slot) = self
            .dtlb_ptp2
            .iter()
            .position(|entry| entry.is_some_and(|(entry_tag, _)| entry_tag == tag))
        {
            self.dtlb_ptp2[slot] = Some((tag, word));
            return;
        }
        if let Some(slot) = self.dtlb_ptp2.iter().position(|entry| entry.is_none()) {
            self.dtlb_ptp2[slot] = Some((tag, word));
            self.dtlb_ptp2_next = (slot + 1) & 3;
        } else {
            let slot = self.dtlb_ptp2_next;
            self.dtlb_ptp2[slot] = Some((tag, word));
            self.dtlb_ptp2_next = (slot + 1) & 3;
        }
    }

    fn reference_diagnostic_read(&mut self, address: u32, asi: u8) -> u32 {
        match asi {
            0x38 => {
                let index = ((address >> 8) & 3) as usize;
                let value = self.breakpoints[index] as u32;
                if index == 3 {
                    self.breakpoints[3] = 0; // status clears on read
                }
                value
            }
            0x49 => self.counter_value,
            0x4a => self.counter_control,
            0x4b => self.counter_status,
            0x4c => self.breakpoint_action,
            _ => unreachable!("filtered diagnostic ASI"),
        }
    }

    fn reference_diagnostic_write(&mut self, address: u32, asi: u8, value: u64) {
        match asi {
            0x38 => {
                let index = ((address >> 8) & 3) as usize;
                let mask = if index < 2 {
                    0x0000_000f_ffff_ffff
                } else {
                    0x7f
                };
                self.breakpoints[index] = value & mask;
            }
            0x49 => self.counter_value = value as u32,
            0x4a => self.counter_control = value as u32 & 3,
            0x4b => self.counter_status = value as u32 & 3,
            0x4c => self.breakpoint_action = value as u32 & 0x1fff,
            _ => unreachable!("filtered diagnostic ASI"),
        }
    }

    fn read_register(&mut self, address: u32) -> Result<u32, MemoryError> {
        if address >> 13 != 0 {
            return Err(MemoryError::UnsupportedRegister(address));
        }
        Ok(match (address >> 8) & 31 {
            0 => self.control(),
            1 => self.context_table,
            2 => self.context,
            3 => std::mem::take(&mut self.fault_status),
            4 => std::mem::take(&mut self.fault_address),
            0x10 => self.trcr,
            0x13 => self.fault_status,
            0x14 => self.fault_address,
            _ => return Err(MemoryError::UnsupportedRegister(address)),
        })
    }

    fn write_register(&mut self, address: u32, value: u32) -> Result<(), MemoryError> {
        if address >> 13 != 0 {
            return Err(MemoryError::UnsupportedRegister(address));
        }
        match (address >> 8) & 31 {
            0 => {
                // Software table walking and watchpoints require additional IU
                // behavior. Performance controls are retained without timing.
                if value & 0x00c0_0000 != 0 {
                    return Err(MemoryError::UnsupportedControl(value));
                }
                self.control = value & self.config.control_mask;
            }
            1 => {
                let value = value & self.config.context_table_mask;
                self.context_table = value;
            }
            2 => {
                self.context = if self.config.reference_mmu {
                    value
                } else {
                    value & 0xff
                };
            }
            3 | 4 => {}
            0x10 => self.trcr = value & self.config.replacement_mask,
            0x13 => self.fault_status = value & self.config.fault_status_mask,
            0x14 => self.fault_address = value,
            _ => return Err(MemoryError::UnsupportedRegister(address)),
        }
        Ok(())
    }

    pub(crate) fn translate_io(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        write: bool,
        mapping: IoMapping,
    ) -> Result<u64, DmaError> {
        self.sync_io_flushes(bus);
        let pte_address = match mapping {
            IoMapping::Bypass(physical) => return Ok(physical),
            IoMapping::PageTable(physical) => physical,
        };
        let mut hit = None;
        for entry in &self.tlb {
            if entry.upper & 8 != 0 && entry.lower & 4 != 0 {
                if entry.upper & 7 != 0 || entry.lower & 2 != 0 {
                    return Err(DmaError::Unsupported("non-page I/O TLB diagnostic entry"));
                }
                // IOPTE matching ignores CPU context and supervisor state.
                if entry.upper & !0xfff == address & !0xfff {
                    hit = Some(*entry);
                }
            }
        }
        let entry = if let Some(entry) = hit {
            entry
        } else {
            if pte_address >= 0x1000_0000 {
                return Err(DmaError::Unsupported("I/O page table outside DRAM"));
            }
            self.request(bus, pte_address, MemoryRequestKind::Read, false, false);
            let pte = bus.read32(pte_address).map_err(DmaError::Bus)?;
            if pte & 2 == 0 {
                return Err(DmaError::Fault {
                    address,
                    kind: DmaFault::InvalidPte,
                });
            }
            Entry {
                virtual_base: address & !0xfff,
                offset_mask: 0xfff,
                level: 3,
                pte: (pte & 0x07ff_ff04) | 0xe000_0000,
                upper: (address & !0xfff) | 8,
                lower: 4,
                ..Entry::default()
            }
        };
        if write && entry.pte & 4 == 0 {
            return Err(DmaError::Fault {
                address,
                kind: DmaFault::WriteProtected,
            });
        }
        if hit.is_none() {
            self.insert(entry);
        }
        // IOPTEs have neither referenced nor modified bits: never write them back.
        Ok((u64::from(entry.pte & IOPTE_PHYSICAL_MASK) << 4) | u64::from(address & 0xfff))
    }

    fn translate(
        &mut self,
        bus: &mut impl Bus,
        access: Access,
    ) -> Result<Translation, MemoryError> {
        if access.instruction() && self.control & self.config.boot_mode_bit != 0 {
            return Ok(Translation {
                physical: self.config.prom_base | u64::from(access.address & self.config.boot_mask),
                cacheable: false,
            });
        }
        if self.config.reference_mmu && (0x21..=0x2f).contains(&access.asi) {
            let physical = (u64::from(access.asi & 0xf) << 32) | u64::from(access.address);
            return Ok(Translation {
                physical,
                cacheable: false,
            });
        }
        if access.asi == 0x20 || self.control & PCR_EN == 0 {
            return Ok(Translation {
                physical: u64::from(access.address) & self.config.physical_mask,
                cacheable: false,
            });
        }
        self.check_tlb_diagnostic()?;
        let context = self.context;
        let supervisor = access.supervisor();
        let cached = if access.instruction() {
            self.fast_instruction
                .get(access.address, context, supervisor, self.tlb_generation)
        } else {
            self.fast_data
                .get(access.address, context, supervisor, self.tlb_generation)
        };
        if let Some(index) = cached {
            let mut entry = self.tlb[index];
            if entry.matches(
                access.address,
                context,
                supervisor,
                self.config.context_mask,
            ) {
                Self::permission(access, entry)?;
                self.touch(bus, access, &mut entry)?;
                self.tlb[index] = entry;
                self.record_diag_view(access, entry);
                self.fill_ptp2(bus, access, entry);
                return Ok(self.translation(access, entry));
            }
        }
        let hit = self
            .tlb
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.matches(
                    access.address,
                    context,
                    supervisor,
                    self.config.context_mask,
                )
            })
            .min_by_key(|(_, entry)| entry.offset_mask)
            .map(|(index, _)| index);
        let mut entry = if let Some(index) = hit {
            self.tlb[index]
        } else {
            self.walk(bus, access)?
        };
        let ptp2_source = entry;
        if let Some(subregion) = self.reference_subregion(bus, access, entry) {
            entry = subregion;
        }
        Self::permission(access, entry)?;
        self.touch(bus, access, &mut entry)?;
        let index = if let Some(index) = hit {
            self.tlb[index] = entry;
            index
        } else {
            let index = self.insert(entry);
            self.record_fill(index, access, entry);
            index
        };
        self.record_diag_view(access, entry);
        self.fill_ptp2(bus, access, ptp2_source);
        self.cache_translation(access, index);
        Ok(self.translation(access, entry))
    }

    fn cache_translation(&mut self, access: Access, index: usize) {
        let context = self.context;
        let supervisor = access.supervisor();
        let generation = self.tlb_generation;
        if access.instruction() {
            self.fast_instruction
                .put(access.address, context, supervisor, generation, index);
        } else {
            self.fast_data
                .put(access.address, context, supervisor, generation, index);
        }
    }

    fn translation(&self, access: Access, entry: Entry) -> Translation {
        Translation {
            physical: ((u64::from(entry.pte & self.config.pte_physical_mask) << 4)
                & !u64::from(entry.offset_mask))
                | u64::from(access.address & entry.offset_mask),
            cacheable: entry.pte & 0x80 != 0,
        }
    }

    /// The SS-20 POST D-cache PTP test installs 256 KiB subregion descriptors
    /// behind a 16 MiB PROM leaf. A matching PTP2 takes the level-2 path for
    /// that subregion instead of the large-page leaf.
    fn reference_subregion(
        &self,
        bus: &mut impl Bus,
        access: Access,
        entry: Entry,
    ) -> Option<Entry> {
        if !self.config.reference_mmu {
            return None;
        }
        let tag = access.address & 0xfffc_0000;
        let cached_ptp = if access.instruction() {
            self.itlb_ptp2
                .filter(|(cached, _)| *cached == tag)
                .map(|(_, value)| value)
        } else {
            self.dtlb_ptp2
                .iter()
                .flatten()
                .find(|(cached, _)| *cached == tag)
                .map(|(_, value)| *value)
        };
        let ptp = if let Some(ptp) = cached_ptp {
            ptp
        } else {
            if entry.level != 1 || entry.pte & 3 != 2 {
                return None;
            }
            let leaf_physical = u64::from(entry.pte & self.config.pte_physical_mask) << 4;
            if leaf_physical < self.config.prom_base {
                return None;
            }
            let pte_address = entry.pte_address?;
            let l1_base = pte_address - u64::from((access.address >> 24) * 4);
            let l1_address = l1_base + u64::from(((access.address >> 18) & 0x3f) * 4);
            let l1_word = bus.read32(l1_address).ok()?;
            if l1_word & 3 == 2 {
                let subregion = self.entry(access.address, 2, l1_address, l1_word);
                let physical = u64::from(subregion.pte & self.config.pte_physical_mask) << 4;
                return (physical < self.config.prom_base).then_some(subregion);
            }
            if l1_word & 3 != 1 {
                return None;
            }
            l1_word
        };
        let l2_base = u64::from(ptp & self.config.pte_table_mask) << 4;
        let l2_address = l2_base + u64::from(((access.address >> 12) & 0x3f) * 4);
        let l2_word = bus.read32(l2_address).ok()?;
        if l2_word & 3 != 2 {
            return None;
        }
        let subregion = self.entry(access.address, 2, l2_address, l2_word);
        let physical = u64::from(subregion.pte & self.config.pte_physical_mask) << 4;
        (physical < self.config.prom_base).then_some(subregion)
    }

    fn permission(access: Access, entry: Entry) -> Result<(), MemoryError> {
        let supervisor = access.supervisor();
        if !supervisor && entry.lower & 8 != 0 {
            return Err(MemoryError::Translation(
                access.fault(FaultType::Privilege, entry.level),
            ));
        }
        let rights = entry.lower >> if supervisor { 4 } else { 7 };
        let executable = rights & 4 != 0;
        let writable = rights & 1 != 0;
        let readable = rights & 2 != 0;
        let allowed = if access.instruction() {
            executable
        } else if access.write {
            true
        } else {
            readable
        };
        if !allowed || (access.write && !writable) {
            return Err(MemoryError::Translation(
                access.fault(FaultType::Protection, entry.level),
            ));
        }
        Ok(())
    }

    fn touch(
        &self,
        bus: &mut impl Bus,
        access: Access,
        entry: &mut Entry,
    ) -> Result<(), MemoryError> {
        if entry.pte_address.is_none() {
            if self.config.reference_mmu && access.write {
                entry.lower |= 1;
                entry.pte |= 0x40;
                return Ok(());
            }
            return if access.write && entry.lower & 1 == 0 {
                Err(MemoryError::UnsupportedTlbModifiedUpdate)
            } else {
                Ok(())
            };
        }
        let bits = REFERENCED
            | if access.write && entry.lower & 1 == 0 {
                MODIFIED
            } else {
                0
            };
        let missing = bits & !entry.pte;
        if missing != 0 || (access.write && entry.lower & 1 == 0) {
            let address = entry
                .pte_address
                .ok_or(MemoryError::UnsupportedTlbModifiedUpdate)?;
            self.request(bus, address, MemoryRequestKind::WriteWord, true, false);
            bus.write32_masked(address, bits, bits)
                .map_err(|error| Self::table_error(access, entry.level, error))?;
            entry.pte |= bits;
            if access.write {
                entry.lower |= 1;
            }
        }
        Ok(())
    }

    fn root(&self) -> u64 {
        (u64::from(self.context_table) << 4)
            | (u64::from(self.context & self.config.context_mask) << 2)
    }

    fn table_error(access: Access, level: u8, error: BusError) -> MemoryError {
        if error.kind.is_host_stop() {
            MemoryError::Bus(error)
        } else {
            MemoryError::Translation(Self::table_fault(access, level, error))
        }
    }

    fn table_fault(access: Access, level: u8, error: BusError) -> MmuFault {
        MmuFault {
            bus_error: Some(error.kind),
            ..access.fault(FaultType::Translation, level)
        }
    }

    fn next_table(&self, word: u32, address: u32, level: u8) -> u64 {
        let index = match level {
            0 => address >> 24,
            1 => (address >> 18) & 63,
            2 => (address >> 12) & 63,
            _ => unreachable!("no table beyond level 3"),
        };
        (u64::from(word & self.config.pte_table_mask) << 4) | u64::from(index << 2)
    }

    fn entry(&self, address: u32, level: u8, pte_address: u64, pte: u32) -> Entry {
        let mask = match level {
            0 => u32::MAX,
            1 => 0x00ff_ffff,
            2 => 0x0003_ffff,
            3 => 0xfff,
            _ => unreachable!(),
        };
        let acc = ((pte >> 2) & 7) as usize;
        // UE/UR/UW/SE/SR/SW at bits9:4, supervisor-global at bit3, M at bit0.
        let permissions = [0x120, 0x1b0, 0x360, 0x3f0, 0x240, 0x130, 0x068, 0x078];
        let level_tag = match level {
            1 => 3,
            2 => 1,
            3 => 0,
            _ => 7,
        };
        Entry {
            virtual_base: address & !mask,
            offset_mask: mask,
            context: self.context,
            level,
            pte_address: Some(pte_address),
            pte: if self.config.reference_mmu {
                pte
            } else {
                (pte & 0x07ff_ffff) | ((!level_tag & 7) << 29)
            },
            upper: (address & !0xfff) | ((self.context & 0xff) << 4) | 8 | level_tag,
            lower: permissions[acc] | u32::from(pte & MODIFIED != 0),
        }
    }

    fn walk(&self, bus: &mut impl Bus, access: Access) -> Result<Entry, MemoryError> {
        let mut physical = self.root();
        for level in 0..=3 {
            self.request(bus, physical, MemoryRequestKind::Read, true, false);
            let word = bus
                .read32(physical)
                .map_err(|error| Self::table_error(access, level, error))?;
            match word & 3 {
                1 if level < 3 => physical = self.next_table(word, access.address, level),
                2 if level == 0 => return Err(MemoryError::UnsupportedRootPte),
                2 => return Ok(self.entry(access.address, level, physical, word)),
                et => {
                    return Err(MemoryError::Translation(access.fault(
                        if et == 0 {
                            FaultType::Invalid
                        } else {
                            FaultType::Translation
                        },
                        level,
                    )))
                }
            }
        }
        unreachable!("level 3 terminates the walk")
    }

    fn probe(&mut self, bus: &mut impl Bus, address: u32) -> Result<u32, MemoryError> {
        let kind = (address >> 8) & 15;
        if kind > 4 {
            return Ok(0);
        }
        let access = Access {
            address: address & !0xfff,
            asi: 3,
            write: false,
        };
        if kind == 4 {
            self.check_tlb_diagnostic()?;
            if let Some(entry) = self.tlb.iter().find(|entry| {
                entry.matches(access.address, self.context, true, self.config.context_mask)
            }) {
                return Ok((entry.pte & (self.config.pte_physical_mask | 0xdc)) | 0x22);
            }
        } else {
            // Conservative invalidation also removes any cached intermediate
            // descriptors if these are added in a later implementation.
            self.flush();
        }
        let mut physical = self.root();
        for level in 0..=3 {
            self.request(bus, physical, MemoryRequestKind::Read, true, false);
            let word = match bus.read32(physical) {
                Ok(word) => word,
                Err(error) => {
                    if error.kind.is_host_stop() {
                        return Err(MemoryError::Bus(error));
                    }
                    self.record(Self::table_fault(access, level, error));
                    return Ok(0);
                }
            };
            let et = word & 3;
            if kind < 4 && level == 3 - kind as u8 {
                return Ok(if et == 3 || (level == 3 && et == 1) {
                    0
                } else {
                    word
                });
            }
            match et {
                1 if level < 3 => physical = self.next_table(word, access.address, level),
                2 if self.config.reference_mmu && kind < 4 && level > 0 => return Ok(word),
                2 if kind == 4 => {
                    let mut entry = self.entry(access.address, level, physical, word);
                    match self.touch(bus, access, &mut entry) {
                        Err(MemoryError::Translation(fault)) => {
                            self.record(fault);
                            return Ok(0);
                        }
                        Err(error) => return Err(error),
                        Ok(()) => {}
                    }
                    if level != 0 {
                        self.insert(entry);
                    }
                    return Ok(entry.pte & (self.config.pte_physical_mask | 0xff));
                }
                _ => return Ok(0),
            }
        }
        Ok(0)
    }
}
