use super::serial::Serial;
use super::{BootFlash, FlashFetch, ResetNvram, FLASH_BASE};
use crate::v9::{V9Cpu, V9DataBus, PHYSICAL_ADDRESS_MASK};

const PCI_CONFIG: u64 = 0x01fe_0100_0000;
const PCI_MEMORY: u64 = 0x01ff_0000_0000;
const EBUS_BAR1: u32 = 0xf100_0000;
const SIO_INDEX: u64 = 0x30_0398;
const SIO_DATA: u64 = SIO_INDEX + 1;
const AUXIO_LED: u64 = PCI_MEMORY + EBUS_BAR1 as u64 + 0x72_6000;
const MEM_CONTROL0: u64 = 0x01fe_0000_f010;
const MEM_CONTROL1: u64 = 0x01fe_0000_f018;

#[derive(Default)]
struct BusNumbers {
    primary: u8,
    secondary: u8,
    subordinate: u8,
}

/// The Ultra 5 reset-time bus. Flash supplies both instructions and the
/// bytes in OBP's serial initialization table. All devices persist across steps.
pub struct Ultra5ResetBoard {
    flash: BootFlash,
    nvram: ResetNvram,
    host_primary: u8,
    host_subordinate: u8,
    apb: [BusNumbers; 2],
    ebus_setup: [u32; 3],
    front_panel_led: bool,
    mem_control0: u32,
    mem_control1: u32,
    super_io: SuperIoConfig,
    serial: Serial,
}

impl Ultra5ResetBoard {
    pub fn new(flash: BootFlash, nvram: ResetNvram) -> Self {
        Self {
            flash,
            nvram,
            host_primary: 0,
            host_subordinate: 0,
            apb: Default::default(),
            ebus_setup: [0; 3],
            front_panel_led: false,
            // IIi tables 18-3 and 18-7: DIMM pairs, refresh interval and DRAM timings.
            mem_control0: 0x0f30,
            mem_control1: 0x004a_2950,
            super_io: SuperIoConfig::default(),
            serial: Serial::default(),
        }
    }

    pub fn step(&mut self, cpu: &mut V9Cpu) -> Result<FlashFetch, String> {
        let fetch = self.flash.fetch_reset_word(cpu.pc(), cpu.npc())?;
        cpu.execute_fetched(fetch.instruction, self)?;
        Ok(fetch)
    }

    /// SAB 82532 channel A/B configuration, indexed in board address order.
    pub fn serial_ccr0(&self) -> [u8; 2] {
        self.serial.ccr0()
    }

    /// Drain bytes actually transmitted by the guest, channel A followed by B.
    pub fn take_serial_output(&mut self) -> [Vec<u8>; 2] {
        self.serial.take_output()
    }

    fn ebus_offset(pa: u64) -> Option<u64> {
        pa.checked_sub(PCI_MEMORY + u64::from(EBUS_BAR1))
            .filter(|&off| off < 0x80_0000)
    }

    fn config_byte(&mut self, offset: u64) -> Result<&mut u8, String> {
        if offset == 0x40 {
            return Ok(&mut self.host_primary);
        }
        if offset == 0x41 {
            return Ok(&mut self.host_subordinate);
        }
        match offset & 0xffff {
            0x818..=0x81a | 0x918..=0x91a if offset >> 16 == u64::from(self.host_primary) => {
                let bridge = &mut self.apb[((offset >> 8) & 1) as usize];
                Ok(match offset & 0xff {
                    0x18 => &mut bridge.primary,
                    0x19 => &mut bridge.secondary,
                    _ => &mut bridge.subordinate,
                })
            }
            _ => Err(format!(
                "unmapped Ultra 5 PCI configuration byte {offset:#x}"
            )),
        }
    }

    fn pcio_bar1_at(&self, offset: u64) -> bool {
        let bridge = &self.apb[1];
        let bus = bridge.secondary;
        bus > self.host_primary
            && bridge.primary == self.host_primary
            && bus <= self.host_subordinate
            && bus <= bridge.subordinate
            && offset == (u64::from(bus) << 16) + 0x814
    }
}

// National Super I/O configuration protocol (§2.2). Only the boot-time
// configuration latches are modeled. The keyboard/mouse UARTs are separate
// from the Siemens A/B console ports.
#[derive(Default)]
struct SuperIoConfig {
    accessed_index: bool,
    index: u8,
    pending: Option<u8>,
    registers: [Option<u8>; 8],
}

impl SuperIoConfig {
    fn read_index(&mut self) -> u8 {
        self.pending = None;
        if std::mem::replace(&mut self.accessed_index, true) {
            self.index
        } else {
            0x88
        }
    }

    fn select(&mut self, value: u8) -> Result<(), String> {
        let index = value & 0x8f; // index bits 6:4 read zero
        if !matches!(index, 0..=4 | 6..=7) {
            return Err(format!("unsupported Ultra 5 Super I/O index {index:#x}"));
        }
        self.accessed_index = true;
        self.index = index;
        self.pending = None;
        Ok(())
    }

    fn write_data(&mut self, value: u8) {
        if self.pending == Some(value) {
            self.registers[self.index as usize] = Some(value);
            self.pending = None;
        } else {
            self.pending = Some(value);
        }
    }
}

impl V9DataBus for Ultra5ResetBoard {
    fn load_byte(&mut self, va: u64) -> Result<u8, String> {
        if !matches!((va as i64) >> 43, 0 | -1) {
            return Err(format!("out-of-range Ultra 5 LDUB at {va:#x}"));
        }
        self.read_bypass_byte(va & PHYSICAL_ADDRESS_MASK)
    }
    fn write_bypass_xword(&mut self, pa: u64, value: u64) -> Result<(), String> {
        match pa {
            MEM_CONTROL0 => {
                // SME1430 RefEnable, ECCEnable, board timing, geometry and refresh fields.
                self.mem_control0 = value as u32 & 0x9e00_1fff;
                return Ok(());
            }
            MEM_CONTROL1 => {
                self.mem_control1 = value as u32;
                return Ok(());
            }
            _ => {}
        }
        let i = self.nvram.sram_index(pa, 8)?;
        self.nvram.image[i..i + 8].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    fn read_bypass_word(&mut self, pa: u64) -> Result<u32, String> {
        if pa == AUXIO_LED {
            // PCIO bit 0 is in the first byte on the little-endian EBus interface.
            return Ok(u32::from(self.front_panel_led).swap_bytes());
        }
        let i = self.nvram.sram_index(pa, 4)?;
        Ok(u32::from_be_bytes(
            self.nvram.image[i..i + 4].try_into().unwrap(),
        ))
    }
    fn write_bypass_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        if pa == AUXIO_LED {
            self.front_panel_led = value.swap_bytes() & 1 != 0;
            return Ok(());
        }
        if !matches!(Self::ebus_offset(pa), Some(0..=0x0f_ffff)) {
            return Err(format!(
                "unmapped Ultra 5 physical word write at {pa:#x} value={value:#010x}"
            ));
        }
        let i = self.nvram.sram_index(pa, 4)?;
        self.nvram.image[i..i + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    fn read_bypass_xword(&mut self, pa: u64) -> Result<u64, String> {
        match pa {
            MEM_CONTROL0 => return Ok(u64::from(self.mem_control0)),
            MEM_CONTROL1 => return Ok(u64::from(self.mem_control1)),
            // A freshly constructed board starts from SYS_RESET_L (IIi table 17-2).
            0x01fe_0000_f020 => return Ok(1 << 31),
            _ => {}
        }
        if matches!(Self::ebus_offset(pa), Some(0..=0x0f_ffff)) {
            return self.nvram.read_bypass_xword(pa);
        }
        Err(format!("unmapped Ultra 5 physical xword read at {pa:#x}"))
    }

    fn read_bypass_byte(&mut self, pa: u64) -> Result<u8, String> {
        if let Some(offset) = pa.checked_sub(PCI_CONFIG).filter(|&off| off < 1 << 24) {
            return self.config_byte(offset).map(|register| *register);
        }
        if let Some(off) = pa
            .checked_sub(FLASH_BASE)
            .filter(|&off| off < self.flash.image().len() as u64)
        {
            return Ok(self.flash.image()[off as usize]);
        }
        match Self::ebus_offset(pa) {
            Some(0..=0x0f_ffff) => Ok(self.nvram.image[self.nvram.sram_index(pa, 1)?]),
            Some(SIO_INDEX) => Ok(self.super_io.read_index()),
            Some(SIO_DATA) => {
                let value =
                    self.super_io.registers[self.super_io.index as usize].ok_or_else(|| {
                        format!(
                            "unmodeled Super I/O reset value for index {}",
                            self.super_io.index
                        )
                    })?;
                self.super_io.pending = None;
                Ok(value)
            }
            Some(off @ 0x40_0000..=0x40_007f) => self.serial.read((off - 0x40_0000) as u8),
            _ => Err(format!("unmapped Ultra 5 physical byte read at {pa:#x}")),
        }
    }

    fn write_bypass_byte(&mut self, pa: u64, value: u8) -> Result<(), String> {
        if let Some(offset) = pa.checked_sub(PCI_CONFIG).filter(|&off| off < 1 << 24) {
            *self.config_byte(offset)? = value;
            return Ok(());
        }
        match Self::ebus_offset(pa) {
            Some(SIO_INDEX) => self.super_io.select(value)?,
            Some(SIO_DATA) => self.super_io.write_data(value),
            Some(off @ 0x40_0000..=0x40_007f) => {
                self.serial.write((off - 0x40_0000) as u8, value)?
            }
            _ => {
                return Err(format!(
                    "unmapped Ultra 5 physical byte write at {pa:#x} value={value:#04x}"
                ))
            }
        }
        Ok(())
    }

    fn write_bypass_le_word(&mut self, pa: u64, value: u32) -> Result<(), String> {
        if pa == AUXIO_LED {
            return self.write_bypass_word(pa, value.swap_bytes());
        }
        if pa
            .checked_sub(PCI_CONFIG)
            .is_some_and(|offset| self.pcio_bar1_at(offset))
        {
            // Scope the fixture to the board's reset aperture. BAR sizing and
            // relocation require additional bridge decoding and are explicit stops.
            if value != EBUS_BAR1 {
                return Err(format!("unsupported Ultra 5 PCIO BAR1 value {value:#x}"));
            }
            return Ok(());
        }
        match Self::ebus_offset(pa) {
            Some(off @ (0x71_0000 | 0x71_0004 | 0x71_0008)) => {
                self.ebus_setup[((off - 0x71_0000) / 4) as usize] = value;
                Ok(())
            }
            _ => Err(format!(
                "unmapped Ultra 5 physical LE word write at {pa:#x} value={value:#x}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ultra5::{FLASH_SIZE, RESET_NVRAM_SIZE};

    fn fixture() -> Ultra5ResetBoard {
        Ultra5ResetBoard::new(
            BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
            ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
        )
    }

    #[test]
    fn auxio_led_obeys_pci_byte_order_and_only_decodes_its_word() {
        let mut board = fixture();
        let led = 0x01ff_f172_6000;
        assert_eq!(board.read_bypass_word(led).unwrap(), 0);
        board.write_bypass_word(led, 0x0100_0000).unwrap();
        assert!(board.front_panel_led);
        assert_eq!(board.read_bypass_word(led).unwrap(), 0x0100_0000);
        board.write_bypass_word(led, 1).unwrap();
        assert!(!board.front_panel_led);
        board.write_bypass_le_word(led, 0xfefe_fe01).unwrap();
        assert!(board.front_panel_led);
        assert!(board.write_bypass_byte(led, 0).is_err());
        assert!(board.write_bypass_xword(led, 0).is_err());
        assert!(board.write_bypass_word(led + 4, 0).is_err());
        assert!(board.write_bypass_word(led - 0x2000, 0).is_err());
        assert!(board.front_panel_led);
        let mut cpu = V9Cpu::from(crate::ultra5::PowerOnReset::ultra_sparc_ii());
        cpu.write_reg(1, led);
        cpu.execute_fetched(0xc480_43a0, &mut board).unwrap(); // lduwa [%g1] ASI 0x1d,%g2
        assert_eq!(cpu.read_reg(2), 1);
        cpu.execute_fetched(0xc480_42a0, &mut board).unwrap(); // lduwa [%g1] ASI 0x15,%g2
        assert_eq!(cpu.read_reg(2), 0x0100_0000);
    }

    #[test]
    fn mcu_registers_retain_writable_fields_independently() {
        let mut board = fixture();
        let config = 0x01fe_0000_f010;
        let timing = config + 8;
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0xf30);
        assert_eq!(board.read_bypass_xword(timing).unwrap(), 0x4a2950);
        board.write_bypass_xword(timing, u64::MAX).unwrap();
        assert_eq!(
            board.read_bypass_xword(timing).unwrap(),
            u64::from(u32::MAX)
        );
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0xf30);
        board.write_bypass_xword(config, u64::MAX).unwrap();
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0x9e00_1fff);
        assert!(board.write_bypass_word(config, 0).is_err());
        assert!(board.write_bypass_xword(config + 4, 0).is_err());
        board.write_bypass_xword(timing, 0).unwrap();
        assert_eq!(board.read_bypass_xword(config).unwrap(), 0x9e00_1fff);
        assert_eq!(board.read_bypass_xword(timing).unwrap(), 0);
    }

    #[test]
    #[ignore = "requires the locally reconstructed OBP 3.31.0 flash"]
    fn original_obp_sets_refresh_and_returns_to_the_ecache_probe() {
        let image = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
        ))
        .unwrap();
        let flash = BootFlash::new(image).unwrap();
        for marker in [[0; 8], *b"soft-rst"] {
            for diagnostics in [false, true] {
                let mut seed = vec![0; RESET_NVRAM_SIZE];
                seed[1] = u8::from(diagnostics);
                seed[0x10..0x18].copy_from_slice(&marker);
                let mut board =
                    Ultra5ResetBoard::new(flash.clone(), ResetNvram::new(seed).unwrap());
                let reset = crate::ultra5::PowerOnReset::ultra_sparc_ii();
                let base = reset.pc - 0x20;
                let mut cpu = V9Cpu::from(reset);
                let mut polls = [0; 2];
                let mut writes = Vec::new();
                let mut retired = 0;
                while cpu.pc() != base + 0x714 && retired < 8_000 {
                    let pc = cpu.pc();
                    if pc == base + 0x14c0 || pc == base + 0x14f8 {
                        assert_eq!(cpu.read_reg(2), 0x01ff_f172_6000);
                        assert!(board.front_panel_led);
                        polls[usize::from(pc == base + 0x14f8)] += 1;
                    }
                    if pc == base + 0xfc && cpu.current_window() == 1 {
                        writes.push((cpu.read_reg(10) + cpu.read_reg(8), cpu.read_reg(9)));
                    }
                    board.step(&mut cpu).unwrap();
                    retired += 1;
                    if pc == base + 0x14fc {
                        assert_eq!(
                            (cpu.current_window(), cpu.pc(), cpu.npc()),
                            (0, base + 0x1500, base + 0x1908)
                        );
                    }
                }
                assert_eq!(
                    (cpu.pc(), cpu.npc(), cpu.read_reg(9)),
                    (base + 0x714, base + 0x718, 1 << 39)
                );
                assert_eq!(
                    writes,
                    [
                        (0x01fe_0000_f018, 0x0c4a_ab14),
                        (0x01fe_0000_f010, 0x8000_053d)
                    ]
                );
                assert_eq!(
                    (board.mem_control0, board.mem_control1),
                    (0x8000_053d, 0x0c4a_ab14)
                );
                assert_eq!(polls, [1_000, 1_000]);
                let [ttya, ttyb] = board.take_serial_output();
                let line: &[u8] = match (diagnostics, marker == *b"soft-rst") {
                    (false, _) => b"",
                    (true, true) => b"Software Power ON\r\n",
                    (true, false) => b"Hardware Power ON\r\n",
                };
                assert_eq!(ttya, line);
                assert!(ttyb.is_empty());
                println!(
                    "marker={marker:?} diagnostics={diagnostics} retired={retired} polls={polls:?}"
                );
            }
        }
    }

    #[test]
    fn bar_setup_needs_host_and_onboard_apb_routing() {
        let mut board = fixture();
        let bar = PCI_CONFIG + 0x10814;
        let write_bar = |b: &mut Ultra5ResetBoard| b.write_bypass_le_word(bar, EBUS_BAR1);
        assert!(write_bar(&mut board).is_err());
        for (offset, value) in [
            (0x40, 0),
            (0x818, 0),
            (0x819, 2),
            (0x81a, 0xff),
            (0x918, 0),
            (0x919, 1),
            (0x91a, 1),
        ] {
            board.write_bypass_byte(PCI_CONFIG + offset, value).unwrap();
            assert_eq!(board.read_bypass_byte(PCI_CONFIG + offset).unwrap(), value);
        }
        assert!(write_bar(&mut board).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x41, 0xff).unwrap();
        write_bar(&mut board).unwrap();
        assert!(board
            .write_bypass_le_word(PCI_CONFIG + 0x20814, EBUS_BAR1)
            .is_err());
        assert!(board.write_bypass_le_word(bar, 0xe100_0000).is_err());
        board.write_bypass_byte(PCI_CONFIG + 0x91a, 0).unwrap();
        assert!(write_bar(&mut board).is_err());
        let base = PCI_MEMORY + u64::from(EBUS_BAR1);
        for (i, value) in [0x1122_3344, 0x5566_7700, 0x0102_0304]
            .into_iter()
            .enumerate()
        {
            board
                .write_bypass_le_word(base + 0x710000 + 4 * i as u64, value)
                .unwrap();
        }
        assert_eq!(board.ebus_setup, [0x1122_3344, 0x5566_7700, 0x0102_0304]);
        assert!(board.write_bypass_le_word(base + 0x71000c, 0).is_err());
    }

    #[test]
    fn super_io_id_is_one_shot_and_paired_writes_are_interruptible() {
        let mut board = fixture();
        let index = PCI_MEMORY + u64::from(EBUS_BAR1) + SIO_INDEX;
        let data = index + 1;
        assert_eq!(board.read_bypass_byte(index).unwrap(), 0x88);
        assert_eq!(board.read_bypass_byte(index).unwrap(), 0);
        board.write_bypass_byte(index, 1).unwrap();
        for v in [0x11, 0x11] {
            board.write_bypass_byte(data, v).unwrap();
        }
        assert_eq!(board.read_bypass_byte(data).unwrap(), 0x11);
        board.write_bypass_byte(data, 0xb0).unwrap();
        assert_eq!(board.read_bypass_byte(data).unwrap(), 0x11);
        board.write_bypass_byte(index, 0x41).unwrap();
        board.write_bypass_byte(data, 0xb0).unwrap();
        board.write_bypass_byte(data, 0xb0).unwrap();
        assert_eq!(board.read_bypass_byte(index).unwrap(), 1);
        assert_eq!(board.read_bypass_byte(data).unwrap(), 0xb0);
        assert!(board.write_bypass_byte(index, 0x80).is_err());
        assert_eq!(board.read_bypass_byte(index).unwrap(), 1);
    }

    #[test]
    fn firmware_table_is_read_only_and_nvram_writes_preserve_endian_and_device_boundaries() {
        let mut board = fixture();
        let base = crate::ultra5::RESET_NVRAM_BASE;
        board
            .write_bypass_xword(base + 16, 0x0102_0304_0506_0708)
            .unwrap();
        board.write_bypass_word(base + 20, 0xaabb_ccdd).unwrap();
        assert_eq!(
            board.read_bypass_xword(base + 16).unwrap(),
            0x0102_0304_aabb_ccdd
        );
        assert_eq!(board.read_bypass_byte(base + 21).unwrap(), 0xbb);
        assert!(board.write_bypass_xword(base + 0x1ff0, 0).is_err());
        assert!(board.write_bypass_word(FLASH_BASE + 0x2a8, 0).is_err());
        assert_eq!(board.read_bypass_byte(FLASH_BASE + 0x2a8).unwrap(), 0);
        assert_eq!(board.read_bypass_xword(0x01fe_0000_f020).unwrap(), 1 << 31);
    }
}
