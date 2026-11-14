//! Per-processor NCR89C105 interrupt controller. See docs/interrupts-timers.md.
use crate::bus::BusErrorKind;
use std::collections::BTreeMap;

pub const MASK_ALL: u32 = 1 << 31;
pub const MASK_BITS: u32 = 0xf05d_ff80;
const SOFT_BITS: u32 = 0xfffe_0000;
pub const MODULE_ERROR: u32 = 1 << 30;
// Sun-4M processor numbers occupy the two-bit TARGET field (§5.7.2).
const MAX_HARDWARE_PROCESSOR: u8 = 3;

/// Target-register policy selected by the machine profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetPolicy {
    /// NCR89C105 uniprocessor: the reserved target address reads zero and
    /// ignores writes.
    Ncr,
    /// Sun-4M MBus controller: TARGET occupies bits 1:0.
    Mbus,
}

/// Level-sensitive inputs supported by the current sun4m wiring contract.
/// Module errors use a separate latch, acknowledged by processor IC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Source {
    SBus1 = 7,
    SBus2 = 8,
    SBus3 = 9,
    SBus4 = 10,
    SBus5 = 11,
    SBus6 = 12,
    SBus7 = 13,
    Keyboard = 14,
    Serial = 15,
    Ethernet = 16,
    Scsi = 18,
    SystemTimer = 19,
    Video = 20,
    Floppy = 22,
    ModuleError = 30,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Bank {
    soft: u32,
    cpu_timer: bool,
    level15: bool,
    levels: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterruptController {
    sources: u32,
    mask: u32,
    target: u8,
    policy: TargetPolicy,
    banks: BTreeMap<u8, Bank>,
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::with_policy(TargetPolicy::Ncr)
    }
}

impl InterruptController {
    /// Build a controller with the target-register policy of the selected
    /// machine profile.
    pub fn with_policy(policy: TargetPolicy) -> Self {
        let mut controller = Self {
            sources: 0,
            mask: MASK_BITS,
            target: 0,
            policy,
            banks: BTreeMap::new(),
        };
        controller.install_processor(0);
        controller
    }

    pub fn policy(&self) -> TargetPolicy {
        self.policy
    }

    fn target_mask(&self) -> u8 {
        match self.policy {
            TargetPolicy::Ncr => 0,
            TargetPolicy::Mbus => 0x03,
        }
    }

    fn is_mbus(&self) -> bool {
        self.policy == TargetPolicy::Mbus
    }

    /// Install the CPU-local pending bank for a hardware processor number.
    /// The number is not a dense module-vector index: slot 1 uses MID 10 and
    /// hardware processor 2.
    pub fn install_processor(&mut self, processor: u8) -> bool {
        if processor > MAX_HARDWARE_PROCESSOR || self.policy == TargetPolicy::Ncr && processor != 0
        {
            return false;
        }
        let inserted = if let std::collections::btree_map::Entry::Vacant(entry) =
            self.banks.entry(processor)
        {
            entry.insert(Bank::default());
            true
        } else {
            false
        };
        self.refresh();
        inserted
    }

    pub fn processors(&self) -> impl Iterator<Item = u8> + '_ {
        self.banks.keys().copied()
    }

    pub fn contains_processor(&self, processor: u8) -> bool {
        self.banks.contains_key(&processor)
    }

    pub(crate) fn latch_module_error(&mut self) {
        // The internal producer reports an event pulse. On the MBus controller
        // the live source status is released after capture; the per-processor
        // latches retain the event until each CPU acknowledges it. An already
        // held external ModuleError input stays asserted.
        let externally_held = self.sources & MODULE_ERROR != 0;
        self.set_module_error(true);
        if self.is_mbus() && !externally_held {
            self.sources &= !MODULE_ERROR;
            self.refresh();
        }
    }

    fn set_module_error(&mut self, asserted: bool) {
        if asserted {
            if self.sources & MODULE_ERROR == 0 {
                self.sources |= MODULE_ERROR;
                let capture = !self.is_mbus() || self.mask & MODULE_ERROR == 0;
                if capture {
                    for bank in self.banks.values_mut() {
                        bank.level15 = true;
                    }
                }
            }
        } else {
            self.sources &= !MODULE_ERROR;
            if !self.is_mbus() {
                for bank in self.banks.values_mut() {
                    bank.level15 = false;
                }
            }
        }
        self.refresh();
    }

    fn clear_level15(&mut self) {
        self.sources &= !MODULE_ERROR;
        for bank in self.banks.values_mut() {
            bank.level15 = false;
        }
    }

    fn acknowledge_level15(&mut self, processor: u8) {
        if let Some(bank) = self.banks.get_mut(&processor) {
            bank.level15 = false;
        }
        if !self.is_mbus() && !self.banks.values().any(|bank| bank.level15) {
            self.sources &= !MODULE_ERROR;
        }
    }

    pub fn reset(&mut self) {
        self.sources = 0;
        self.mask = MASK_BITS;
        self.target = 0;
        for bank in self.banks.values_mut() {
            *bank = Bank::default();
        }
        self.refresh();
    }

    pub fn set_source(&mut self, source: Source, asserted: bool) {
        if source == Source::ModuleError {
            self.set_module_error(asserted);
            return;
        }
        let bit = 1u32 << source as u8;
        let sources = (self.sources & !bit) | if asserted { bit } else { 0 };
        if sources != self.sources {
            self.sources = sources;
            self.refresh();
        }
    }

    pub fn set_cpu_timer(&mut self, asserted: bool) {
        self.set_processor_timer(0, asserted);
    }

    pub fn set_processor_timer(&mut self, processor: u8, asserted: bool) {
        if let Some(bank) = self.banks.get_mut(&processor) {
            if asserted != bank.cpu_timer {
                bank.cpu_timer = asserted;
                self.refresh();
            }
        }
    }

    pub fn target(&self) -> u8 {
        if self.policy == TargetPolicy::Ncr {
            0
        } else {
            self.target
        }
    }

    pub fn set_target(&mut self, processor: u8) {
        if self.policy == TargetPolicy::Ncr {
            return;
        }
        let target = processor & self.target_mask();
        if target != self.target {
            self.target = target;
            self.refresh();
        }
    }

    fn refresh(&mut self) {
        let mask_all = self.mask & MASK_ALL != 0;
        let hard = Self::hardware_levels(self.sources & !self.mask & !MODULE_ERROR);
        let level15_enabled = self.is_mbus() || self.mask & MODULE_ERROR == 0;
        let target = self.target;
        for (&processor, bank) in &mut self.banks {
            let mut levels = 0;
            if !mask_all {
                if processor == target {
                    levels |= hard;
                }
                if bank.cpu_timer {
                    levels |= 1 << 14;
                }
                if bank.level15 && level15_enabled {
                    levels |= 1 << 15;
                }
                levels |= (bank.soft >> 16) as u16;
            }
            bank.levels = levels;
        }
    }

    fn hardware_levels(sources: u32) -> u16 {
        // NCR89C105 table 6-24. Independent source bits can share a CPU level.
        let mut levels = 0;
        for (bit, level) in [
            (7, 2),
            (8, 3),
            (9, 5),
            (10, 7),
            (11, 9),
            (12, 11),
            (13, 13),
            (14, 12),
            (15, 12),
            (16, 6),
            (18, 4),
            (19, 10),
            (20, 8),
            (22, 11),
            (30, 15),
        ] {
            if sources & (1 << bit) != 0 {
                levels |= 1 << level;
            }
        }
        levels
    }

    /// Pending display is independent of masks. Mask All gates delivery,
    /// including software requests (NCR's documented "all interrupts" rule).
    pub fn levels(&self) -> u16 {
        self.levels_for(0)
    }

    pub fn levels_for(&self, processor: u8) -> u16 {
        self.banks.get(&processor).map_or(0, |bank| bank.levels)
    }

    pub fn read(&self, system: bool, offset: u64) -> Result<u32, BusErrorKind> {
        if system {
            self.read_system(offset)
        } else {
            self.read_cpu(0, offset)
        }
    }

    pub fn read_system(&self, offset: u64) -> Result<u32, BusErrorKind> {
        match offset {
            0 => Ok(self.sources),
            4 => Ok(self.mask),
            8 | 12 => Ok(0),
            16 if self.policy == TargetPolicy::Ncr => Ok(0),
            16 => Ok(u32::from(self.target & self.target_mask())),
            _ => Err(BusErrorKind::Unsupported("interrupt register offset")),
        }
    }

    pub fn read_cpu(&self, processor: u8, offset: u64) -> Result<u32, BusErrorKind> {
        let Some(bank) = self.banks.get(&processor) else {
            return match offset {
                0..=12 if offset & 3 == 0 => Ok(0),
                _ => Err(BusErrorKind::Unsupported("interrupt register offset")),
            };
        };
        match offset {
            0 => {
                let mut pending = bank.soft;
                if processor == self.target {
                    pending |= u32::from(Self::hardware_levels(self.sources & !MODULE_ERROR));
                }
                if bank.cpu_timer {
                    pending |= 1 << 14;
                }
                if bank.level15 {
                    pending |= 1 << 15;
                }
                Ok(pending)
            }
            4 | 8 | 12 => Ok(0),
            _ => Err(BusErrorKind::Unsupported("interrupt register offset")),
        }
    }

    pub fn write(&mut self, system: bool, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        if system {
            self.write_system(offset, value)
        } else {
            self.write_cpu(0, offset, value)
        }
    }

    pub fn write_system(&mut self, offset: u64, value: u32) -> Result<(), BusErrorKind> {
        match offset {
            8 => {
                self.mask &= !(value & MASK_BITS);
                self.refresh();
            }
            12 => {
                self.mask |= value & MASK_BITS;
                if value & MODULE_ERROR != 0 && !self.is_mbus() {
                    self.clear_level15();
                }
                self.refresh();
            }
            16 if self.policy == TargetPolicy::Ncr => {}
            16 => self.set_target((value & 0x03) as u8),
            0 | 4 => {} // read-only
            _ => return Err(BusErrorKind::Unsupported("interrupt register offset")),
        }
        Ok(())
    }

    pub fn write_cpu(
        &mut self,
        processor: u8,
        offset: u64,
        value: u32,
    ) -> Result<(), BusErrorKind> {
        let mut acknowledge = false;
        if let Some(bank) = self.banks.get_mut(&processor) {
            match offset {
                4 => {
                    bank.soft &= !(value & SOFT_BITS);
                    acknowledge = value & (1 << 15) != 0;
                }
                8 => bank.soft |= value & SOFT_BITS,
                0 | 12 => {}
                _ => return Err(BusErrorKind::Unsupported("interrupt register offset")),
            }
        } else {
            match offset {
                0 | 4 | 8 | 12 => return Ok(()),
                _ => return Err(BusErrorKind::Unsupported("interrupt register offset")),
            }
        }
        if acknowledge {
            self.acknowledge_level15(processor);
        }
        self.refresh();
        Ok(())
    }
}
