//! microSPARC-II SRMMU. See docs/memory.md for the supported control spaces,
//! implementation choices, and differences from generic SPARC Reference MMUs.

use crate::bus::{Bus, BusError, BusErrorKind};
use crate::cache::CacheDiagnostics;

pub const PCR_EN: u32 = 1;
pub const PCR_NF: u32 = 2;
pub const PCR_BM: u32 = 1 << 14;
pub const PROM_BASE: u64 = 0x7000_0000;
const PCR_VERSION: u32 = 0x0400_0000;
const PHYSICAL_MASK: u64 = 0x7fff_ffff;
const REFERENCED: u32 = 1 << 5;
const MODIFIED: u32 = 1 << 6;
const TLB_ENTRIES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
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
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Entry {
    virtual_base: u32,
    offset_mask: u32,
    context: u8,
    level: u8,
    pte_address: Option<u64>,
    pte: u32,
    upper: u32,
    lower: u32,
}

impl Entry {
    fn matches(self, address: u32, context: u8, supervisor: bool) -> bool {
        let global = self.lower & 8 != 0;
        self.upper & 0xc == 8
            && self.lower & 6 == 0
            && address & !self.offset_mask == self.virtual_base
            && (context == self.context || (supervisor && global))
    }

    fn retag(&mut self, upper: u32) {
        self.upper = upper;
        self.context = (upper >> 4) as u8;
        (self.level, self.offset_mask) = match upper & 7 {
            3 => (1, 0x00ff_ffff),
            1 => (2, 0x0003_ffff),
            0 => (3, 0xfff),
            _ => (0, u32::MAX),
        };
        self.virtual_base = upper & !self.offset_mask;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srmmu {
    control: u32,
    context_table: u32,
    context: u8,
    fault_status: u32,
    fault_address: u32,
    tlb: [Entry; TLB_ENTRIES],
    trcr: u32,
    caches: CacheDiagnostics,
}

impl Default for Srmmu {
    /// Translation and boot mode disabled, for the bare-metal harness.
    fn default() -> Self {
        Self {
            control: 0,
            context_table: 0,
            context: 0,
            fault_status: 0,
            fault_address: 0,
            tlb: [Entry::default(); TLB_ENTRIES],
            // Functional reset choice: all 64 entries initially replaceable.
            trcr: 63 << 7,
            caches: CacheDiagnostics::default(),
        }
    }
}

impl Srmmu {
    pub fn reset() -> Self {
        Self {
            control: PCR_BM,
            ..Self::default()
        }
    }

    pub fn control(&self) -> u32 {
        PCR_VERSION | self.control
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

    fn flush(&mut self) {
        for entry in &mut self.tlb {
            entry.upper &= !8;
        }
    }

    fn insert(&mut self, entry: Entry) {
        let wrap = ((self.trcr >> 7) & 63) as usize;
        let limit = if self.trcr & (1 << 13) != 0 {
            wrap.min(47)
        } else {
            wrap
        };
        let counter = (self.trcr & 63) as usize;
        let mut index = counter % (limit + 1);
        // Locked PTP slots have alternate CPU PTE locations 3, 4 and 5.
        if index < 3 && self.trcr & (1 << (14 + index)) != 0 {
            index += 3;
        }
        self.tlb[index] = entry;
        // Functional scheduling: advance on fills, not on individual CPU clocks.
        if self.trcr & (1 << 6) == 0 {
            self.trcr = (self.trcr & !63) | ((counter + 1) % (limit + 1)) as u32;
        }
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
        Ok(())
    }

    fn validate_tlb_lookup(&self) -> Result<(), MemoryError> {
        for (index, entry) in self.tlb.iter().enumerate() {
            if entry.upper & 8 != 0 && (entry.lower & 2 != 0 || entry.upper & 7 == 2) {
                return Err(MemoryError::UnsupportedTlbDiagnostic {
                    address: 0x300 | (index as u32 * 4),
                    value: entry.upper,
                });
            }
        }
        Ok(())
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
        self.fault_status = (u32::from(fault.level) << 8)
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
                if matches!(error.kind, BusErrorKind::Unmapped | BusErrorKind::ReadOnly) =>
            {
                access.fault(FaultType::AccessBus, 0)
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
        Self::aligned(address, width)?;
        match asi {
            3 if width == Width::Word => return self.probe(bus, address),
            4 if width == Width::Word => return self.read_register(address),
            6 if width == Width::Word => return self.diagnostic_read(address),
            0x0c..=0x0e if width == Width::Word => {
                return Ok(self.caches.read(address, asi, width));
            }
            0x0f => return Ok(self.caches.read(address, asi, width)),
            8..=11 | 0x20 => {}
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
        let result = self.translate(bus, access).and_then(|physical| {
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
        Self::aligned(address, width)?;
        match asi {
            3 if width == Width::Word => {
                if (address >> 8) & 15 <= 4 {
                    self.flush();
                }
                return Ok(());
            }
            4 if width == Width::Word => return self.write_register(address, value),
            6 if width == Width::Word => return self.diagnostic_write(address, value),
            0x0c..=0x0e if width == Width::Word => {
                self.caches.write(address, asi, width, value);
                return Ok(());
            }
            0x0f => {
                self.caches.write(address, asi, width, value);
                return Ok(());
            }
            0x10..=0x14 if width == Width::Word => {
                self.caches.flush(address, self.context, asi);
                return Ok(());
            }
            8..=11 | 0x20 => {}
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
        let result = self.translate(bus, access).and_then(|physical| {
            match width {
                Width::Byte => bus.write8(physical, value as u8),
                Width::Half => bus.write16(physical, value as u16),
                Width::Word => bus.write32(physical, value),
            }
            .map_err(MemoryError::Bus)
        });
        self.finish(access, result)
    }

    pub fn load_doubleword(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
    ) -> Result<[u32; 2], MemoryError> {
        Self::aligned_to(address, 8)?;
        if asi == 0x0f {
            return Ok(self.caches.read_doubleword(address));
        }
        Self::memory_space(asi, MemoryOperation::LoadDoubleword)?;
        let access = Access {
            address,
            asi,
            write: false,
        };
        // Eight-byte alignment guarantees that both words occupy the same page.
        let result = self
            .translate(bus, access)
            .and_then(|physical| bus.read_doubleword(physical).map_err(MemoryError::Bus));
        self.finish(access, result)
    }

    pub fn store_doubleword(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        asi: u8,
        words: [u32; 2],
    ) -> Result<(), MemoryError> {
        Self::aligned_to(address, 8)?;
        if asi == 0x0f {
            self.caches.write_doubleword(address, words);
            return Ok(());
        }
        Self::memory_space(asi, MemoryOperation::StoreDoubleword)?;
        let access = Access {
            address,
            asi,
            write: true,
        };
        let result = self.translate(bus, access).and_then(|physical| {
            bus.write_doubleword(physical, words)
                .map_err(MemoryError::Bus)
        });
        self.finish(access, result)
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
        Self::memory_space(
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
        let result = self.translate(bus, access).and_then(|physical| {
            match value {
                Some(value) => bus.swap32(physical, value),
                None => bus.ldstub8(physical).map(u32::from),
            }
            .map_err(MemoryError::Bus)
        });
        self.finish(access, result)
    }

    fn memory_space(asi: u8, operation: MemoryOperation) -> Result<(), MemoryError> {
        if matches!(asi, 8..=11 | 0x20) {
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

    fn read_register(&mut self, address: u32) -> Result<u32, MemoryError> {
        if address >> 13 != 0 {
            return Err(MemoryError::UnsupportedRegister(address));
        }
        Ok(match (address >> 8) & 31 {
            0 => self.control(),
            1 => self.context_table,
            2 => u32::from(self.context),
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
                self.control = value & 0x003e_ff03;
                self.flush();
            }
            1 => {
                self.context_table = value & 0x00ff_ffc0;
                self.flush();
            }
            2 => self.context = value as u8,
            3 | 4 => {}
            0x10 => self.trcr = value & 0x01f1_ffff,
            0x13 => self.fault_status = value & 0x0001_6fff,
            0x14 => self.fault_address = value,
            _ => return Err(MemoryError::UnsupportedRegister(address)),
        }
        Ok(())
    }

    fn translate(&mut self, bus: &mut impl Bus, access: Access) -> Result<u64, MemoryError> {
        if access.instruction() && self.control & PCR_BM != 0 {
            return Ok(PROM_BASE | u64::from(access.address & 0x0fff_ffff));
        }
        if access.asi == 0x20 || self.control & PCR_EN == 0 {
            return Ok(u64::from(access.address) & PHYSICAL_MASK);
        }
        self.validate_tlb_lookup()?;
        let hit = self
            .tlb
            .iter()
            .position(|entry| entry.matches(access.address, self.context, access.supervisor()));
        let mut entry = if let Some(index) = hit {
            self.tlb[index]
        } else {
            self.walk(bus, access)?
        };
        Self::permission(access, entry)?;
        Self::touch(bus, access, &mut entry)?;
        if let Some(index) = hit {
            self.tlb[index] = entry;
        } else {
            self.insert(entry);
        }
        Ok(
            ((u64::from(entry.pte & 0x07ff_ff00) << 4) & !u64::from(entry.offset_mask))
                | u64::from(access.address & entry.offset_mask),
        )
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

    fn touch(bus: &mut impl Bus, access: Access, entry: &mut Entry) -> Result<(), MemoryError> {
        if entry.pte_address.is_none() {
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
        (u64::from(self.context_table) << 4) | (u64::from(self.context) << 2)
    }

    fn table_error(access: Access, level: u8, error: BusError) -> MemoryError {
        if error.kind.is_host_stop() {
            MemoryError::Bus(error)
        } else {
            MemoryError::Translation(access.fault(FaultType::Translation, level))
        }
    }

    fn next_table(word: u32, address: u32, level: u8) -> u64 {
        let index = match level {
            0 => address >> 24,
            1 => (address >> 18) & 63,
            2 => (address >> 12) & 63,
            _ => unreachable!("no table beyond level 3"),
        };
        (u64::from(word & 0x07ff_fff0) << 4) | u64::from(index << 2)
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
            pte: (pte & 0x07ff_ffff) | ((!level_tag & 7) << 29),
            upper: (address & !0xfff) | (u32::from(self.context) << 4) | 8 | level_tag,
            lower: permissions[acc] | u32::from(pte & MODIFIED != 0),
        }
    }

    fn walk(&self, bus: &mut impl Bus, access: Access) -> Result<Entry, MemoryError> {
        let mut physical = self.root();
        for level in 0..=3 {
            let word = bus
                .read32(physical)
                .map_err(|error| Self::table_error(access, level, error))?;
            match word & 3 {
                1 if level < 3 => physical = Self::next_table(word, access.address, level),
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
            self.validate_tlb_lookup()?;
            if let Some(entry) = self
                .tlb
                .iter()
                .find(|entry| entry.matches(access.address, self.context, true))
            {
                return Ok((entry.pte & 0x07ff_ffdc) | 0x22);
            }
        } else {
            // Conservative invalidation also removes any cached intermediate
            // descriptors if these are added in a later implementation.
            self.flush();
        }
        let mut physical = self.root();
        for level in 0..=3 {
            let word = match bus.read32(physical) {
                Ok(word) => word,
                Err(error) => {
                    if error.kind.is_host_stop() {
                        return Err(MemoryError::Bus(error));
                    }
                    self.record(access.fault(FaultType::Translation, level));
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
                1 if level < 3 => physical = Self::next_table(word, access.address, level),
                2 if kind == 4 => {
                    let mut entry = self.entry(access.address, level, physical, word);
                    match Self::touch(bus, access, &mut entry) {
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
                    return Ok(entry.pte & 0x07ff_ffff);
                }
                _ => return Ok(0),
            }
        }
        Ok(0)
    }
}
