//! SPARCstation 20 built-in SX/cgfourteen and VSIMM functional subset.
//! Register layout: NetBSD sys/arch/sparc/dev/{cgfourteen,sx}reg.h.
use crate::bus::BusErrorKind;

pub const WIDTH: usize = 1152;
pub const HEIGHT: usize = 900;
/// Installed VSIMM storage. OBP derives the framebuffer size from this value;
/// a 64 MiB backing store made it publish a false 16 MiB frame buffer.
pub const VRAM_BYTES: usize = 8 * 1024 * 1024;
/// Decoded VSIMM aperture. Storage is mirrored through this larger window.
pub const VRAM_APERTURE_BYTES: u64 = 0x0400_0000;
pub const SX_BASE: u64 = 0xf_8000_0000;
pub const SX_SIZE: u64 = 0x2000;
/// SX memory operations address `0x800000000 + target physical address`.
/// The framebuffer target is at `VSIMM_VRAM_BASE`, not at the alias base.
pub const SX_MEM_BASE: u64 = 0x8_0000_0000;
pub const SX_MEM_SIZE: u64 = 0x1_0000_0000;
pub const SX_VRAM_ALIAS_BASE: u64 = SX_MEM_BASE + VSIMM_VRAM_BASE;
pub const VSIMM_CTL_BASE: u64 = 0x9c00_0000;
pub const VSIMM_CTL_SIZE: u64 = 0x1_0000;
pub const VSIMM_VRAM_BASE: u64 = 0xfc00_0000;

const CG14_CLUT1: usize = 0x4000;
const CG14_RSR: usize = 6;
// VSIMM 2: MDI revision 1, double-buffered/three-CLUT implementation 1.
// Sun cg14reg.h: VBC VCA at +0x20c, HIRES bit 13 (8 MiB), version bits
// 11:10. Keep these board straps consistent with the installed VRAM.
const CG14_REVISION: u8 = 0x11;
const VBC_VCA: usize = 0x20c;
const VBC_STRAPS: u32 = (1 << 13) | (1 << 10);
const SX_CONTROL_STATUS: usize = 0x00;
const SX_ERROR: usize = 0x04;
const SX_PAGE_BOUND_LOWER: usize = 0x08;
const SX_PAGE_BOUND_UPPER: usize = 0x0c;
const SX_PLANEMASK: usize = 0x10;
const SX_ROP_CONTROL: usize = 0x14;
const SX_IQ_OVERFLOW: usize = 0x18;
const SX_DIAGNOSTICS: usize = 0x1c;
const SX_INSTRUCTIONS: usize = 0x20;
const SX_ID: usize = 0x28;
const SX_R0_INIT: usize = 0x2c;
const SX_SOFTRESET: usize = 0x30;
const SX_SYNC: usize = 0x34;
const SX_DIRECT_R0: usize = 0x100;
const SX_DIRECT_END: usize = SX_DIRECT_R0 + 0x1fc;
const SX_QUEUED_R0: usize = 0x300;
const SX_QUEUED_END: usize = SX_QUEUED_R0 + 0x1fc;
const SX_USER_PAGE: usize = 0x1000;

const SX_GO: u32 = 0x1000;
const SX_MT: u32 = 0x4000;
const RETRACE_PERIOD_NS: u64 = 1_000_000_000 / 76;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableStream {
    Xlut,
    Clut(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sx {
    control: Box<[u8; VSIMM_CTL_SIZE as usize]>,
    regs: [u32; 128],
    status: u32,
    error: u32,
    page_bound_lower: u32,
    page_bound_upper: u32,
    plane_mask: u32,
    rop_control: u32,
    overflow: u32,
    diagnostics: u32,
    go: bool,
    vram: Vec<u8>,
    generation: u64,
    table_stream: Option<TableStream>,
    table_index: usize,
    retrace_phase_ns: u64,
    retrace_pending: bool,
    trace: bool,
    fb_trace: u32,
}

impl Default for Sx {
    fn default() -> Self {
        Self::new()
    }
}

impl Sx {
    pub fn new() -> Self {
        let mut control = Box::new([0u8; VSIMM_CTL_SIZE as usize]);
        control[0] = 0x41; // VSIMM 2 video power and active-low blanking.
        control[CG14_RSR] = CG14_REVISION;
        control[VBC_VCA..VBC_VCA + 4].copy_from_slice(&VBC_STRAPS.to_be_bytes());
        // Monitor sense 4 selects the renderer's 1152x900 timing.
        control[4] = 4 << 1;
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
            // CG14 CLUT words are 00BBGGRR, not 00RRGGBB.
            let value = ((value & 0xff) << 16) | (value & 0x0000_ff00) | ((value >> 16) & 0xff);
            control[off..off + 4].copy_from_slice(&value.to_be_bytes());
        }
        Self {
            control,
            regs: [0; 128],
            status: SX_MT,
            error: 0,
            page_bound_lower: VSIMM_VRAM_BASE as u32,
            page_bound_upper: u32::MAX,
            plane_mask: u32::MAX,
            rop_control: 0,
            overflow: 0,
            diagnostics: 0x4804,
            go: false,
            vram: vec![0; VRAM_BYTES],
            generation: 0,
            table_stream: None,
            table_index: 0,
            retrace_phase_ns: 0,
            retrace_pending: false,
            trace: false,
            fb_trace: 0,
        }
    }

    pub fn reset(&mut self) {
        let trace = self.trace;
        *self = Self::new();
        self.trace = trace;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn set_trace(&mut self, trace: bool) {
        self.trace = trace;
    }

    pub fn advance_ns(&mut self, ns: u64) {
        self.retrace_phase_ns = self.retrace_phase_ns.saturating_add(ns);
        while self.retrace_phase_ns >= RETRACE_PERIOD_NS {
            self.retrace_phase_ns -= RETRACE_PERIOD_NS;
            if self.control[0] & 0x81 == 0x81 {
                self.retrace_pending = true;
            }
        }
    }

    pub fn irq(&self) -> bool {
        self.retrace_pending && self.control[0] & 0x80 != 0
    }

    fn msr(&self) -> u8 {
        (self.control[4] & !0x30) | if self.retrace_pending { 0x30 } else { 0 }
    }

    fn table_stream(&mut self, offset: usize, value: u32, mask: u32) -> bool {
        if (0x3800..0x3900).contains(&offset) {
            self.table_stream = Some(TableStream::Xlut);
            self.table_index = offset - 0x3800;
            return true;
        }
        for (base, table) in [(0x4800, 0), (0x5800, 1), (0x6800, 2)] {
            if (base..base + 0x400).contains(&offset) {
                self.table_stream = Some(TableStream::Clut(table));
                self.table_index = (offset - base) / 4;
                return true;
            }
        }
        if offset != 0xf000 {
            return false;
        }
        match self.table_stream {
            Some(TableStream::Xlut) if self.table_index < 256 => {
                self.control[0x3000 + self.table_index] = value as u8;
                self.table_index += 1;
            }
            Some(TableStream::Clut(table)) if self.table_index < 256 => {
                let base = 0x4000 + table * 0x1000 + self.table_index * 4;
                Self::write_bytes(&mut self.control[..], base, value, mask);
                self.table_index += 1;
            }
            Some(TableStream::Xlut) | Some(TableStream::Clut(_)) | None => {}
        }
        true
    }
    pub fn control_byte(&self, offset: usize) -> u8 {
        self.control[offset]
    }
    pub fn vram_byte(&self, offset: usize) -> u8 {
        self.vram[offset]
    }
    pub fn vram_bytes(&self) -> &[u8] {
        &self.vram
    }
    pub fn vram_word(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.vram[offset..offset + 4].try_into().unwrap())
    }
    pub fn control_word(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.control[offset..offset + 4].try_into().unwrap())
    }
    pub fn register_word(&self, offset: usize) -> u32 {
        self.sx_word(offset)
    }

    pub fn contains(&self, address: u64) -> bool {
        let ctl = (VSIMM_CTL_BASE..VSIMM_CTL_BASE + VSIMM_CTL_SIZE).contains(&address);
        let vram = (VSIMM_VRAM_BASE..VSIMM_VRAM_BASE + VRAM_APERTURE_BYTES).contains(&address);
        let commands = (SX_MEM_BASE..SX_MEM_BASE + SX_MEM_SIZE).contains(&address);
        let regs = (SX_BASE..SX_BASE + SX_SIZE).contains(&address);
        ctl || vram || commands || regs
    }

    fn aperture_offset(&self, address: u64) -> Option<usize> {
        let offset = address.checked_sub(VSIMM_VRAM_BASE)?;
        (offset < VRAM_APERTURE_BYTES).then_some(offset as usize)
    }

    fn vram_offsets(offset: usize, bytes: u64) -> Option<([usize; 4], bool)> {
        let count = usize::try_from(bytes).ok()?;
        if count == 0 || count > 4 {
            return None;
        }
        let mut lanes = [0usize; 4];
        if offset < 0x0100_0000 {
            for (index, lane) in lanes.iter_mut().take(count).enumerate() {
                *lane = (offset + index) % VRAM_BYTES;
            }
            Some((lanes, false))
        } else if offset < 0x0200_0000 {
            for (index, lane) in lanes.iter_mut().take(count).enumerate() {
                *lane = ((offset - 0x0100_0000) + index) % VRAM_BYTES;
            }
            Some((lanes, true))
        } else if offset < 0x0300_0000 {
            let plane = usize::from((offset >> 23) & 1 != 0);
            let base = (((offset - 0x0200_0000) & 0x007f_ffff) << 1) + plane;
            for (index, lane) in lanes.iter_mut().take(count).enumerate() {
                *lane = (base + index * 2) % VRAM_BYTES;
            }
            Some((lanes, false))
        } else if offset < 0x0400_0000 {
            let plane = (offset >> 22) & 3;
            let base = (((offset - 0x0300_0000) << 2) & (VRAM_BYTES - 1)) + plane;
            for (index, lane) in lanes.iter_mut().take(count).enumerate() {
                *lane = (base + index * 4) % VRAM_BYTES;
            }
            Some((lanes, false))
        } else {
            None
        }
    }

    fn read_vram(&self, offset: usize, bytes: u64) -> Result<u32, BusErrorKind> {
        if bytes != 4 && bytes != 2 && bytes != 1 {
            return Err(BusErrorKind::Unsupported("SX access width"));
        }
        if offset as u64 + bytes > VRAM_APERTURE_BYTES {
            return Err(BusErrorKind::Unmapped);
        }
        if offset % bytes as usize != 0 {
            let mut value = 0u32;
            for index in 0..bytes {
                value = (value << 8) | self.read_vram(offset + index as usize, 1)?;
            }
            return Ok(value);
        }
        if offset < 0x0200_0000 {
            return self.read_bytes(&self.vram, offset % VRAM_BYTES, bytes);
        }
        let (lanes, _) = Self::vram_offsets(offset, bytes)
            .ok_or(BusErrorKind::Unsupported("SX packed access"))?;
        let mut value = 0u32;
        for lane in lanes.iter().take(usize::try_from(bytes).unwrap()) {
            value = (value << 8) | u32::from(self.vram[*lane]);
        }
        Ok(value)
    }

    fn write_vram(&mut self, offset: usize, value: u32, mask: u32) -> Result<(), BusErrorKind> {
        let bytes = mask.count_ones().div_ceil(8) as usize;
        if bytes == 0 || bytes > 4 {
            return Err(BusErrorKind::Unsupported("SX write width"));
        }
        if offset as u64 + bytes as u64 > VRAM_APERTURE_BYTES {
            return Err(BusErrorKind::Unmapped);
        }
        if offset % bytes != 0 {
            for index in 0..bytes {
                let shift = (bytes - 1 - index) * 8;
                let byte_mask = (mask >> shift) & 0xff;
                if byte_mask == 0 {
                    continue;
                }
                let old = self.read_vram(offset + index, 1)?;
                let value = (old & !byte_mask) | (((value >> shift) & 0xff) & byte_mask);
                self.write_vram(offset + index, value, byte_mask)?;
            }
            return Ok(());
        }
        if offset < 0x0100_0000 {
            Self::write_bytes(&mut self.vram, offset % VRAM_BYTES, value, mask);
            return Ok(());
        }
        if offset < 0x0200_0000 {
            let base = (offset - 0x0100_0000) % VRAM_BYTES;
            let lane = offset & 3;
            if lane + bytes > 4 {
                return Err(BusErrorKind::Unmapped);
            }
            let shift = (4 - lane - bytes) * 8;
            let raw = (value << shift).to_be_bytes();
            let mask = (mask << shift).to_be_bytes();
            for index in 0..bytes {
                if (base + index) & 3 == 0 || mask[lane + index] == 0 {
                    continue;
                }
                self.vram[(base + index) % VRAM_BYTES] = raw[lane + index];
            }
            return Ok(());
        }
        let (lanes, blocked) = Self::vram_offsets(offset, bytes as u64)
            .ok_or(BusErrorKind::Unsupported("SX packed access"))?;
        let shift = (4 - bytes) * 8;
        let raw = (value << shift).to_be_bytes();
        let mask = (mask << shift).to_be_bytes();
        for index in 0..bytes {
            if blocked && (lanes[index] & 3) == 0 {
                continue;
            }
            if mask[index] == 0 {
                continue;
            }
            self.vram[lanes[index]] = raw[index];
        }
        Ok(())
    }

    fn sx_word(&self, offset: usize) -> u32 {
        let offset = offset & 0x1fff;
        let offset = if offset >= SX_USER_PAGE {
            offset - SX_USER_PAGE
        } else {
            offset
        };
        match offset {
            SX_CONTROL_STATUS => self.status | if self.go { SX_GO } else { 0 },
            SX_ERROR => self.error,
            SX_PAGE_BOUND_LOWER => self.page_bound_lower,
            SX_PAGE_BOUND_UPPER => self.page_bound_upper,
            SX_PLANEMASK => self.plane_mask,
            SX_ROP_CONTROL => self.rop_control,
            SX_IQ_OVERFLOW => self.overflow,
            SX_DIAGNOSTICS => self.diagnostics,
            SX_ID => 1,
            SX_R0_INIT => self.regs[0],
            SX_DIRECT_R0..=SX_DIRECT_END => self.regs[(offset - SX_DIRECT_R0) / 4],
            SX_QUEUED_R0..=SX_QUEUED_END => self.regs[(offset - SX_QUEUED_R0) / 4],
            _ => 0,
        }
    }

    fn read_sx(&self, offset: usize, bytes: u64) -> Result<u32, BusErrorKind> {
        if bytes != 4 && bytes != 2 && bytes != 1 {
            return Err(BusErrorKind::Unsupported("SX access width"));
        }
        if offset % bytes as usize != 0 || offset + bytes as usize > SX_SIZE as usize {
            return Err(BusErrorKind::Unmapped);
        }
        let word = self.sx_word(offset & !3);
        let shift = (4 - (offset & 3) - bytes as usize) * 8;
        let value = word >> shift;
        Ok(if bytes == 4 {
            value
        } else {
            value & ((1u32 << (bytes * 8)) - 1)
        })
    }

    fn write_sx(&mut self, offset: usize, value: u32, mask: u32) -> Result<(), BusErrorKind> {
        if offset + 4 > SX_SIZE as usize {
            return Err(BusErrorKind::Unmapped);
        }
        let user = offset >= SX_USER_PAGE;
        let offset = if user { offset - SX_USER_PAGE } else { offset };
        let bytes = mask.count_ones().div_ceil(8) as usize;
        if bytes == 0 || bytes > 4 || offset & 3 != 0 {
            return Err(BusErrorKind::Unsupported("SX register access width"));
        }
        let value = if mask == u32::MAX {
            value
        } else {
            let shift = (4 - (offset & 3) - bytes) * 8;
            let old = self.sx_word(offset);
            (old & !(mask << shift)) | ((value << shift) & (mask << shift))
        };
        let user_writable = matches!(
            offset,
            SX_PLANEMASK | SX_ROP_CONTROL | SX_INSTRUCTIONS | SX_SYNC
        ) || (SX_QUEUED_R0..=SX_QUEUED_END).contains(&offset);
        if user && !user_writable {
            return Err(BusErrorKind::ReadOnly);
        }
        match offset {
            SX_CONTROL_STATUS => {
                self.go = value & SX_GO != 0;
                self.status = value & !SX_GO;
            }
            SX_ERROR => self.error = value,
            SX_PAGE_BOUND_LOWER => self.page_bound_lower = value,
            SX_PAGE_BOUND_UPPER => self.page_bound_upper = value,
            SX_PLANEMASK => self.plane_mask = value,
            SX_ROP_CONTROL => self.rop_control = value & 0xff,
            SX_IQ_OVERFLOW => self.overflow = value,
            SX_DIAGNOSTICS => self.diagnostics = value,
            SX_INSTRUCTIONS => {
                if self.go {
                    self.execute_instruction(value)?;
                }
            }
            SX_ID => {}
            SX_R0_INIT => self.regs[0] = value,
            SX_SOFTRESET => {
                self.regs = [0; 128];
                self.error = 0;
                self.status = SX_MT;
                self.go = false;
            }
            SX_SYNC => {}
            SX_DIRECT_R0..=SX_DIRECT_END => {
                self.regs[(offset - SX_DIRECT_R0) / 4] = value;
            }
            SX_QUEUED_R0..=SX_QUEUED_END => {
                self.regs[(offset - SX_QUEUED_R0) / 4] = value;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusErrorKind> {
        if (SX_BASE..SX_BASE + SX_SIZE).contains(&address) {
            let offset = (address - SX_BASE) as usize;
            if offset == SX_INSTRUCTIONS || offset == SX_USER_PAGE + SX_INSTRUCTIONS {
                if address & 7 != 0 {
                    return Err(BusErrorKind::Unmapped);
                }
                if !self.go {
                    return Ok(());
                }
                let saved_regs = self.regs;
                let saved_error = self.error;
                let saved_status = self.status;
                let saved_rop = self.rop_control;
                let result = self
                    .execute_instruction(words[0])
                    .and_then(|()| self.execute_instruction(words[1]));
                if result.is_err() {
                    self.regs = saved_regs;
                    self.error = saved_error;
                    self.status = saved_status;
                    self.rop_control = saved_rop;
                } else {
                    self.generation = self.generation.wrapping_add(1);
                }
                return result;
            }
        }
        self.write(address, words[0], u32::MAX)?;
        self.write(address + 4, words[1], u32::MAX)
    }

    fn execute_instruction(&mut self, instruction: u32) -> Result<(), BusErrorKind> {
        let top = instruction & 0xf000_0000;
        if top == 0x9000_0000 {
            let count = usize::from(((instruction >> 24) & 0xf) as u8) + 1;
            let op = (instruction >> 21) & 7;
            let a = ((instruction >> 14) & 0x7f) as usize;
            let b = (instruction & 0x7f) as usize;
            let d = ((instruction >> 7) & 0x7f) as usize;
            if op == 1 {
                if count > 16 || a + count > 128 || b + count > 128 || d + count > 128 {
                    return Err(BusErrorKind::Unsupported("SX ROP register range"));
                }
                let rop = self.rop_control & 0xff;
                let mut values = [0u32; 16];
                for (index, value) in values.iter_mut().take(count).enumerate() {
                    let lhs = self.regs[a + index];
                    let rhs = self.regs[b + index];
                    let mut result = 0u32;
                    for bit in 0..32 {
                        let select = (((lhs >> bit) & 1) << 1) | ((rhs >> bit) & 1);
                        let table = if (self.regs[2] >> bit) & 1 != 0 {
                            (rop >> 4) & 0xf
                        } else {
                            rop & 0xf
                        };
                        result |= ((table >> select) & 1) << bit;
                    }
                    *value = result;
                }
                self.regs[d..d + count].copy_from_slice(&values[..count]);
                return Ok(());
            }
            match op {
                6 => {
                    for index in 0..count {
                        self.regs[(d + index) & 127] =
                            self.regs[(a + index) & 127] | self.regs[(b + index) & 127];
                    }
                }
                7 => {
                    for index in 0..count {
                        self.regs[(d + index) & 127] = self.regs[(a + index) & 127] | self.regs[b];
                    }
                }
                _ => return Err(BusErrorKind::Unsupported("SX select instruction")),
            }
            return Ok(());
        }
        if top == 0xb000_0000 {
            let count = usize::from(((instruction >> 24) & 0xf) as u8) + 1;
            let op = (instruction >> 21) & 7;
            let a = ((instruction >> 14) & 0x7f) as usize;
            let b = (instruction & 0x7f) as usize;
            let d = ((instruction >> 7) & 0x7f) as usize;
            for index in 0..count {
                let (src, rhs) = match op {
                    0 | 3 | 6 => (self.regs[(a + index) & 127], self.regs[(b + index) & 127]),
                    1 | 4 | 7 => (self.regs[a], self.regs[(b + index) & 127]),
                    2 | 5 => (self.regs[(a + index) & 127], b as u32),
                    _ => return Err(BusErrorKind::Unsupported("SX logic instruction")),
                };
                self.regs[(d + index) & 127] = match op {
                    0..=2 => src & rhs,
                    3..=5 => src ^ rhs,
                    6 | 7 => src | rhs,
                    _ => return Err(BusErrorKind::Unsupported("SX logic instruction")),
                };
            }
            return Ok(());
        }
        if top == 0xe000_0000 {
            let op = (instruction >> 21) & 7;
            if op != 1 {
                return Err(BusErrorKind::Unsupported("SX plot instruction"));
            }
            let count = usize::from(((instruction >> 24) & 0xf) as u8) + 1;
            let a = ((instruction >> 14) & 0x7f) as usize;
            let b = (instruction & 0x7f) as usize;
            let d = ((instruction >> 7) & 0x7f) as usize;
            if count > 16 || (a + 1) & 127 != a + 1 || (b + 3) & 127 != b + 3 || d + count > 127 {
                return Err(BusErrorKind::Unsupported("SX plot register range"));
            }
            let mut base = self.regs[a] as i32;
            let mut error = self.regs[(a + 1) & 127] as i32;
            let major_offset = self.regs[b & 127] as i32;
            let major_error = self.regs[(b + 1) & 127] as i32;
            let minor_offset = self.regs[(b + 2) & 127] as i32;
            let minor_error = self.regs[(b + 3) & 127] as i32;
            let mut outputs = [0u32; 16];
            for output in outputs.iter_mut().take(count) {
                let (offset, error_delta) = if error < 0 {
                    (major_offset, major_error)
                } else {
                    (minor_offset, minor_error)
                };
                base = base.wrapping_add(offset);
                error = error.wrapping_add(error_delta);
                *output = base as u32;
            }
            self.regs[d..d + count].copy_from_slice(&outputs[..count]);
            self.regs[d + count] = error as u32;
            return Ok(());
        }
        if top == 0xc000_0000 {
            let count = usize::from(((instruction >> 24) & 0xf) as u8) + 1;
            let op = (instruction >> 21) & 7;
            let a = ((instruction >> 14) & 0x7f) as usize;
            let b = (instruction & 0x7f) as usize;
            let d = ((instruction >> 7) & 0x7f) as usize;
            if op >= 6 {
                if count > 16 || a + count + 1 > 128 || d + count > 128 {
                    return Err(BusErrorKind::Unsupported("SX funnel register range"));
                }
                let shift = if op == 6 { self.regs[b] } else { b as u32 };
                if shift >= 32 {
                    return Err(BusErrorKind::Unsupported("SX funnel shift count"));
                }
                let mut source = [0u32; 17];
                source[..=count].copy_from_slice(&self.regs[a..=a + count]);
                let mut outputs = [0u32; 16];
                for (index, output) in outputs.iter_mut().take(count).enumerate() {
                    *output = if shift == 0 {
                        source[index]
                    } else {
                        (source[index] << shift) | (source[index + 1] >> (32 - shift))
                    };
                }
                self.regs[d..d + count].copy_from_slice(&outputs[..count]);
                return Ok(());
            }
            for index in 0..count {
                let source = self.regs[(a + index) & 127];
                let shift = if op & 1 == 0 {
                    self.regs[b] & 31
                } else {
                    b as u32
                };
                let result = match op >> 1 {
                    0 => source >> shift,
                    1 => ((source as i32) >> shift) as u32,
                    2 => source << shift,
                    _ => return Err(BusErrorKind::Unsupported("SX shift instruction")),
                };
                self.regs[(d + index) & 127] = result;
            }
            return Ok(());
        }
        Err(BusErrorKind::Unsupported("SX instruction"))
    }

    fn execute_memory_command(
        &mut self,
        address: u64,
        instruction: u32,
    ) -> Result<(), BusErrorKind> {
        let op = (instruction >> 19) & 0xf;
        let data_type = (instruction >> 14) & 0x1f;
        let source = ((instruction >> 7) & 0x7f) as usize;
        let count = usize::from(((instruction >> 23) & 0x1f) as u8) + 1;
        let displacement = (instruction & 0x7f) as u64;
        if op == 0xa {
            let target = ((address - SX_MEM_BASE) & !7) + displacement;
            if data_type == 0x1f {
                let start_lane = (target & 3) as usize;
                let registers = (start_lane + count).div_ceil(4);
                if count > 32 || source + registers > self.regs.len() {
                    return Err(BusErrorKind::Unsupported("SX packed load register range"));
                }
                let mut values = [0u32; 9];
                for index in 0..count {
                    let offset = self
                        .aperture_offset(target + index as u64)
                        .ok_or(BusErrorKind::Unsupported("SX command target"))?;
                    if offset as u64 + 1 > VRAM_APERTURE_BYTES {
                        return Err(BusErrorKind::Unsupported("SX command target width"));
                    }
                    let position = start_lane + index;
                    let word = &mut values[position / 4];
                    *word |= self.read_vram(offset, 1)? << (24 - (position % 4) * 8);
                }
                self.regs[source..source + registers].copy_from_slice(&values[..registers]);
                return Ok(());
            }
            let width = match data_type {
                0 => 1,
                0x1b => 4,
                _ => return Err(BusErrorKind::Unsupported("SX load format")),
            };
            if count > 32 || source + count > self.regs.len() {
                return Err(BusErrorKind::Unsupported("SX load register range"));
            }
            let mut values = [0u32; 32];
            for (index, value) in values.iter_mut().take(count).enumerate() {
                let offset = self
                    .aperture_offset(target + (index * width) as u64)
                    .ok_or(BusErrorKind::Unsupported("SX command target"))?;
                if offset as u64 + width as u64 > VRAM_APERTURE_BYTES {
                    return Err(BusErrorKind::Unsupported("SX command target width"));
                }
                *value = self.read_vram(offset, width as u64)?;
            }
            self.regs[source..source + count].copy_from_slice(&values[..count]);
            return Ok(());
        }
        let select = op & 8 != 0;
        let masked = op & 1 != 0;
        if data_type == 0x1f {
            if !matches!(op, 1 | 5 | 9) {
                return Err(BusErrorKind::Unsupported("SX packed memory command"));
            }
            let target = ((address - SX_MEM_BASE) & !7) + displacement;
            let mut offsets = [0usize; 32];
            if count > offsets.len() {
                return Err(BusErrorKind::Unsupported("SX packed memory count"));
            }
            for (index, offset) in offsets.iter_mut().take(count).enumerate() {
                *offset = self
                    .aperture_offset(target + index as u64)
                    .ok_or(BusErrorKind::Unsupported("SX command target"))?;
            }
            let source_lane = if op == 1 { (target & 3) as usize } else { 0 };
            if op == 1 && source + (source_lane + count - 1) / 4 >= 128 {
                return Err(BusErrorKind::Unsupported("SX packed store register range"));
            }
            for (index, offset) in offsets.iter().take(count).enumerate() {
                let enable = (self.regs[2] >> (31 - (index & 31))) & 1 != 0;
                if op == 5 && !enable {
                    continue;
                }
                let source_index = source_lane + index;
                let source_register = if op == 9 {
                    source + usize::from(enable)
                } else if op == 1 {
                    source + source_index / 4
                } else {
                    source + index / 4
                };
                let source_word = self.regs[source_register & 127];
                let value = (source_word >> ((3 - (source_index & 3)) * 8)) & 0xff;
                let mask = (self.plane_mask >> (((source_lane + index) & 3) * 8)) & 0xff;
                let old = self.read_vram(*offset, 1)?;
                let value = (old & !mask) | (value & mask);
                self.write_vram(*offset, value, 0xff)?;
            }
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        let width = match data_type {
            0..=0x07 | 0x10..=0x17 => 1,
            0x18..=0x1a => 2,
            0x1b => 4,
            _ => return Err(BusErrorKind::Unsupported("SX memory command type")),
        };
        if !matches!(op & 0xf, 0 | 1 | 8 | 9) {
            return Err(BusErrorKind::Unsupported("SX memory command"));
        }
        let array = width == 1 && select && source >= 32 && count > 1;
        let target = ((address - SX_MEM_BASE) & !7) + displacement;
        let mut offsets = [0usize; 32];
        if count > offsets.len() {
            return Err(BusErrorKind::Unsupported("SX memory command count"));
        }
        for (index, offset) in offsets.iter_mut().take(count).enumerate() {
            let physical = if array {
                let byte_offset = self.regs[(displacement as usize + index) & 127] as i32;
                ((address - SX_MEM_BASE) as i64)
                    .wrapping_add(i64::from(byte_offset))
                    .try_into()
                    .map_err(|_| BusErrorKind::Unsupported("SX command target"))?
            } else {
                target + (index * width) as u64
            };
            *offset = self
                .aperture_offset(physical)
                .ok_or(BusErrorKind::Unsupported("SX command target"))?;
            if *offset as u64 + width as u64 > VRAM_APERTURE_BYTES {
                return Err(BusErrorKind::Unsupported("SX command target width"));
            }
        }
        let width_mask = if width == 4 {
            u32::MAX
        } else {
            (1u32 << (width * 8)) - 1
        };
        let source_mask = if masked {
            self.plane_mask & width_mask
        } else {
            width_mask
        };
        for (index, offset) in offsets.iter().take(count).enumerate() {
            let selected = if array || select {
                let selector = ((self.regs[2] >> (31 - (index & 31))) & 1) as usize;
                self.regs[(source + selector) & 127]
            } else {
                self.regs[(source + index) & 127]
            };
            let value = selected & width_mask;
            let value = if masked {
                let old = self.read_vram(*offset, width as u64)?;
                (old & !source_mask) | (value & source_mask)
            } else {
                value
            };
            self.write_vram(*offset, value, width_mask)?;
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub fn read(&mut self, address: u64, bytes: u64) -> Result<u32, BusErrorKind> {
        if let Some(offset) = self.aperture_offset(address) {
            return self.read_vram(offset, bytes);
        }
        if (SX_BASE..SX_BASE + SX_SIZE).contains(&address) {
            return self.read_sx((address - SX_BASE) as usize, bytes);
        }
        if (VSIMM_CTL_BASE..VSIMM_CTL_BASE + VSIMM_CTL_SIZE).contains(&address) {
            let offset = (address - VSIMM_CTL_BASE) as usize;
            let mut value = self.read_bytes(&self.control[..], offset, bytes)?;
            if offset <= 4 && offset + bytes as usize > 4 {
                let shift = (offset + bytes as usize - 5) * 8;
                value = (value & !(0xff << shift)) | (u32::from(self.msr()) << shift);
            }
            return Ok(value);
        }
        Err(BusErrorKind::Unmapped)
    }

    pub fn write(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusErrorKind> {
        if (SX_MEM_BASE..SX_MEM_BASE + SX_MEM_SIZE).contains(&address) {
            if self.trace {
                eprintln!(
                    "SX CMD PA {:#010x} <- {value:#010x} mask {mask:#010x}",
                    address - SX_MEM_BASE
                );
            }
            return self.execute_memory_command(address, value);
        }
        if let Some(offset) = self.aperture_offset(address) {
            if self.trace && self.fb_trace < 80 {
                eprintln!("CG14 FB PA {address:#010x} <- {value:#010x} mask {mask:#010x}");
                self.fb_trace += 1;
            }
            self.write_vram(offset, value, mask)?;
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        if (SX_BASE..SX_BASE + SX_SIZE).contains(&address) {
            let offset = (address - SX_BASE) as usize;
            if self.trace {
                eprintln!("SX REG {offset:#06x} <- {value:#010x} mask {mask:#010x}");
            }
            self.write_sx(offset, value, mask)?;
            self.generation = self.generation.wrapping_add(1);
            return Ok(());
        }
        if (VSIMM_CTL_BASE..VSIMM_CTL_BASE + VSIMM_CTL_SIZE).contains(&address) {
            let offset = (address - VSIMM_CTL_BASE) as usize;
            if offset == 4 {
                self.retrace_pending = false;
                self.generation = self.generation.wrapping_add(1);
                return Ok(());
            }
            if self.table_stream(offset, value, mask) {
                if self.trace {
                    eprintln!("CG14 STREAM {offset:#06x} <- {value:#010x} mask {mask:#010x}");
                }
                self.generation = self.generation.wrapping_add(1);
                return Ok(());
            }
            Self::write_bytes(&mut self.control[..], offset, value, mask);
            // Revision and VBC configuration/current-row status are read-only.
            // The functional model does not yet advance the VBC row counter.
            self.control[CG14_RSR] = CG14_REVISION;
            self.control[VBC_VCA..VBC_VCA + 4].copy_from_slice(&VBC_STRAPS.to_be_bytes());
            if self.trace
                && (offset < 0x100
                    || (0x1000..0x1120).contains(&offset)
                    || (0x2000..0x2400).contains(&offset)
                    || (0x3000..0x3100).contains(&offset))
            {
                eprintln!("CG14 REG {offset:#06x} <- {value:#010x} mask {mask:#010x}");
            }
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
        if self.control[0] & 0x41 != 0x41 {
            output.fill(0);
            return;
        }
        let mode = (self.control[0] >> 4) & 3;
        for (index, pixel) in output.iter_mut().enumerate() {
            let rgb = match mode {
                0 => self.clut_rgb(0, usize::from(self.vram[index])),
                2 => {
                    let raw = index * 2;
                    let x = usize::from(self.vram[raw]);
                    let c = u32::from(self.vram[raw + 1]);
                    if self.control[0x3000 + x] == 0x40 {
                        self.clut_rgb(0, x)
                    } else {
                        (u32::try_from(x).unwrap() << 8) | c
                    }
                }
                _ => {
                    let raw = index * 4;
                    let x = usize::from(self.vram[raw]);
                    let b = u32::from(self.vram[raw + 1]);
                    let g = u32::from(self.vram[raw + 2]);
                    let r = u32::from(self.vram[raw + 3]);
                    self.routed_rgb(x, b, g, r)
                }
            };
            *pixel = rgb;
            if let Some(cursor) = self.cursor_pixel(index % WIDTH, index / WIDTH) {
                *pixel = cursor;
            }
        }
    }

    fn clut_rgb(&self, table: usize, color: usize) -> u32 {
        let off = CG14_CLUT1 + table * 0x1000 + color * 4;
        let rgb = u32::from_be_bytes(self.control[off..off + 4].try_into().unwrap());
        ((rgb & 0xff) << 16) | (rgb & 0x0000_ff00) | ((rgb >> 16) & 0xff)
    }

    fn routed_rgb(&self, x: usize, b: u32, g: u32, r: u32) -> u32 {
        let xlut = self.control[0x3000 + x];
        let table = match xlut & 0xc0 {
            0x40 => Some(0),
            0x80 => Some(1),
            0xc0 => Some(2),
            _ => None,
        };
        let index = match xlut & 0x0c {
            0x04 => b as usize,
            0x08 => g as usize,
            0x0c => r as usize,
            _ => x,
        };
        if let Some(table) = table {
            self.clut_rgb(table, index)
        } else {
            (r << 16) | (g << 8) | b
        }
    }

    fn cursor_pixel(&self, x: usize, y: usize) -> Option<u32> {
        let control = self.control[0x1100];
        if control & 0x04 == 0 {
            return None;
        }
        let position = u32::from_be_bytes(self.control[0x1104..0x1108].try_into().unwrap());
        let cx = i64::from(position >> 16);
        let cy = i64::from(position & 0xffff);
        let dx = x as i64 - cx;
        let dy = y as i64 - cy;
        if !(0..32).contains(&dx) || !(0..32).contains(&dy) {
            return None;
        }
        let row = dy as usize;
        let bit = 31 - dx as u32;
        let p0 = u32::from_be_bytes(
            self.control[0x1000 + row * 4..0x1004 + row * 4]
                .try_into()
                .unwrap(),
        );
        let p1 = u32::from_be_bytes(
            self.control[0x1080 + row * 4..0x1084 + row * 4]
                .try_into()
                .unwrap(),
        );
        let index = ((p0 >> bit) & 1) | (((p1 >> bit) & 1) << 1);
        let color = match index {
            1 => u32::from_be_bytes(self.control[0x1108..0x110c].try_into().unwrap()),
            2 => u32::from_be_bytes(self.control[0x110c..0x1110].try_into().unwrap()),
            _ => return None,
        };
        Some(color & 0x00ff_ffff)
    }
}
