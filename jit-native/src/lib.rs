//! Isolated native x86-64 backend for straight-line SETHI/ALU blocks.
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

pub fn compile_span(words: &[u32]) -> Option<NativeBlock> {
    let mut ops = Vec::new();
    for &word in words.iter().take(MAX_BLOCK_INSNS) {
        ops.push(decode_span(word)?);
    }
    if ops.is_empty() {
        return None;
    }
    NativeBlock::emit(&ops)
}

pub fn compile(words: &[u32]) -> Option<NativeBlock> {
    let mut ops = Vec::new();
    for &word in words.iter().take(MAX_BLOCK_INSNS) {
        ops.push(decode(word)?);
    }
    if ops.is_empty() {
        return None;
    }
    NativeBlock::emit(&ops)
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

    fn entry(&self) -> extern "C" fn(*mut State) {
        unsafe { std::mem::transmute::<*mut u8, extern "C" fn(*mut State)>(self.ptr) }
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

pub struct NativeBlock {
    #[cfg(all(unix, target_arch = "x86_64"))]
    code: Executable,
    instructions: usize,
    inputs: u32,
    outputs: u32,
    sets_icc: bool,
}

impl NativeBlock {
    pub fn instructions(&self) -> usize {
        self.instructions
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
    fn emit(ops: &[Op]) -> Option<Self> {
        let mut code = Vec::new();
        for op in ops {
            emit_op(&mut code, *op)?;
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
        Some(Self {
            code: Executable::new(&code)?,
            instructions: ops.len(),
            inputs,
            outputs: outputs & !1,
            sets_icc,
        })
    }

    #[cfg(not(all(unix, target_arch = "x86_64")))]
    fn emit(_ops: &[Op]) -> Option<Self> {
        None
    }

    pub fn execute(&self, state: &mut State) {
        #[cfg(all(unix, target_arch = "x86_64"))]
        {
            let entry = self.code.entry();
            entry(state as *mut State);
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
        Op::Branch { .. } => code.push(0x90),
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

#[cfg(test)]
mod span_tests {
    use super::*;
    #[test]
    fn branch_span_compiles() {
        let words = [0x80a0_6001, 0x12bf_ffff];
        assert!(decode_span(words[1]).is_some());
        assert!(compile_span(&words).is_some());
    }
}
