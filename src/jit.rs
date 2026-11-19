//! Safe basic-block JIT for straight-line SETHI and integer ALU operations.
//! This feature is experimental and default-off. It emits interpreter IR, not
//! host machine code, so the crate keeps `unsafe_code = "forbid"`.

use crate::cpu::Step;
use std::collections::HashMap;

pub const MAX_BLOCK_INSNS: usize = 16;
pub const MAX_CACHE_ENTRIES: usize = 64;
pub const MAX_COMPILE_ATTEMPTS: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Register(u8),
    Immediate(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Sethi {
        rd: u8,
        value: u32,
    },
    Alu {
        op3: u8,
        rd: u8,
        rs1: u8,
        rhs: Operand,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub pc: u32,
    pub supervisor: bool,
    pub mutation_epoch: u64,
    pub mmu_generation: u64,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub ops: Vec<Op>,
}

#[derive(Clone, Debug, Default)]
pub struct Cache {
    entries: HashMap<Key, Block>,
}

impl Cache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get(&self, key: &Key) -> Option<&Block> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: Key, block: Block) {
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.entries.clear();
        }
        self.entries.insert(key, block);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    pub executed: u64,
    pub compiled_blocks: u64,
    pub compiled_instructions: u64,
    pub compile_calls: u64,
    pub fallback_steps: u64,
    pub compile_ns: u64,
    pub stopped: Option<Step>,
}

#[cfg(feature = "jit-native")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeRunStats {
    pub executed: u64,
    pub native_blocks: u64,
    pub native_instructions: u64,
    pub compiled_instructions: u64,
    pub cache_hits: u64,
    pub evictions: u64,
    pub compile_calls: u64,
    pub compile_ns: u64,
    pub fallback_steps: u64,
    pub stopped: Option<Step>,
}

pub fn decode(word: u32) -> Option<Op> {
    let rd = ((word >> 25) & 31) as u8;
    match word >> 30 {
        0 if (word >> 22) & 7 == 4 => Some(Op::Sethi {
            rd,
            value: (word & 0x3f_ffff) << 10,
        }),
        2 => {
            let op3 = ((word >> 19) & 63) as u8;
            if !matches!(op3, 0x00..=0x08 | 0x0c | 0x10..=0x18 | 0x1c) {
                return None;
            }
            let rs1 = ((word >> 14) & 31) as u8;
            let rhs = if word & (1 << 13) != 0 {
                Operand::Immediate(((word << 19) as i32 >> 19) as u32)
            } else {
                Operand::Register((word & 31) as u8)
            };
            Some(Op::Alu { op3, rd, rs1, rhs })
        }
        _ => None,
    }
}

pub fn compile(words: &[u32]) -> Block {
    Block {
        ops: words
            .iter()
            .take(MAX_BLOCK_INSNS)
            .map_while(|word| decode(*word))
            .collect(),
    }
}

#[cfg(feature = "jit-native")]
pub const MAX_ENGINE_ENTRIES: usize = 256;

#[cfg(feature = "jit-native")]
struct NativeEntry {
    pc: u32,
    words: Vec<u32>,
    epoch: u64,
    block: Option<helios_jit_native::NativeBlock>,
    last_used: u64,
}

#[cfg(feature = "jit-native")]
pub enum NativeLookup<'a> {
    Miss,
    Negative,
    Block(&'a helios_jit_native::NativeBlock),
}

#[cfg(feature = "jit-native")]
#[derive(Default)]
pub struct NativeEngine {
    entries: Vec<NativeEntry>,
    tick: u64,
    pub hits: u64,
    pub evictions: u64,
}

#[cfg(feature = "jit-native")]
impl std::fmt::Debug for NativeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeEngine")
            .field("entries", &self.entries.len())
            .field("hits", &self.hits)
            .field("evictions", &self.evictions)
            .finish()
    }
}

impl NativeEngine {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn lookup_epoch(&mut self, pc: u32, epoch: u64) -> NativeLookup<'_> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        if let Some(index) = self.entries.iter().position(|entry| entry.pc == pc) {
            if self.entries[index].epoch == epoch {
                self.entries[index].last_used = tick;
                self.hits = self.hits.wrapping_add(1);
                return match &self.entries[index].block {
                    Some(block) => NativeLookup::Block(block),
                    None => NativeLookup::Negative,
                };
            }
        }
        NativeLookup::Miss
    }

    pub fn validate(&mut self, pc: u32, words: &[u32], epoch: u64) -> NativeLookup<'_> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        if let Some(index) = self.entries.iter().position(|entry| entry.pc == pc) {
            if self.entries[index].words == words {
                self.entries[index].last_used = tick;
                self.entries[index].epoch = epoch;
                self.hits = self.hits.wrapping_add(1);
                return match &self.entries[index].block {
                    Some(block) => NativeLookup::Block(block),
                    None => NativeLookup::Negative,
                };
            }
            self.entries.swap_remove(index);
        }
        NativeLookup::Miss
    }

    pub fn insert(
        &mut self,
        pc: u32,
        words: &[u32],
        epoch: u64,
        block: Option<helios_jit_native::NativeBlock>,
    ) {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        if self.entries.len() >= MAX_ENGINE_ENTRIES {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
                .unwrap_or(0);
            self.entries.swap_remove(oldest);
            self.evictions = self.evictions.wrapping_add(1);
        }
        self.entries.push(NativeEntry {
            pc,
            words: words.to_vec(),
            epoch,
            block,
            last_used: tick,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_stops_at_unsupported_instruction() {
        let block = compile(&[0x0100_0000, 0x8400_a001, 0x0100_0000, 0x81c3_e008]);
        assert_eq!(block.ops.len(), 3);
        assert!(matches!(block.ops[0], Op::Sethi { .. }));
        assert!(matches!(block.ops[1], Op::Alu { .. }));
    }

    #[test]
    fn decoder_rejects_memory_and_branch_words() {
        assert!(decode(0xc400_6000).is_none());
        assert!(decode(0x12bf_fffd).is_none());
    }
}
