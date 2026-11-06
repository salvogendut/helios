//! microSPARC-II floating-point state and non-arithmetic operations.
//! See docs/fpu.md for the transfer subset and deferred-exception limits.

const FSR_VERSION: u32 = 4 << 17; // Meiko, microSPARC-II table 6
const FSR_WRITABLE: u32 = 0xcf80_0fff; // RD, TEM, FCC, AEXC, CEXC; NS is zero
const FSR_FTT: u32 = 7 << 14;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fpu {
    registers: [u32; 32],
    status: u32,
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
        self.status | FSR_VERSION
    }

    pub(crate) fn load_fsr(&mut self, value: u32) {
        // LDFSR cannot change FTT or QNE. No queued operations exist in this
        // subset; arithmetic/deferred exceptions remain explicit host stops.
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
