//! microSPARC-II floating-point state, comparisons and deferred exceptions.
//! See docs/fpu.md for the serialized execution model and remaining limits.

const FSR_VERSION: u32 = 4 << 17; // Meiko, microSPARC-II table 6
const FSR_WRITABLE: u32 = 0xcf80_0fff; // RD, TEM, FCC, AEXC, CEXC; NS is zero
const FSR_FTT: u32 = 7 << 14;
const FSR_FCC: u32 = 3 << 10;
const NV: u32 = 1 << 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FpuState {
    #[default]
    Execute,
    ExceptionPending,
    Exception,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeferredEntry {
    pub pc: u32,
    pub instruction: u32,
}

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Load,
    Store,
    Branch,
    Operate,
}

pub(crate) enum Action {
    Execute,
    /// A sequence error was deferred; do not perform the instruction's effects.
    Skip,
    Trap,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fpu {
    registers: [u32; 32],
    status: u32,
    state: FpuState,
    // One FPop resolves before another is issued in this interpreter. Hardware
    // has three pipeline entries; only the faulting entry can remain here.
    deferred: Option<DeferredEntry>,
}

impl Fpu {
    /// Raw guest bits. All registers, including f0, are ordinary storage.
    pub fn read_reg(&self, index: u8) -> u32 {
        self.registers[usize::from(index)]
    }

    /// Debug/test setup, independent of PSR.EF. Guest transfers check EF in Cpu.
    pub fn write_reg(&mut self, index: u8, word: u32) {
        self.registers[usize::from(index)] = word;
    }

    pub fn fsr(&self) -> u32 {
        self.status | FSR_VERSION | (u32::from(self.deferred.is_some()) << 13)
    }

    pub fn state(&self) -> FpuState {
        self.state
    }

    pub fn deferred_entry(&self) -> Option<DeferredEntry> {
        self.deferred
    }

    /// Called after IU privilege/EF/alignment checks, before any FP effect or
    /// data translation. microSPARC-II 4.4, not QEMU's immediate-trap shortcut.
    pub(crate) fn prepare(&mut self, operation: Operation) -> Action {
        match self.state {
            FpuState::Execute => Action::Execute,
            FpuState::ExceptionPending => {
                self.state = FpuState::Exception;
                Action::Trap
            }
            FpuState::Exception if matches!(operation, Operation::Load | Operation::Operate) => {
                self.status = (self.status & !FSR_FTT) | (4 << 14);
                self.state = FpuState::ExceptionPending;
                Action::Skip
            }
            FpuState::Exception => Action::Execute,
        }
    }

    /// Destructive queue read commits only after STDFQ's memory store succeeds.
    pub(crate) fn finish_queue_store(&mut self) {
        self.deferred = None;
        self.state = FpuState::Execute;
    }

    pub(crate) fn load_fsr(&mut self, value: u32) {
        // LDFSR cannot change FTT or QNE (the latter is derived from the queue).
        self.status = (self.status & !FSR_WRITABLE) | (value & FSR_WRITABLE);
    }

    pub(crate) fn empty_queue_error(&mut self) {
        self.status = (self.status & !FSR_FTT) | (4 << 14);
    }

    /// Returns false for an unimplemented FPop, without changing any state.
    pub(crate) fn move_word(&mut self, opf: u32, rd: u8, rs2: u8) -> bool {
        let source = self.read_reg(rs2);
        let result = match opf {
            0x01 => source,
            0x05 => source ^ 0x8000_0000,
            0x09 => source & 0x7fff_ffff,
            _ => return false,
        };
        self.write_reg(rd, result);
        // A completed FPop clears FTT (microSPARC-II 4.5). These three cannot
        // raise IEEE exceptions: clear CEXC, retain accrued exceptions and FCC.
        self.status &= !(FSR_FTT | 0x1f);
        true
    }

    pub(crate) fn compare(
        &mut self,
        opf: u32,
        rs1: u8,
        rs2: u8,
        pc: u32,
        instruction: u32,
    ) -> bool {
        let (a, b, sign, exponent, quiet) = match opf {
            0x51 | 0x55 => (
                u64::from(self.read_reg(rs1)),
                u64::from(self.read_reg(rs2)),
                1 << 31,
                0x7f80_0000,
                1 << 22,
            ),
            0x52 | 0x56 => (
                self.pair(rs1),
                self.pair(rs2),
                1 << 63,
                0x7ff0_0000_0000_0000,
                1 << 51,
            ),
            _ => return false,
        };
        let fraction = (quiet << 1) - 1;
        let nan_a = a & exponent == exponent && a & fraction != 0;
        let nan_b = b & exponent == exponent && b & fraction != 0;
        let unordered = nan_a || nan_b;
        let invalid =
            (nan_a && a & quiet == 0) || (nan_b && b & quiet == 0) || (opf & 4 != 0 && unordered);
        if invalid && self.status & (NV << 23) != 0 {
            // Trap inhibits FCC and AEXC updates; only CEXC/FTT change.
            self.status = (self.status & !(FSR_FTT | 0x1f)) | (1 << 14) | NV;
            self.deferred = Some(DeferredEntry { pc, instruction });
            self.state = FpuState::ExceptionPending;
        } else {
            let fcc = if unordered {
                3
            } else if a == b || (a & !sign == 0 && b & !sign == 0) {
                0 // signed zeros compare equal
            } else {
                let less = if (a ^ b) & sign != 0 {
                    a & sign != 0
                } else if a & sign != 0 {
                    a > b // negative encodings run opposite to numeric order
                } else {
                    a < b
                };
                if less {
                    1
                } else {
                    2
                }
            };
            self.status = (self.status & !(FSR_FTT | FSR_FCC | 0x1f))
                | (fcc << 10)
                | if invalid { NV | (NV << 5) } else { 0 };
        }
        true
    }

    fn pair(&self, index: u8) -> u64 {
        (u64::from(self.read_reg(index & !1)) << 32) | u64::from(self.read_reg(index | 1))
    }

    pub(crate) fn matches(&self, condition: u32) -> bool {
        let fcc = (self.status >> 10) & 3;
        let equal = fcc == 0;
        let less = fcc == 1;
        let greater = fcc == 2;
        let unordered = fcc == 3;
        match condition {
            0 => false,
            1 => !equal,
            2 => less || greater,
            3 => less || unordered,
            4 => less,
            5 => greater || unordered,
            6 => greater,
            7 => unordered,
            8 => true,
            9 => equal,
            10 => equal || unordered,
            11 => equal || greater,
            12 => !less,
            13 => equal || less,
            14 => !greater,
            15 => !unordered,
            _ => unreachable!("four-bit condition"),
        }
    }
}
