//! microSPARC-II diagnostic RAM and line invalidation. Normal memory accesses
//! still use the coherent bus path; this is not a cache hit/fill implementation.
//! See docs/cache.md for the hardware sources and the functional boundary.

use crate::mmu::Width;

const VALID: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Array<const WORDS: usize, const LINE_SHIFT: u32> {
    tags: Box<[u32]>,
    data: Box<[u32]>,
}

impl<const WORDS: usize, const LINE_SHIFT: u32> Default for Array<WORDS, LINE_SHIFT> {
    fn default() -> Self {
        Self {
            tags: vec![0; 512].into_boxed_slice(),
            data: vec![0; WORDS].into_boxed_slice(),
        }
    }
}

impl<const WORDS: usize, const LINE_SHIFT: u32> Array<WORDS, LINE_SHIFT> {
    const ADDRESS_MASK: u32 = !((WORDS as u32 * 4) - 1);

    fn tag_index(address: u32) -> usize {
        ((address >> LINE_SHIFT) & 511) as usize
    }

    fn read_tag(&self, address: u32) -> u32 {
        self.tags[Self::tag_index(address)]
    }

    fn write_tag(&mut self, address: u32, value: u32) {
        // Context[11:4], ACC[3:1], valid[0]. Reserved bits read as zero.
        self.tags[Self::tag_index(address)] = value & (Self::ADDRESS_MASK | 0xfff);
    }

    fn read_word(&self, address: u32) -> u32 {
        self.data[(address as usize >> 2) & (WORDS - 1)]
    }

    fn write_word(&mut self, address: u32, value: u32, mask: u32) {
        let word = &mut self.data[(address as usize >> 2) & (WORDS - 1)];
        *word = (*word & !mask) | (value & mask);
    }

    fn flush(&mut self, address: u32, context: u8, scope: u8) {
        let tag = &mut self.tags[Self::tag_index(address)];
        let supervisor = (*tag >> 1) & 7 >= 6;
        let same_context = ((*tag >> 4) & 0xff) == u32::from(context);
        // The cache index supplies address bits below the stored VA tag. Only
        // one indexed line is inspected, even for context/user flushes.
        let address_mask = match scope {
            0 => 0xffff_f000,
            1 => 0xfffc_0000,
            2 => 0xff00_0000,
            _ => 0,
        } & Self::ADDRESS_MASK;
        let matches = match scope {
            0..=2 => (supervisor || same_context) && (*tag ^ address) & address_mask == 0,
            3 => !supervisor && same_context,
            4 => !supervisor,
            _ => unreachable!(),
        };
        if matches {
            *tag &= !VALID;
        }
    }

    fn flush_instruction(&mut self, address: u32) {
        let tag = &mut self.tags[Self::tag_index(address)];
        // FLUSH may conservatively invalidate other contexts at the same VA.
        if (*tag ^ address) & Self::ADDRESS_MASK == 0 {
            *tag &= !VALID;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CacheDiagnostics {
    instruction: Array<4096, 5>,
    data: Array<2048, 4>,
    reference: bool,
    instruction_tags64: Box<[[u32; 2]]>,
    data_tags64: Box<[[u32; 2]]>,
}

impl Default for CacheDiagnostics {
    fn default() -> Self {
        Self {
            instruction: Array::default(),
            data: Array::default(),
            reference: false,
            instruction_tags64: vec![[0, 0]; 512].into_boxed_slice(),
            data_tags64: vec![[0, 0]; 512].into_boxed_slice(),
        }
    }
}

impl CacheDiagnostics {
    pub(crate) fn set_reference(&mut self, value: bool) {
        self.reference = value;
    }

    // The caller checks widths and alignment before any of these operations.
    pub(crate) fn read(&self, address: u32, asi: u8, width: Width) -> u32 {
        if self.reference && asi == 0x0e {
            return self.data_tags64[((address >> 3) & 511) as usize]
                [((address >> 2) & 1) as usize];
        }
        match asi {
            0x0c => self.instruction.read_tag(address),
            0x0d => self.instruction.read_word(address),
            0x0e => self.data.read_tag(address),
            0x0f => {
                let (mask, shift) = lane(address, width);
                (self.data.read_word(address) >> shift) & mask
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn write(&mut self, address: u32, asi: u8, width: Width, value: u32) {
        if self.reference && asi == 0x0e {
            let index = ((address >> 3) & 511) as usize;
            let half = ((address >> 2) & 1) as usize;
            let low_mask = if address & 0x4000_0000 != 0 && address & 0x8000_0000 == 0 {
                0x0000_0f0e
            } else {
                0x00ff_ffff
            };
            let mask = [low_mask, 0x0100_0000][half];
            self.data_tags64[index][half] = value & mask;
            return;
        }
        if self.reference && asi == 0x0c {
            let index = ((address >> 3) & 511) as usize;
            let half = ((address >> 2) & 1) as usize;
            let low_mask = if address & 0x4000_0000 != 0 && address & 0x8000_0000 == 0 {
                0x0000_1f1e
            } else {
                0x00ff_ffff
            };
            let mask = [low_mask, 0x0300_0000][half];
            self.instruction_tags64[index][half] = value & mask;
            return;
        }
        match asi {
            0x0c => self.instruction.write_tag(address, value),
            0x0d => self.instruction.write_word(address, value, u32::MAX),
            0x0e => self.data.write_tag(address, value),
            0x0f => {
                let (mask, shift) = lane(address, width);
                self.data.write_word(address, value << shift, mask << shift);
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn read_doubleword(&self, address: u32, asi: u8) -> [u32; 2] {
        if self.reference && asi == 0x0e {
            let tag = self.data_tags64[((address >> 3) & 511) as usize];
            return [tag[1], tag[0]];
        }
        if self.reference && asi == 0x0c {
            let tag = self.instruction_tags64[((address >> 3) & 511) as usize];
            return [tag[1], tag[0]];
        }
        match asi {
            0x0c..=0x0e => [
                self.read(address, asi, Width::Word),
                self.read(address + 4, asi, Width::Word),
            ],
            _ => [
                self.data.read_word(address),
                self.data.read_word(address + 4),
            ],
        }
    }

    pub(crate) fn write_doubleword(&mut self, address: u32, asi: u8, words: [u32; 2]) {
        match asi {
            0x0c..=0x0e => {
                self.write(address, asi, Width::Word, words[0]);
                self.write(address + 4, asi, Width::Word, words[1]);
            }
            _ => {
                self.data.write_word(address, words[0], u32::MAX);
                self.data.write_word(address + 4, words[1], u32::MAX);
            }
        }
    }

    pub(crate) fn flush(&mut self, address: u32, context: u8, asi: u8) {
        self.instruction.flush(address, context, asi - 0x10);
        self.data.flush(address, context, asi - 0x10);
    }

    pub(crate) fn flush_instruction(&mut self, address: u32) {
        self.instruction.flush_instruction(address);
    }
}

fn lane(address: u32, width: Width) -> (u32, u32) {
    match width {
        Width::Byte => (0xff, (3 - (address & 3)) * 8),
        Width::Half => (0xffff, (2 - (address & 2)) * 8),
        Width::Word => (u32::MAX, 0),
    }
}
