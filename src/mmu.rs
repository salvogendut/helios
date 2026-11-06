//! microSPARC-II SRMMU. See docs/memory.md for the supported control spaces,
//! implementation choices, and differences from generic SPARC Reference MMUs.

use crate::bus::{Bus, BusError, BusErrorKind};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    virtual_base: u32,
    offset_mask: u32,
    context: u8,
    level: u8,
    pte_address: u64,
    pte: u32,
}

impl Entry {
    fn matches(self, address: u32, context: u8, supervisor: bool) -> bool {
        let global = (self.pte >> 2) & 7 >= 6;
        address & !self.offset_mask == self.virtual_base
            && (context == self.context || (supervisor && global))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srmmu {
    control: u32,
    context_table: u32,
    context: u8,
    fault_status: u32,
    fault_address: u32,
    tlb: [Option<Entry>; TLB_ENTRIES],
    replacement: usize,
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
            tlb: [None; TLB_ENTRIES],
            replacement: 0,
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

    fn flush(&mut self) {
        self.tlb.fill(None);
        self.replacement = 0;
    }

    fn insert(&mut self, entry: Entry) {
        self.tlb[self.replacement] = Some(entry);
        self.replacement = (self.replacement + 1) % TLB_ENTRIES;
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
            Err(MemoryError::Bus(error)) if error.kind != BusErrorKind::Misaligned => {
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
            3 if width == Width::Word => return Ok(self.probe(bus, address)),
            4 if width == Width::Word => return self.read_register(address),
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
            // Coherent functional caches have no dirty lines to write back.
            0x10..=0x14 if width == Width::Word => return Ok(()),
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

    fn aligned(address: u32, width: Width) -> Result<(), MemoryError> {
        if address % width.alignment() != 0 {
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
        let hit = self.tlb.iter().position(|slot| {
            slot.is_some_and(|entry| {
                entry.matches(access.address, self.context, access.supervisor())
            })
        });
        let mut entry = if let Some(index) = hit {
            self.tlb[index].expect("matched TLB entry")
        } else {
            self.walk(bus, access)?
        };
        Self::permission(access, entry)?;
        Self::touch(bus, access, &mut entry)?;
        if let Some(index) = hit {
            self.tlb[index] = Some(entry);
        } else {
            self.insert(entry);
        }
        Ok(
            ((u64::from(entry.pte & 0x07ff_ff00) << 4) & !u64::from(entry.offset_mask))
                | u64::from(access.address & entry.offset_mask),
        )
    }

    fn permission(access: Access, entry: Entry) -> Result<(), MemoryError> {
        let acc = (entry.pte >> 2) & 7;
        let supervisor = access.supervisor();
        if !supervisor && acc >= 6 {
            return Err(MemoryError::Translation(
                access.fault(FaultType::Privilege, entry.level),
            ));
        }
        let executable = matches!(acc, 2 | 3 | 4 | 6 | 7);
        let writable = matches!(acc, 1 | 3 | 7) || (supervisor && acc == 5);
        let readable = acc != 4;
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
        let bits = REFERENCED | if access.write { MODIFIED } else { 0 };
        let missing = bits & !entry.pte;
        if missing != 0 {
            bus.write32_masked(entry.pte_address, missing, missing)
                .map_err(|_| {
                    MemoryError::Translation(access.fault(FaultType::Translation, entry.level))
                })?;
            entry.pte |= missing;
        }
        Ok(())
    }

    fn root(&self) -> u64 {
        (u64::from(self.context_table) << 4) | (u64::from(self.context) << 2)
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
        Entry {
            virtual_base: address & !mask,
            offset_mask: mask,
            context: self.context,
            level,
            pte_address,
            pte,
        }
    }

    fn walk(&self, bus: &mut impl Bus, access: Access) -> Result<Entry, MemoryError> {
        let mut physical = self.root();
        for level in 0..=3 {
            let word = bus.read32(physical).map_err(|_| {
                MemoryError::Translation(access.fault(FaultType::Translation, level))
            })?;
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

    fn probe(&mut self, bus: &mut impl Bus, address: u32) -> u32 {
        let kind = (address >> 8) & 15;
        if kind > 4 {
            return 0;
        }
        let access = Access {
            address: address & !0xfff,
            asi: 3,
            write: false,
        };
        if kind == 4 {
            if let Some(entry) = self
                .tlb
                .iter()
                .flatten()
                .find(|entry| entry.matches(access.address, self.context, true))
            {
                return entry.pte;
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
                Err(_) => {
                    self.record(access.fault(FaultType::Translation, level));
                    return 0;
                }
            };
            let et = word & 3;
            if kind < 4 && level == 3 - kind as u8 {
                return if et == 3 || (level == 3 && et == 1) {
                    0
                } else {
                    word
                };
            }
            match et {
                1 if level < 3 => physical = Self::next_table(word, access.address, level),
                2 if kind == 4 => {
                    let mut entry = self.entry(access.address, level, physical, word);
                    if let Err(MemoryError::Translation(fault)) =
                        Self::touch(bus, access, &mut entry)
                    {
                        self.record(fault);
                        return 0;
                    }
                    if level != 0 {
                        self.insert(entry);
                    }
                    return entry.pte;
                }
                _ => return 0,
            }
        }
        0
    }
}
