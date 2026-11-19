//! Native x86-64 SETHI/ALU blocks and translated Bicc spans with optional delay slots.
//!
//! The core crate forbids unsafe code. This optional backend owns its small
//! executable-memory and calling-boundary audit. It never runs in the default
//! build. On non-x86-64 Unix it reports every block unsupported.

use std::collections::HashMap;

pub const MAX_BLOCK_INSNS: usize = 16;

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
    Branch {
        condition: u8,
        annul: bool,
        displacement: u32,
    },
}

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct State {
    pub regs: [u32; 32],
    pub icc: u32,
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
            if !matches!(op3, 0x00..=0x04 | 0x10..=0x14) {
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

pub fn decode_span(word: u32) -> Option<Op> {
    if word >> 30 == 0 && (word >> 22) & 7 == 2 {
        return Some(Op::Branch {
            condition: ((word >> 25) & 15) as u8,
            annul: word & (1 << 29) != 0,
            displacement: ((word << 10) as i32 >> 8) as u32,
        });
    }
    decode(word)
}

/// Compile the whole span. An optional Bicc ends an ALU/SETHI prefix.
pub fn compile_span(words: &[u32]) -> Option<NativeBlock> {
    compile_translated(words, false)
}

/// Compile an ALU/SETHI prefix, a Bicc and one possibly executed ALU/SETHI slot.
pub fn compile_span_with_slot(words: &[u32]) -> Option<NativeBlock> {
    compile_translated(words, true)
}

fn compile_translated(words: &[u32], with_slot: bool) -> Option<NativeBlock> {
    if words.is_empty() || words.len() > MAX_BLOCK_INSNS {
        return None;
    }
    let (body, slot) = words.split_at(words.len() - usize::from(with_slot));
    let (last, prefix) = body.split_last()?;
    let mut ops = Vec::with_capacity(words.len());
    for &word in prefix {
        ops.push(decode(word)?);
    }
    let tail = if prefix.is_empty() {
        decode(*last)?
    } else {
        decode_span(*last)?
    };
    ops.push(tail);
    if let [word] = slot {
        if !tail.can_fuse_delay_slot() {
            return None;
        }
        ops.push(decode(*word)?);
    }
    NativeBlock::emit(&ops, with_slot)
}

impl Op {
    pub fn can_fuse_delay_slot(self) -> bool {
        matches!(self, Self::Branch { condition, annul, .. } if !annul || !matches!(condition, 0 | 8))
    }
}

pub fn compile(words: &[u32]) -> Option<NativeBlock> {
    let mut ops = Vec::new();
    for &word in words.iter().take(MAX_BLOCK_INSNS) {
        ops.push(decode(word)?);
    }
    if ops.is_empty() {
        return None;
    }
    NativeBlock::emit(&ops, false)
}

#[cfg(all(unix, target_arch = "x86_64"))]
struct Executable {
    ptr: *mut u8,
    len: usize,
}

#[cfg(all(unix, target_arch = "x86_64"))]
impl Executable {
    fn new(bytes: &[u8]) -> Option<Self> {
        let len = bytes.len().max(4096);
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return None;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
            if libc::mprotect(ptr, len, libc::PROT_READ | libc::PROT_EXEC) != 0 {
                libc::munmap(ptr, len);
                return None;
            }
        }
        Some(Self {
            ptr: ptr.cast(),
            len,
        })
    }

    fn entry(&self) -> extern "C" fn(*mut State) -> u64 {
        unsafe { std::mem::transmute::<*mut u8, extern "C" fn(*mut State) -> u64>(self.ptr) }
    }
}

#[cfg(all(unix, target_arch = "x86_64"))]
impl Drop for Executable {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr.cast(), self.len);
        }
    }
}

/// A register-only prefix followed by an ordinary aligned word transfer.
pub struct WordPair {
    #[cfg(all(unix, target_arch = "x86_64"))]
    code: Executable,
    pub inputs: u32,
    pub outputs: u32,
    pub sets_icc: bool,
}

/// Only a borrowed backing word can reach generated code. Guest address bits
/// never serve as a host address; the core resolves the post-prefix address.
pub fn compile_word_pair(first: u32, last: u32, little: bool) -> Option<WordPair> {
    let prefix = decode(first)?;
    let store = match (last >> 30, (last >> 19) & 63) {
        (3, 0) => false,
        (3, 4) => true,
        _ => return None,
    };
    #[cfg(all(unix, target_arch = "x86_64"))]
    {
        let mut code = Vec::new();
        emit_op(&mut code, prefix)?;
        let rd = ((last >> 25) & 31) as u8;
        let (prefix_rd, mut inputs, sets_icc) = match prefix {
            Op::Sethi { rd, .. } => (rd, 0, false),
            Op::Alu { rd, rs1, rhs, op3 } => {
                let inputs = (1u32 << rs1)
                    | match rhs {
                        Operand::Register(r) => 1u32 << r,
                        Operand::Immediate(_) => 0,
                    };
                (rd, inputs, op3 & 16 != 0)
            }
            Op::Branch { .. } => return None,
        };
        let mut outputs = 1u32 << prefix_rd;
        if store {
            if prefix_rd != rd {
                inputs |= 1u32 << rd;
            }
            code.extend([0x8b, 0x47, rd * 4]); // mov eax,[rdi+4*rd]
        } else {
            outputs |= 1u32 << rd;
            code.extend([0x8b, 0x06]); // mov eax,[rsi]
        }
        if little {
            code.extend([0x0f, 0xc8]);
        } // bswap eax
        if store {
            code.extend([0x89, 0x06]); // mov [rsi],eax
        } else if rd != 0 {
            code.extend([0x89, 0x47, rd * 4]); // mov [rdi+4*rd],eax
        }
        code.push(0xc3);
        Some(WordPair {
            code: Executable::new(&code)?,
            inputs,
            outputs: outputs & !1,
            sets_icc,
        })
    }
    #[cfg(not(all(unix, target_arch = "x86_64")))]
    {
        let _ = (prefix, store, little);
        None
    }
}

impl WordPair {
    pub fn execute(&self, state: &mut State, word: &mut u32) {
        #[cfg(all(unix, target_arch = "x86_64"))]
        {
            // This entry point is created only by compile_word_pair. The two
            // exclusive borrows cover the entire call; no pointer is retained.
            let run = unsafe {
                std::mem::transmute::<*mut u8, extern "C" fn(*mut State, *mut u32)>(self.code.ptr)
            };
            run(state, word);
        }
        #[cfg(not(all(unix, target_arch = "x86_64")))]
        {
            let _ = (state, word);
            unreachable!("native backend unavailable");
        }
    }
}
impl std::fmt::Debug for WordPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordPair")
            .field("inputs", &self.inputs)
            .field("outputs", &self.outputs)
            .finish()
    }
}

/// Guest PC and nPC immediately after Bicc, as deltas from its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchExit {
    pub pc_delta: u32,
    pub npc_delta: u32,
}

pub struct NativeBlock {
    #[cfg(all(unix, target_arch = "x86_64"))]
    code: Executable,
    instructions: usize,
    has_delay_slot: bool,
    #[cfg(all(unix, target_arch = "x86_64"))]
    has_branch_exit: bool,
    inputs: u32,
    outputs: u32,
    sets_icc: bool,
}

impl NativeBlock {
    /// Number of encoded guest instructions, including any conditional slot.
    pub fn instructions(&self) -> usize {
        self.instructions
    }

    pub fn has_delay_slot(&self) -> bool {
        self.has_delay_slot
    }

    pub fn inputs(&self) -> u32 {
        self.inputs
    }

    pub fn outputs(&self) -> u32 {
        self.outputs
    }

    pub fn sets_icc(&self) -> bool {
        self.sets_icc
    }

    #[cfg(all(unix, target_arch = "x86_64"))]
    fn emit(ops: &[Op], with_slot: bool) -> Option<Self> {
        let (body, slot) = ops.split_at(ops.len() - usize::from(with_slot));
        let mut code = Vec::new();
        for &op in body {
            emit_op(&mut code, op)?;
        }
        let conditional_slot =
            with_slot && matches!(body.last(), Some(Op::Branch { annul: true, .. }));
        if let [op] = slot {
            if conditional_slot {
                // The native Bicc returns PC delta 4 on a taken conditional
                // branch and 8 on its annulled fall-through. Return before
                // touching any slot register on that fall-through.
                code.extend([0x83, 0xf8, 4, 0x74, 1, 0xc3]); // cmp eax,4; je +1; ret
            }
            code.extend([0x49, 0x89, 0xc0]); // mov r8,rax (pre-slot branch exit)
            emit_op(&mut code, *op)?; // ALU/SETHI only clobbers rax, rcx, rdx
            code.extend([0x4c, 0x89, 0xc0]); // mov rax,r8
        }
        let has_branch_exit = matches!(body.last(), Some(Op::Branch { .. }));
        if !has_branch_exit {
            code.extend([0x31, 0xc0]); // Return a defined zero for a straight-line block.
        }
        code.push(0xc3);
        let mut written = 0u32;
        let mut inputs = 0u32;
        let mut outputs = 0u32;
        let mut sets_icc = false;
        for op in ops {
            match *op {
                Op::Branch { .. } => {}
                Op::Sethi { rd, .. } => {
                    outputs |= 1u32 << rd;
                    written |= 1u32 << rd;
                }
                Op::Alu {
                    rd, rs1, rhs, op3, ..
                } => {
                    if written & (1u32 << rs1) == 0 {
                        inputs |= 1u32 << rs1;
                    }
                    if let Operand::Register(rs2) = rhs {
                        if written & (1u32 << rs2) == 0 {
                            inputs |= 1u32 << rs2;
                        }
                    }
                    outputs |= 1u32 << rd;
                    written |= 1u32 << rd;
                    sets_icc |= op3 & 0x10 != 0;
                }
            }
        }
        if conditional_slot {
            // The core writes back the block's static output mask. Seed a
            // slot-only destination from the guest in case the slot is skipped.
            match slot[0] {
                Op::Alu { rd, .. } | Op::Sethi { rd, .. } => inputs |= 1u32 << rd,
                Op::Branch { .. } => return None,
            }
        }
        Some(Self {
            code: Executable::new(&code)?,
            instructions: ops.len(),
            has_delay_slot: with_slot,
            has_branch_exit,
            inputs,
            outputs: outputs & !1,
            sets_icc,
        })
    }

    #[cfg(not(all(unix, target_arch = "x86_64")))]
    fn emit(_ops: &[Op], _with_slot: bool) -> Option<Self> {
        None
    }

    /// Returns the PC/nPC offsets selected by Bicc before its optional slot.
    /// For a block with a slot, PC delta 4 indicates that it executed.
    pub fn execute(&self, state: &mut State) -> Option<BranchExit> {
        #[cfg(all(unix, target_arch = "x86_64"))]
        {
            let entry = self.code.entry();
            let deltas = entry(state as *mut State);
            self.has_branch_exit.then_some(BranchExit {
                pc_delta: deltas as u32,
                npc_delta: (deltas >> 32) as u32,
            })
        }
        #[cfg(not(all(unix, target_arch = "x86_64")))]
        {
            let _ = state;
            unreachable!("native backend unavailable on this host");
        }
    }
}

impl std::fmt::Debug for NativeBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeBlock")
            .field("instructions", &self.instructions)
            .field("inputs", &self.inputs)
            .field("outputs", &self.outputs)
            .field("sets_icc", &self.sets_icc)
            .finish()
    }
}

#[cfg(all(unix, target_arch = "x86_64"))]
fn emit_op(code: &mut Vec<u8>, op: Op) -> Option<()> {
    match op {
        Op::Branch {
            condition,
            annul,
            displacement,
        } => {
            emit_branch_exit(code, condition, annul, displacement);
        }
        Op::Sethi { rd, value } => {
            if rd != 0 {
                // mov dword ptr [rdi + 4*rd], imm32
                code.extend([0xc7, 0x47, rd * 4]);
                code.extend(value.to_le_bytes());
            }
        }
        Op::Alu { op3, rd, rs1, rhs } => {
            // mov eax, [rdi + 4*rs1]
            code.extend([0x8b, 0x47, rs1 * 4]);
            match rhs {
                Operand::Register(index) => {
                    // mov ecx, [rdi + 4*index]
                    code.extend([0x8b, 0x4f, index * 4]);
                }
                Operand::Immediate(value) => {
                    // mov ecx, imm32
                    code.push(0xb9);
                    code.extend(value.to_le_bytes());
                }
            }
            match op3 & 0x0f {
                0x0 => code.extend([0x03, 0xc1]), // add eax, ecx
                0x1 => code.extend([0x23, 0xc1]), // and eax, ecx
                0x2 => code.extend([0x0b, 0xc1]), // or eax, ecx
                0x3 => code.extend([0x33, 0xc1]), // xor eax, ecx
                0x4 => code.extend([0x2b, 0xc1]), // sub eax, ecx
                _ => return None,
            }
            if rd != 0 {
                // mov [rdi + 4*rd], eax
                code.extend([0x89, 0x47, rd * 4]);
            }
            if op3 & 0x10 != 0 {
                emit_icc(code);
            }
        }
    }
    Some(())
}

#[cfg(all(unix, target_arch = "x86_64"))]
fn emit_branch_exit(code: &mut Vec<u8>, condition: u8, annul: bool, displacement: u32) {
    // Bit i in each mask corresponds to packed ICC = i (N,Z,V,C from bit 0).
    const N: u16 = 0xaaaa;
    const Z: u16 = 0xcccc;
    const V: u16 = 0xf0f0;
    const C: u16 = 0xff00;
    const CONDITIONS: [u16; 8] = [0, Z, Z | (N ^ V), N ^ V, C | Z, C, N, V];
    let pack = |pc: u32, npc: u32| (u64::from(npc) << 32) | u64::from(pc);
    let fallthrough = if annul { pack(8, 12) } else { pack(4, 8) };
    let taken = if annul && condition == 8 {
        pack(displacement, displacement.wrapping_add(4))
    } else {
        pack(4, displacement)
    };
    if condition == 0 || condition == 8 {
        code.extend([0x48, 0xb8]); // movabs rax, selected exit
        code.extend(if condition == 0 { fallthrough } else { taken }.to_le_bytes());
        return;
    }
    let mask = CONDITIONS[usize::from(condition & 7)];
    let mask = if condition & 8 == 0 { mask } else { !mask };
    code.push(0xb8); // mov eax, truth table
    code.extend(u32::from(mask).to_le_bytes());
    code.extend([0x8b, 0x8f, 0x80, 0, 0, 0]); // mov ecx, [rdi + 128] (post-prefix ICC)
    code.extend([0x0f, 0xa3, 0xc8]); // bt eax, ecx
    code.extend([0x48, 0xb8]); // movabs rax, fallthrough exit (preserves CF)
    code.extend(fallthrough.to_le_bytes());
    code.extend([0x48, 0xba]); // movabs rdx, taken exit
    code.extend(taken.to_le_bytes());
    code.extend([0x48, 0x0f, 0x42, 0xc2]); // cmovc rax, rdx
}

#[cfg(all(unix, target_arch = "x86_64"))]
fn emit_icc(code: &mut Vec<u8>) {
    // pushfq; pop rdx
    code.extend([0x9c, 0x5a]);
    // mov eax, edx
    code.extend([0x89, 0xd0]);
    // shr eax, 7; and eax, 1
    code.extend([0xc1, 0xe8, 7, 0x83, 0xe0, 1]);
    // mov ecx, edx; shr ecx, 6; and ecx, 1; shl ecx, 1; or eax, ecx
    code.extend([
        0x89, 0xd1, 0xc1, 0xe9, 6, 0x83, 0xe1, 1, 0xd1, 0xe1, 0x09, 0xc8,
    ]);
    // mov ecx, edx; shr ecx, 11; and ecx, 1; shl ecx, 2; or eax, ecx
    code.extend([
        0x89, 0xd1, 0xc1, 0xe9, 11, 0x83, 0xe1, 1, 0xc1, 0xe1, 2, 0x09, 0xc8,
    ]);
    // mov ecx, edx; and ecx, 1; shl ecx, 3; or eax, ecx
    code.extend([0x89, 0xd1, 0x83, 0xe1, 1, 0xc1, 0xe1, 3, 0x09, 0xc8]);
    // mov [rdi + 128], eax
    code.extend([0x89, 0x87, 0x80, 0x00, 0x00, 0x00]);
}

pub fn cache_key(pc: u32, mutation_epoch: u64) -> (u32, u64) {
    (pc, mutation_epoch)
}

pub type Cache = HashMap<(u32, u64), Option<NativeBlock>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sethi(rd: u32, value: u32) -> u32 {
        0x0100_0000 | (rd << 25) | ((value >> 10) & 0x3f_ffff)
    }

    fn alu(rd: u32, rs1: u32, imm: u32, op3: u32) -> u32 {
        0x8000_0000 | (rd << 25) | (op3 << 19) | (rs1 << 14) | 0x2000 | (imm & 0x1fff)
    }

    #[test]
    fn native_sethi_and_alu_execute() {
        let words = [sethi(1, 0x1234_4000), alu(2, 1, 3, 0), alu(3, 2, 1, 0x14)];
        let block = compile(&words).unwrap();
        let mut state = State::default();
        block.execute(&mut state);
        assert_eq!(state.regs[1], 0x1234_4000);
        assert_eq!(state.regs[2], 0x1234_4003);
        assert_eq!(state.regs[3], 0x1234_4002);
        assert_eq!(state.icc & 0xf, 0);
    }

    #[test]
    fn native_subcc_sets_condition_codes() {
        let words = [sethi(1, 0x1000), alu(2, 1, 1, 0x14)];
        let block = compile(&words).unwrap();
        let mut state = State::default();
        block.execute(&mut state);
        assert_eq!(state.regs[2], 0x1000 - 1);
        assert_eq!(state.icc & 8, 0);
        assert_eq!(state.icc & 4, 0);
    }
}

#[cfg(all(test, unix, target_arch = "x86_64"))]
mod span_tests {
    use super::*;
    #[test]
    fn branch_span_compiles() {
        let words = [0x80a0_6001, 0x12bf_ffff];
        assert!(decode_span(words[1]).is_some());
        assert!(compile_span(&words).is_some());
    }
}
