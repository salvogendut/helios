//! SPARCstation 20 built-in SX/cgfourteen and VSIMM functional subset.
//! Register layout: NetBSD sys/arch/sparc/dev/{cgfourteen,sx}reg.h.
use crate::bus::BusErrorKind;

pub const WIDTH: usize = 1152;
pub const HEIGHT: usize = 900;
pub const VRAM_BYTES: usize = 64 * 1024 * 1024;
pub const SX_BASE: u64 = 0xf_8000_0000;
pub const SX_SIZE: u64 = 0x2000;
pub const SX_MEM_BASE: u64 = 0x8_0000_0000;
pub const VSIMM_CTL_BASE: u64 = 0x9c00_0000;
pub const VSIMM_CTL_SIZE: u64 = 0x1_0000;
pub const VSIMM_VRAM_BASE: u64 = 0xfc00_0000;

const CG14_CLUT1: usize = 0x4000;
const CG14_RSR: usize = 6;
const SX_ID: usize = 0x28;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sx {
    control: Box<[u8; VSIMM_CTL_SIZE as usize]>,
    sx: Box<[u8; SX_SIZE as usize]>,
    vram: Vec<u8>,
    generation: u64,
}

impl Default for Sx {
    fn default() -> Self {
        Self::new()
    }
}

impl Sx {
    pub fn new() -> Self {
        let mut control = Box::new([0u8; VSIMM_CTL_SIZE as usize]);
        control[CG14_RSR] = 0x01;
        control[0x0d] = 0x01;
        for index in 0..256 {
            let value = match index {
                0 => 0x0000_0000,
                1 => 0x00ff_ffff,
                2..=15 => {
                    let level = (index as u32) * 17;
                    (level << 16) | (level << 8) | level
                }
                _ => {
                    let r = (index as u32 * 37) & 0xff;
                    let g = (index as u32 * 73) & 0xff;
                    let b = (index as u32 * 109) & 0xff;
                    (r << 16) | (g << 8) | b
                }
            };
            let off = CG14_CLUT1 + index * 4;
            control[off..off + 4].copy_from_slice(&value.to_be_bytes());
        }
        let mut sx = Box::new([0u8; SX_SIZE as usize]);
        sx[SX_ID..SX_ID + 4].copy_from_slice(&1u32.to_be_bytes());
        sx[8..12].copy_from_slice(&0xfc00_0000u32.to_be_bytes());
        sx[12..16].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
        Self {
            control,
            sx,
            vram: vec![0; VRAM_BYTES],
            generation: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn control_byte(&self, offset: usize) -> u8 {
        self.control[offset]
    }
    pub fn vram_byte(&self, offset: usize) -> u8 {
        self.vram[offset]
    }
    pub fn vram_word(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.vram[offset..offset + 4].try_into().unwrap())
    }

    pub fn contains(&self, address: u64) -> bool {
        let ctl = (VSIMM_CTL_BASE..VSIMM_CTL_BASE + VSIMM_CTL_SIZE).contains(&address);
        let vram = (VSIMM_VRAM_BASE..VSIMM_VRAM_BASE + VRAM_BYTES as u64).contains(&address);
        let alias = (SX_MEM_BASE..SX_MEM_BASE + VRAM_BYTES as u64).contains(&address);
        let regs = (SX_BASE..SX_BASE + SX_SIZE).contains(&address);
        ctl || vram || alias || regs
    }

    fn vram_offset(&self, address: u64) -> Option<usize> {
        if (VSIMM_VRAM_BASE..VSIMM_VRAM_BASE + VRAM_BYTES as u64).contains(&address) {
            Some((address - VSIMM_VRAM_BASE) as usize)
        } else if (SX_MEM_BASE..SX_MEM_BASE + VRAM_BYTES as u64).contains(&address) {
            Some((address - SX_MEM_BASE) as usize)
        } else {
            None
        }
    }

    pub fn read(&mut self, address: u64, bytes: u64) -> Result<u32, BusErrorKind> {
        if let Some(offset) = self.vram_offset(address) {
            return self.read_bytes(&self.vram, offset, bytes);
        }
        if (SX_BASE..SX_BASE + SX_SIZE).contains(&address) {
            return self.read_bytes(&self.sx[..], (address - SX_BASE) as usize, bytes);
        }
        if (VSIMM_CTL_BASE..VSIMM_CTL_BASE + VSIMM_CTL_SIZE).contains(&address) {
            let offset = (address - VSIMM_CTL_BASE) as usize;
            if offset == CG14_RSR {
                return Ok(0x01u32);
            }
            return self.read_bytes(&self.control[..], offset, bytes);
        }
        Err(BusErrorKind::Unmapped)
    }

    pub fn write(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusErrorKind> {
        if let Some(offset) = self.vram_offset(address) {
            Self::write_bytes(&mut self.vram, offset, value, mask);
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        if (SX_BASE..SX_BASE + SX_SIZE).contains(&address) {
            let offset = (address - SX_BASE) as usize;
            if offset != SX_ID {
                Self::write_bytes(&mut self.sx[..], offset, value, mask);
                self.generation = self.generation.wrapping_add(1);
            }
            return Ok(());
        }
        if (VSIMM_CTL_BASE..VSIMM_CTL_BASE + VSIMM_CTL_SIZE).contains(&address) {
            let offset = (address - VSIMM_CTL_BASE) as usize;
            Self::write_bytes(&mut self.control[..], offset, value, mask);
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        Err(BusErrorKind::Unmapped)
    }

    fn read_bytes(&self, data: &[u8], offset: usize, bytes: u64) -> Result<u32, BusErrorKind> {
        if bytes != 4 && bytes != 2 && bytes != 1 {
            return Err(BusErrorKind::Unsupported("SX access width"));
        }
        if offset + bytes as usize > data.len() || offset % bytes as usize != 0 {
            return Err(BusErrorKind::Unmapped);
        }
        let mut value = 0u32;
        for byte in &data[offset..offset + bytes as usize] {
            value = (value << 8) | u32::from(*byte);
        }
        Ok(value)
    }

    fn write_bytes(data: &mut [u8], offset: usize, value: u32, mask: u32) {
        let bytes = mask.count_ones().div_ceil(8) as usize;
        if bytes == 0 || offset + bytes > data.len() {
            return;
        }
        let lane = offset & 3;
        if bytes < 4 {
            if lane + bytes > 4 {
                return;
            }
            let base = offset & !3;
            let shift = (4 - lane - bytes) * 8;
            let word_mask = mask << shift;
            let word_value = value << shift;
            for i in 0..4 {
                let m = (word_mask >> ((3 - i) * 8)) & 0xff;
                let v = (word_value >> ((3 - i) * 8)) & 0xff;
                data[base + i] = (data[base + i] & !(m as u8)) | (v as u8 & m as u8);
            }
        } else {
            for i in 0..bytes {
                let shift = (bytes - 1 - i) * 8;
                let byte_mask = (mask >> shift) & 0xff;
                let old = data[offset + i];
                data[offset + i] =
                    (old & !(byte_mask as u8)) | ((value >> shift) as u8 & byte_mask as u8);
            }
        }
    }

    pub fn pixels(&self, output: &mut [u32]) {
        assert_eq!(output.len(), WIDTH * HEIGHT);
        for (index, pixel) in output.iter_mut().enumerate() {
            let color = usize::from(self.vram[index]);
            let off = CG14_CLUT1 + color * 4;
            let rgb = u32::from_be_bytes(self.control[off..off + 4].try_into().unwrap());
            let r = (rgb >> 16) & 0xff;
            let g = (rgb >> 8) & 0xff;
            let b = rgb & 0xff;
            *pixel = (r << 16) | (g << 8) | b;
        }
    }
}
