//! Machine-owned physical decoding. Device bytes never pass through RAM's
//! word-based narrow-access defaults or a speculative read-modify-write.
use crate::bus::{
    AccessContext, Bus, BusError, BusErrorKind, IoTlbFlush, MapError, MemoryBus, MemoryRequest,
};
use crate::devices::mxcc::MxccState;
use crate::devices::{
    auxiliary::Auxiliary,
    cgsix::{Cgsix, VRAM_BYTES, VRAM_OFFSET},
    cs4231::Cs4231,
    ecc::{Ecc, ECC_SIZE},
    esp::Esp,
    ethernet::Ethernet,
    ethernet_dma::EthernetDma,
    floppy::Floppy,
    interrupt::{InterruptController, Source, TargetPolicy},
    io_control::{IoControl, IO_CONTROL_SIZE},
    keyboard::Keyboard,
    mouse::Mouse,
    nvram::{Nvram, NVRAM_SIZE},
    reset::ResetControl,
    scsi::ScsiBus,
    scsi_dma::ScsiDma,
    serial::Serial,
    sx::Sx,
    timer::Timers,
};
use crate::dram::{Ss5Dram, APERTURE_BYTES};
use crate::topology::{ProcessorArbitration, ProcessorModule};
use std::cell::RefCell;
use std::rc::Rc;

pub const IO_CONTROL_BASE: u64 = 0x1000_0000;
pub const CGSIX_BASE: u64 = 0x3000_0000; // SS5 external SBus slot 1
/// SS-20/Axil SBus slot 2, PA(35:28) = 0xE2 (Sun-4M System Architecture 3.2.3).
pub const SS20_CGSIX_BASE: u64 = 0xe_2000_0000;
const CGSIX_SIZE: u64 = 0x0200_0000;
/// SS5 onboard Crystal CS4231 audio register aperture.
pub const CS4231_BASE: u64 = 0x6c00_0000;
/// SS-5 board-level AFX presence byte. The selected SBus CGsix profile has
/// no AFX card. Only the firmware-observed byte read is implemented.
pub const AFX_PRESENCE_BASE: u64 = 0x6e00_0000;
pub const KEYBOARD_SERIAL_BASE: u64 = 0x7100_0000;
pub const SERIAL_BASE: u64 = 0x7110_0000;
pub const NVRAM_BASE: u64 = 0x7120_0000;
pub const FLOPPY_BASE: u64 = 0x7140_0000;
pub const AUXILIARY_BASE: u64 = 0x7190_0000;
pub const AUXILIARY_POWER_BASE: u64 = 0x7191_0000;
pub const CPU_TIMER_BASE: u64 = 0x71d0_0000;
pub const SYSTEM_TIMER_BASE: u64 = 0x71d1_0000;
pub const CPU_INTERRUPT_BASE: u64 = 0x71e0_0000;
pub const SYSTEM_INTERRUPT_BASE: u64 = 0x71e1_0000;
pub const RESET_BASE: u64 = 0x71f0_0000;
pub const MACIO_ID_BASE: u64 = 0x7800_0000;
pub const SCSI_DMA_BASE: u64 = 0x7840_0000;
pub const ETHERNET_DMA_BASE: u64 = 0x7840_0010;
pub const ESP_BASE: u64 = 0x7880_0000;
pub const ETHERNET_BASE: u64 = 0x78c0_0000;
const MACIO_DMA_SIZE: u64 = 0x40;

/// Physical decode for one sun4m profile. The device models are shared; only
/// their physical bases, PROM aperture and SBus error classification change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sun4mLayout {
    pub io_control: u64,
    pub cs4231: Option<u64>,
    pub ecc: Option<u64>,
    pub cgsix: u64,
    pub afx_presence: Option<u64>,
    pub keyboard_serial: u64,
    pub serial: u64,
    pub nvram: u64,
    pub floppy: u64,
    pub auxiliary: u64,
    pub auxiliary_power: u64,
    pub cpu_timer: u64,
    pub system_timer: u64,
    pub cpu_interrupt: u64,
    pub system_interrupt: u64,
    pub reset: u64,
    /// Unpopulated CPU timer/interrupt slots, absent MBus module windows and
    /// their architected port words.
    pub zero_windows: [Option<(u64, u64)>; 27],
    pub macio_id: u64,
    pub scsi_dma: u64,
    pub esp: u64,
    pub ethernet: u64,
    pub prom: u64,
    /// Decoded boot PROM alias size. SS-20/Axil OBP maps a 16 MiB page over
    /// this space; only the first MAX_PROM_BYTES are populated by the image.
    pub prom_aperture: u64,
    /// The reference-MMU PROM alias ignores write strobes: SS-20 POST
    /// rewrites bytes in the D-cache level-2 PTP test and must not fault.
    pub prom_writes_ignored: bool,
    /// SS-5 NCR windows have documented slave-error holes; other profiles do
    /// not yet classify reserved NCR windows differently from empty slots.
    pub ncr_windows: bool,
}

impl Sun4mLayout {
    pub const fn ss5() -> Self {
        Self {
            io_control: IO_CONTROL_BASE,
            cs4231: Some(CS4231_BASE),
            ecc: None,
            cgsix: CGSIX_BASE,
            afx_presence: Some(AFX_PRESENCE_BASE),
            keyboard_serial: KEYBOARD_SERIAL_BASE,
            serial: SERIAL_BASE,
            nvram: NVRAM_BASE,
            floppy: FLOPPY_BASE,
            auxiliary: AUXILIARY_BASE,
            auxiliary_power: AUXILIARY_POWER_BASE,
            cpu_timer: CPU_TIMER_BASE,
            system_timer: SYSTEM_TIMER_BASE,
            cpu_interrupt: CPU_INTERRUPT_BASE,
            system_interrupt: SYSTEM_INTERRUPT_BASE,
            reset: RESET_BASE,
            zero_windows: [None; 27],
            macio_id: MACIO_ID_BASE,
            scsi_dma: SCSI_DMA_BASE,
            esp: ESP_BASE,
            ethernet: ETHERNET_BASE,
            prom: 0x7000_0000,
            prom_aperture: 0x10_0000,
            prom_writes_ignored: false,
            ncr_windows: true,
        }
    }

    /// SPARCstation 20 sun4m decode. The Axil 320 reuses the CPU/memory map
    /// but does not expose the SS-20 built-in SX/VSIMM device.
    #[allow(clippy::unusual_byte_groupings)]
    pub const fn ss20() -> Self {
        Self {
            io_control: 0xf_e000_0000,
            cs4231: None,
            ecc: Some(0xf_0000_0000),
            cgsix: SS20_CGSIX_BASE,
            afx_presence: None,
            keyboard_serial: 0xf_f100_0000,
            serial: 0xf_f110_0000,
            nvram: 0xf_f120_0000,
            floppy: 0xf_f170_0000,
            auxiliary: 0xf_f180_0000,
            auxiliary_power: 0xf_f1a0_1000,
            cpu_timer: 0xf_f130_0000,
            system_timer: 0xf_f131_0000,
            cpu_interrupt: 0xf_f140_0000,
            system_interrupt: 0xf_f141_0000,
            reset: 0xf_f1f0_0000,
            zero_windows: [
                Some((0xf_f130_1000, 0xf_f131_0000)),
                Some((0xf_f140_1000, 0xf_f141_0000)),
                Some((0x9c00_0000, 0x9c00_2000)),
                Some((0x9000_0000, 0x9000_2000)),
                Some((0x9400_0000, 0x9400_2000)),
                Some((0x9800_0000, 0x9800_2000)),
                Some((0x8c00_0000, 0x8c00_2000)),
                Some((0x8800_0000, 0x8800_2000)),
                Some((0x8400_0000, 0x8400_2000)),
                Some((0x8000_0000, 0x8000_2000)),
                Some((0xf_fbc0_0000, 0xf_fbc0_2000)),
                Some((0xf_fac0_0000, 0xf_fac0_2000)),
                Some((0xf_f9c0_0000, 0xf_f9c0_2000)),
                Some((0xf_f900_0000, 0xf_f900_2000)),
                Some((0xf_f940_0000, 0xf_f940_2000)),
                Some((0xf_f980_0000, 0xf_f980_2000)),
                Some((0xf_f8c0_0000, 0xf_f8c0_2000)),
                Some((0xf_f880_0000, 0xf_f880_2000)),
                Some((0xf_f840_0000, 0xf_f840_2000)),
                Some((0xf_f800_0000, 0xf_f800_2000)),
                Some((0xf_8000_0000, 0xf_8000_2000)),
                // Architected MBus port words for unpopulated MIDs 9, A and B.
                Some((0xf_f9ff_fffc, 0xf_fa00_0000)),
                Some((0xf_faff_fffc, 0xf_fb00_0000)),
                Some((0xf_fbff_fffc, 0xf_fc00_0000)),
                Some((0xe_e000_1000, 0xe_e000_1030)),
                Some((0xe_e001_0000, 0xe_e001_0100)),
                Some((0xe_f480_0000, 0xe_f480_0020)),
            ],
            macio_id: 0xe_f000_0000,
            scsi_dma: 0xe_f040_0000,
            esp: 0xe_f080_0000,
            ethernet: 0xe_f0c0_0000,
            prom: 0xf_f000_0000,
            prom_aperture: 0x100_0000,
            prom_writes_ignored: true,
            ncr_windows: false,
        }
    }

    pub const fn axil320() -> Self {
        Self::ss20()
    }

    pub const fn ethernet_dma(&self) -> u64 {
        self.scsi_dma + 0x10
    }

    /// SS-5's integrated I/O control identifies as microSPARC-II rev 4; the
    /// SS-20/Axil standalone IOMMU reports implementation 1, version 3.
    pub const fn io_control_version(&self) -> u32 {
        if self.ncr_windows {
            0x0400_0000
        } else {
            0x1300_0000
        }
    }

    pub const fn device_ranges(&self) -> [(u64, u64); 46] {
        let cs4231 = match self.cs4231 {
            Some(base) => (base, base + 0x40),
            None => (0, 0),
        };
        let afx = match self.afx_presence {
            Some(base) => (base, base + 1),
            None => (0, 0),
        };
        let ecc = match self.ecc {
            Some(base) => (base, base + ECC_SIZE),
            None => (0, 0),
        };
        [
            (self.io_control, self.io_control + IO_CONTROL_SIZE),
            cs4231,
            afx,
            (self.keyboard_serial, self.keyboard_serial + 8),
            (self.serial, self.serial + 8),
            (self.nvram, self.nvram + NVRAM_SIZE as u64),
            (self.floppy, self.floppy + 0x10_0000),
            (self.auxiliary, self.auxiliary + 1),
            (self.auxiliary_power, self.auxiliary_power + 1),
            (self.cpu_timer, self.cpu_timer + 0x10),
            (self.system_timer, self.system_timer + 0x14),
            (self.cpu_interrupt, self.cpu_interrupt + 0x10),
            (self.system_interrupt, self.system_interrupt + 0x14),
            (self.reset, self.reset + 4),
            (self.macio_id, self.macio_id + 4),
            (self.scsi_dma, self.scsi_dma + MACIO_DMA_SIZE),
            (self.esp, self.esp + 0x40),
            (self.ethernet, self.ethernet + 4),
            ecc,
            match self.zero_windows[0] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[1] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[2] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[3] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[4] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[5] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[6] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[7] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[8] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[9] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[10] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[11] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[12] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[13] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[14] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[15] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[16] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[17] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[18] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[19] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[20] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[21] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[22] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[23] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[24] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[25] {
                Some(range) => range,
                None => (0, 0),
            },
            match self.zero_windows[26] {
                Some(range) => range,
                None => (0, 0),
            },
        ]
    }
}

#[derive(Clone, Copy)]
enum WordDevice {
    IoControl,
    Cs4231,
    Ecc,
    Timer(bool, u8),
    Interrupt(bool, u8),
    Reset,
    MacioId,
    ScsiDma,
    EthernetDma,
}

#[derive(Debug)]
pub struct Sun4mDevices {
    pub auxiliary: Auxiliary,
    pub cs4231: Cs4231,
    pub ecc: Ecc,
    pub floppy: Floppy,
    pub io_control: IoControl,
    pub nvram: Nvram,
    pub serial: Serial,
    /// SCC channels A (keyboard) and B (Sun Mouse Systems).
    pub keyboard_serial: Serial,
    pub keyboard: Option<Keyboard>,
    pub mouse: Mouse,
    pub interrupts: InterruptController,
    pub timers: Timers,
    pub reset: ResetControl,
    pub scsi_dma: ScsiDma,
    pub esp: Esp,
    pub scsi: ScsiBus,
    pub ethernet_dma: EthernetDma,
    pub ethernet: Ethernet,
    rtc_phase_ns: u32,
    timer_ns: u64,
}

impl Sun4mDevices {
    pub(crate) fn sync_interrupts(&mut self) {
        self.ethernet_dma.set_mac_irq(self.ethernet.irq());
        self.scsi_dma.set_scsi_irq(self.esp.irq());
        self.interrupts
            .set_source(Source::Scsi, self.scsi_dma.irq());
        self.interrupts
            .set_source(Source::Ethernet, self.ethernet_dma.irq());
        self.interrupts
            .set_source(Source::Floppy, self.floppy.irq());
        self.interrupts
            .set_source(Source::Serial, self.serial.irq());
        self.interrupts
            .set_source(Source::Keyboard, self.keyboard_serial.irq());
        self.interrupts
            .set_source(Source::SystemTimer, self.timers.system_irq());
        for processor in self.timers.processors() {
            let asserted = self.timers.processor_irq_for(processor);
            self.interrupts.set_processor_timer(processor, asserted);
        }
    }

    /// Install the interrupt and timer bank for a hardware processor number.
    pub fn install_processor(&mut self, processor: u8) -> bool {
        let supported = match self.interrupts.policy() {
            TargetPolicy::Ncr => processor == 0,
            TargetPolicy::Mbus => matches!(processor, 0 | 2),
        };
        if !supported {
            return false;
        }
        let interrupt = self.interrupts.install_processor(processor);
        let timer = self.timers.install_processor(processor);
        self.sync_interrupts();
        interrupt || timer
    }

    fn advance_ns(&mut self, ns: u64) {
        if let Some(keyboard) = &mut self.keyboard {
            keyboard.advance_ns(ns, &mut self.keyboard_serial);
        }
        if !self.mouse.idle() {
            self.mouse.advance_ns(ns, &mut self.keyboard_serial);
        }
        self.ethernet.advance_ns(ns);
        self.esp.advance_ns(ns);
        self.floppy.advance_ns(ns);
        self.timer_ns += ns;
        if self.timer_ns >= 500 {
            self.timers.advance_ns(self.timer_ns);
            self.timer_ns = 0;
        }
        let remainder = ns % 1_000_000_000 + u64::from(self.rtc_phase_ns);
        self.nvram
            .advance_seconds(ns / 1_000_000_000 + remainder / 1_000_000_000);
        self.rtc_phase_ns = (remainder % 1_000_000_000) as u32;
        self.sync_interrupts();
    }
}

#[derive(Debug)]
struct InstalledMxcc {
    mid: u8,
    state: Rc<RefCell<MxccState>>,
}

#[derive(Debug)]
pub struct SystemBus {
    layout: Sun4mLayout,
    memory: MemoryBus,
    devices: Option<Sun4mDevices>,
    dram: Option<Ss5Dram>,
    context: AccessContext,
    configured_processors: u8,
    prom_cycles: Option<u64>,
    prom_phase: u64,
    cgsix: Option<Cgsix>,
    sx: Option<Sx>,
    mxccs: Vec<InstalledMxcc>,
    device_ns: u64,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::with_layout(Sun4mLayout::ss5())
    }
}

/// SS5/85 SBus clock (CPU / 4), SPARCengine 5 table 2-1.
pub const SBUS_CLOCK_HZ: u64 = 21_250_000;
/// NCR89C105 table 6-10: six SBus clocks for each byte-wide PROM read.
pub const PROM_BYTE_CYCLES: u64 = 6;

impl SystemBus {
    pub fn with_layout(layout: Sun4mLayout) -> Self {
        Self {
            layout,
            memory: MemoryBus::default(),
            devices: None,
            dram: None,
            context: AccessContext::default(),
            configured_processors: 0x01, // hardware processor 0 / MID 8 boot CPU
            prom_cycles: None,
            prom_phase: 0,
            cgsix: None,
            sx: None,
            mxccs: Vec::new(),
            device_ns: 0,
        }
    }

    pub fn layout(&self) -> Sun4mLayout {
        self.layout
    }

    /// Current bus access attributes. The default is a host/debug baseline
    /// with no MBus owner; CPU steps and DMA transfers replace it for the
    /// complete operation and restore it on success or failure.
    pub fn access_context(&self) -> AccessContext {
        self.context
    }

    pub fn set_access_context(&mut self, context: AccessContext) {
        self.context = context;
    }

    /// Run a direct host/debug access with an explicit no-CPU identity. The
    /// previous context is restored even when the closure reports an error.
    pub fn with_host_access<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.with_access_context(AccessContext::host(), f)
    }

    /// Scoped access context for CPU/DMA/debug owners. Nested bus calls retain
    /// the context; the caller's previous attributes are restored afterwards.
    pub fn with_access_context<R>(
        &mut self,
        context: AccessContext,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = self.context;
        self.context = context;
        let result = f(self);
        self.context = previous;
        result
    }

    /// Mark a hardware processor as configured on the board. This is presence
    /// for arbitration queries, not CPU storage or a scheduler slot.
    pub fn configure_processor(&mut self, processor: u8) -> bool {
        let supported = if self.layout.ncr_windows {
            processor == 0
        } else {
            matches!(processor, 0 | 2)
        };
        if !supported {
            return false;
        }
        if self.configured_processors & (1 << processor) != 0 {
            return false;
        }
        self.configured_processors |= 1 << processor;
        true
    }

    pub fn processor_present(&self, processor: u8) -> bool {
        processor < 8 && self.configured_processors & (1 << processor) != 0
    }

    /// Arbitration eligibility from configured presence and the Sun-4M AER.
    /// Processor 0 needs no enable bit. Enabling an absent MID 9/A/B does not
    /// create a processor.
    pub fn processor_arbitration(&self, processor: u8) -> ProcessorArbitration {
        if !self.processor_present(processor) {
            return ProcessorArbitration::Absent;
        }
        // Processor 0 has no AER enable bit and is the boot master.
        if processor == 0 {
            return ProcessorArbitration::Enabled;
        }
        let enabled = self
            .devices
            .as_ref()
            .is_some_and(|devices| devices.io_control.processor_arbitration_enabled(processor));
        if enabled {
            ProcessorArbitration::Enabled
        } else {
            ProcessorArbitration::Disabled
        }
    }

    /// Host-side enable/disable transition for an installed processor. Returns
    /// false for absent modules, processor 0 (always enabled), the integrated
    /// SS-5 controller, or before the sun4m devices are installed.
    pub fn set_processor_arbitration(&mut self, processor: u8, enabled: bool) -> bool {
        if !self.processor_present(processor) {
            return false;
        }
        let Some(devices) = &mut self.devices else {
            return false;
        };
        if !devices
            .io_control
            .set_processor_arbitration(processor, enabled)
        {
            return false;
        }
        self.processor_arbitration(processor).is_enabled() == enabled
    }

    pub fn prom_base(&self) -> u64 {
        self.layout.prom
    }

    pub fn prom_aperture(&self) -> u64 {
        self.layout.prom_aperture
    }

    /// The reference-MMU PROM chips ignore write strobes; OBP's POST relies on
    /// this when it rewrites the low boot alias. Reads still come from the
    /// padded image.
    fn prom_contains(&self, address: u64) -> bool {
        (self.layout.prom..self.layout.prom + self.layout.prom_aperture).contains(&address)
    }

    fn prom_writes_ignored(&self, address: u64) -> bool {
        self.layout.prom_writes_ignored && self.prom_contains(address)
    }

    pub fn install_cgsix(&mut self, card: Cgsix) -> Result<(), MapError> {
        if self.cgsix.is_some()
            || self
                .memory
                .overlaps(self.layout.cgsix, self.layout.cgsix + CGSIX_SIZE)
        {
            return Err(MapError::Overlap);
        }
        self.cgsix = Some(card);
        Ok(())
    }
    pub fn cgsix(&self) -> Option<&Cgsix> {
        self.cgsix.as_ref()
    }
    pub fn install_sx(&mut self, card: Sx) -> Result<(), MapError> {
        if self.sx.is_some() {
            return Err(MapError::Overlap);
        }
        self.sx = Some(card);
        Ok(())
    }
    /// Remove an SX/VSIMM card. An SS-20 without a VSIMM has no cg14 device.
    pub fn remove_sx(&mut self) -> Option<Sx> {
        self.sx.take()
    }
    pub fn sx(&self) -> Option<&Sx> {
        self.sx.as_ref()
    }
    /// Install or replace the boot CPU's MXCC state.
    pub fn install_mxcc(&mut self, mxcc: Rc<RefCell<MxccState>>) {
        self.install_mxcc_module(mxcc);
    }

    /// Install an independently addressed processor-module MXCC. MID 8 and
    /// MID 10 are distinct physical windows; replacing the same MID updates
    /// that module rather than creating an alias.
    pub fn install_mxcc_module(&mut self, mxcc: Rc<RefCell<MxccState>>) {
        let mid = mxcc.borrow().mid();
        if let Some(slot) = self.mxccs.iter_mut().find(|slot| slot.mid == mid) {
            slot.state = mxcc;
        } else {
            self.mxccs.push(InstalledMxcc { mid, state: mxcc });
        }
    }

    /// Decode physical module addresses from install-time identity only. Do not
    /// borrow live MXCC state here: a stream transfer may be running from a
    /// snapshot while the guest also addresses ordinary RAM or another module.
    fn mxcc_at(&self, address: u64) -> Option<(Rc<RefCell<MxccState>>, u32)> {
        self.mxccs.iter().find_map(|slot| {
            let module = ProcessorModule::from_mid(slot.mid)?;
            let register = module.mxcc_register(address)?;
            Some((slot.state.clone(), register))
        })
    }
    fn sx_at(&mut self, address: u64) -> Option<&mut Sx> {
        if self.sx.as_ref().is_some_and(|card| card.contains(address)) {
            self.sx.as_mut()
        } else {
            None
        }
    }
    fn sync_cgsix_irq(&mut self) {
        if let (Some(devices), Some(card)) = (&mut self.devices, &self.cgsix) {
            devices.interrupts.set_source(Source::SBus5, card.irq());
        }
    }
    fn sync_sx_irq(&mut self) {
        if let (Some(devices), Some(card)) = (&mut self.devices, &self.sx) {
            devices.interrupts.set_source(Source::Video, card.irq());
        }
    }
    fn cgsix_at(&mut self, address: u64) -> Option<&mut Cgsix> {
        if (self.layout.cgsix..self.layout.cgsix + CGSIX_SIZE).contains(&address) {
            self.cgsix.as_mut()
        } else {
            None
        }
    }
    pub(crate) fn begin_cpu_step(&mut self) {
        self.prom_cycles = Some(0);
    }

    pub(crate) fn finish_cpu_step(&mut self, completed: bool) -> u64 {
        let cycles = self.prom_cycles.take().unwrap_or(0);
        if !completed {
            return 0;
        }
        let numerator = cycles * 1_000_000_000 + self.prom_phase;
        self.prom_phase = numerator % SBUS_CLOCK_HZ;
        numerator / SBUS_CLOCK_HZ
    }

    fn account_prom_read(&mut self, address: u64, bytes: u64) {
        if self.devices.is_some() && (0x7000_0000..0x7010_0000).contains(&address) {
            if let Some(cycles) = &mut self.prom_cycles {
                *cycles += bytes * PROM_BYTE_CYCLES;
            }
        }
    }

    fn check_macio_access(&mut self, address: u64, bytes: u64) -> Result<(), BusError> {
        let dma = (self.layout.scsi_dma..self.layout.scsi_dma + MACIO_DMA_SIZE).contains(&address);
        let scsi = dma && address & 0x10 == 0;
        let ethernet = dma && address & 0x10 != 0;
        let mac = (self.layout.ethernet..self.layout.ethernet + 4).contains(&address);
        let id = (self.layout.macio_id..self.layout.macio_id + 4).contains(&address);
        let esp = (self.layout.esp..self.layout.esp + 0x40).contains(&address);
        if let Some(devices) = &mut self.devices {
            if esp && self.layout.ncr_windows && bytes != 1 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("wide ESP access"),
                });
            }
            if esp && !self.layout.ncr_windows {
                if address % bytes != 0 {
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::Misaligned,
                    });
                }
                if bytes > 4 {
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::SBusError,
                    });
                }
            }
            if scsi || ethernet || mac || id {
                if address % bytes != 0 {
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::Misaligned,
                    });
                }
                if ((scsi || ethernet) && bytes != 4) || (mac && bytes != 2) || bytes > 4 {
                    if scsi {
                        devices.scsi_dma.slave_access_error();
                    }
                    if ethernet || mac {
                        devices.ethernet_dma.slave_access_error();
                    }
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::SBusError,
                    });
                }
            }
        }
        Ok(())
    }
    fn check_floppy_access(&self, address: u64, bytes: u64) -> Result<(), BusError> {
        if self.devices.is_some()
            && (self.layout.floppy..self.layout.floppy + 0x10_0000).contains(&address)
        {
            if address % bytes != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            if bytes != 1 {
                // NCR89C105 rejects wider transfers throughout this aperture.
                return Err(BusError {
                    address,
                    kind: BusErrorKind::SBusError,
                });
            }
        }
        Ok(())
    }
    fn physical_error(&self, error: BusError) -> BusError {
        // microSPARC-II table 15: PA[30:28]=3..7 select SBus slots.
        if self.devices.is_some()
            && self.layout.ncr_windows
            && (0x3000_0000..0x8000_0000).contains(&error.address)
            && error.kind == BusErrorKind::Unmapped
        {
            // Reserved NCR89C105 windows are decoded by an existing slave,
            // unlike an empty SBus slot. Keep missing register mirrors and
            // devices outside these documented holes separate.
            let reserved_ncr = (0x7150_0000..0x7180_0000).contains(&error.address)
                || (0x71c0_0000..0x71d0_0000).contains(&error.address)
                || (0x71e0_0010..0x71e1_0000).contains(&error.address)
                || (0x71e1_0014..0x71f0_0000).contains(&error.address)
                || (0x71f0_0004..0x7200_0000).contains(&error.address);
            BusError {
                kind: if reserved_ncr {
                    BusErrorKind::SBusError
                } else {
                    BusErrorKind::SBusTimeout
                },
                ..error
            }
        } else {
            error
        }
    }
    pub fn install_ss5_dram(&mut self, dram: Ss5Dram) -> Result<(), MapError> {
        if self.dram.is_some() || self.memory.overlaps(0, APERTURE_BYTES) {
            return Err(MapError::Overlap);
        }
        self.dram = Some(dram);
        Ok(())
    }

    pub fn dram(&self) -> Option<&Ss5Dram> {
        self.dram.as_ref()
    }

    fn dram_at(&mut self, address: u64) -> Option<&mut Ss5Dram> {
        if self
            .dram
            .as_ref()
            .is_some_and(|dram| address < dram.aperture_bytes())
        {
            self.dram.as_mut()
        } else {
            None
        }
    }

    pub fn install_sun4m(&mut self, nvram: Nvram) -> Result<(), MapError> {
        if self.devices.is_some()
            || self
                .layout
                .device_ranges()
                .iter()
                .any(|&(start, end)| self.memory.overlaps(start, end))
        {
            return Err(MapError::Overlap);
        }
        let mut devices = Sun4mDevices {
            auxiliary: Auxiliary::default(),
            cs4231: Cs4231::default(),
            ecc: Ecc::default(),
            floppy: Floppy::default(),
            io_control: IoControl::new(self.layout.io_control_version()),
            nvram,
            serial: Serial::default(),
            keyboard_serial: Serial::default(),
            keyboard: None,
            mouse: Mouse::default(),
            interrupts: InterruptController::with_policy(if self.layout.ncr_windows {
                TargetPolicy::Ncr
            } else {
                TargetPolicy::Mbus
            }),
            timers: Timers::default(),
            reset: ResetControl::default(),
            scsi_dma: ScsiDma::default(),
            esp: Esp::default(),
            scsi: ScsiBus::default(),
            ethernet_dma: EthernetDma::default(),
            ethernet: Ethernet::default(),
            rtc_phase_ns: 0,
            timer_ns: 0,
        };
        devices
            .esp
            .set_reference_registers(!self.layout.ncr_windows);
        self.devices = Some(devices);
        Ok(())
    }
    pub fn devices(&self) -> Option<&Sun4mDevices> {
        self.devices.as_ref()
    }
    pub fn devices_mut(&mut self) -> Option<&mut Sun4mDevices> {
        self.devices.as_mut()
    }

    /// Install processor-local interrupt and timer banks by hardware processor
    /// number. The default machine has only processor 0; a second module uses
    /// processor 2. Unpopulated numbers remain zero windows.
    pub fn install_processor_banks(&mut self, processor: u8) -> bool {
        let supported = if self.layout.ncr_windows {
            processor == 0
        } else {
            matches!(processor, 0 | 2)
        };
        if !supported {
            return false;
        }
        self.devices
            .as_mut()
            .is_some_and(|devices| devices.install_processor(processor))
    }

    /// Compatibility spelling for callers that install one processor record.
    pub fn install_processor(&mut self, processor: u8) -> bool {
        self.install_processor_banks(processor)
    }

    pub fn processor_timer_base(&self, processor: u8) -> u64 {
        self.layout.cpu_timer + u64::from(processor) * 0x1000
    }

    pub fn processor_interrupt_base(&self, processor: u8) -> u64 {
        self.layout.cpu_interrupt + u64::from(processor) * 0x1000
    }

    pub fn system_timer_base(&self) -> u64 {
        self.layout.system_timer
    }

    pub fn system_interrupt_base(&self) -> u64 {
        self.layout.system_interrupt
    }

    pub fn reset_devices(&mut self) {
        self.reset_io();
        if let Some(devices) = &mut self.devices {
            devices.reset.power_on_reset();
        }
    }
    pub(crate) fn software_reset_devices(&mut self) {
        self.reset_io();
        if let Some(devices) = &mut self.devices {
            devices.reset.software_reset();
        }
    }
    fn reset_io(&mut self) {
        self.flush_device_time();
        if let Some(card) = &mut self.cgsix {
            card.reset();
        }
        if let Some(card) = &mut self.sx {
            card.reset();
        }
        for mxcc in &self.mxccs {
            mxcc.state.borrow_mut().reset();
        }
        self.context = AccessContext::default();
        self.prom_cycles = None;
        self.prom_phase = 0;
        if let Some(devices) = &mut self.devices {
            devices.io_control.reset();
            devices.auxiliary.reset();
            devices.cs4231.reset();
            devices.ecc.reset();
            devices.floppy.reset();
            devices.scsi_dma.reset();
            devices.esp.reset();
            devices.ethernet_dma.reset();
            devices.ethernet.reset();
            devices.serial.reset();
            devices.keyboard_serial.reset();
            if let Some(keyboard) = &mut devices.keyboard {
                keyboard.reset();
            }
            devices.mouse.reset();
            devices.interrupts.reset();
            devices.timers.reset();
            devices.timer_ns = 0;
        }
    }
    pub fn advance_ns(&mut self, ns: u64) {
        self.advance_devices_ns(ns);
    }

    /// CPU-boundary path: coalesce device work to a 2 us boundary.
    pub(crate) fn advance_ns_fast(&mut self, ns: u64) {
        self.device_ns = self.device_ns.saturating_add(ns);
        if self.device_ns < 2000 {
            return;
        }
        let elapsed = std::mem::take(&mut self.device_ns);
        self.advance_devices_ns(elapsed);
    }

    fn advance_devices_ns(&mut self, ns: u64) {
        if let Some(card) = &mut self.cgsix {
            card.advance_ns(ns);
        }
        if let Some(card) = &mut self.sx {
            card.advance_ns(ns);
        }
        if let Some(devices) = &mut self.devices {
            devices.advance_ns(ns);
        }
        self.sync_cgsix_irq();
        self.sync_sx_irq();
    }

    fn flush_device_time(&mut self) {
        if self.device_ns != 0 {
            let pending = std::mem::take(&mut self.device_ns);
            self.advance_devices_ns(pending);
        }
    }
    pub fn advance_seconds(&mut self, seconds: u64) {
        if self.device_ns != 0 {
            let pending = std::mem::take(&mut self.device_ns);
            self.advance_devices_ns(pending);
        }
        if let Some(card) = &mut self.cgsix {
            card.advance_ns(seconds.saturating_mul(1_000_000_000));
        }
        if let Some(card) = &mut self.sx {
            card.advance_ns(seconds.saturating_mul(1_000_000_000));
        }
        if let Some(devices) = &mut self.devices {
            if let Some(keyboard) = &mut devices.keyboard {
                keyboard.advance_ns(
                    seconds.saturating_mul(1_000_000_000),
                    &mut devices.keyboard_serial,
                );
            }
            devices
                .ethernet
                .advance_ns(seconds.saturating_mul(1_000_000_000));
            devices
                .esp
                .advance_ns(seconds.saturating_mul(1_000_000_000));
            devices
                .floppy
                .advance_ns(seconds.saturating_mul(1_000_000_000));
            devices.timers.advance_seconds(seconds);
            devices.nvram.advance_seconds(seconds);
            devices.sync_interrupts();
        }
        self.sync_cgsix_irq();
        self.sync_sx_irq();
    }
    pub fn interrupt_levels(&mut self) -> u16 {
        self.interrupt_levels_for(0)
    }

    pub fn interrupt_levels_for(&mut self, processor: u8) -> u16 {
        self.sync_cgsix_irq();
        self.sync_sx_irq();
        self.devices
            .as_ref()
            .map_or(0, |devices| devices.interrupts.levels_for(processor))
    }
    fn processor_bank_overlaps(&self, address: u64, size: u64) -> bool {
        let end = address.saturating_add(size);
        self.devices.as_ref().is_some_and(|devices| {
            devices.interrupts.processors().any(|processor| {
                let timer = self.layout.cpu_timer + u64::from(processor) * 0x1000;
                let interrupt = self.layout.cpu_interrupt + u64::from(processor) * 0x1000;
                [timer, interrupt]
                    .into_iter()
                    .any(|start| address < start + 0x10 && end > start)
            })
        })
    }

    fn zero_at(&self, address: u64) -> bool {
        if self.processor_bank_overlaps(address, 1) {
            return false;
        }
        self.layout
            .zero_windows
            .iter()
            .flatten()
            .any(|&(start, end)| (start..end).contains(&address))
    }

    fn zero_window_overlaps(&self, address: u64, size: u64) -> bool {
        if self.processor_bank_overlaps(address, size) {
            return false;
        }
        let end = address.saturating_add(size);
        self.layout
            .zero_windows
            .iter()
            .flatten()
            .any(|&(start, window_end)| address < window_end && end > start)
    }

    fn processor_timer_at(&self, address: u64) -> Option<u8> {
        self.devices.as_ref().and_then(|devices| {
            devices
                .timers
                .processors()
                .find(|&processor| address == self.layout.cpu_timer + u64::from(processor) * 0x1000)
        })
    }

    fn word_device(&self, address: u64) -> Option<(WordDevice, u64)> {
        self.devices.as_ref()?;
        // NCR89C100 table 2-7 footnote 2 and DMA2 table 3-1: bit 5 is
        // not decoded. The second 32 bytes alias the same register bank.
        if (self.layout.scsi_dma..self.layout.scsi_dma + MACIO_DMA_SIZE).contains(&address) {
            let device = if address & 0x10 == 0 {
                WordDevice::ScsiDma
            } else {
                WordDevice::EthernetDma
            };
            return Some((device, address & 0xf));
        }
        if let Some(base) = self.layout.cs4231 {
            if (base..base + 0x40).contains(&address) {
                return Some((WordDevice::Cs4231, address - base));
            }
        }
        if let Some(base) = self.layout.ecc {
            if (base..base + ECC_SIZE).contains(&address) {
                return Some((WordDevice::Ecc, address - base));
            }
        }
        let devices = self.devices.as_ref()?;
        for processor in devices.interrupts.processors() {
            let timer = self.layout.cpu_timer + u64::from(processor) * 0x1000;
            if (timer..timer + 0x10).contains(&address) {
                return Some((WordDevice::Timer(false, processor), address - timer));
            }
            let interrupt = self.layout.cpu_interrupt + u64::from(processor) * 0x1000;
            if (interrupt..interrupt + 0x10).contains(&address) {
                return Some((WordDevice::Interrupt(false, processor), address - interrupt));
            }
        }
        for (base, size, device) in [
            (
                self.layout.io_control,
                IO_CONTROL_SIZE,
                WordDevice::IoControl,
            ),
            (self.layout.system_timer, 0x14, WordDevice::Timer(true, 0)),
            (
                self.layout.system_interrupt,
                0x14,
                WordDevice::Interrupt(true, 0),
            ),
            (self.layout.reset, 4, WordDevice::Reset),
            (self.layout.macio_id, 4, WordDevice::MacioId),
        ] {
            if (base..base + size).contains(&address) {
                return Some((device, address - base));
            }
        }
        None
    }
    fn reject_narrow_word_device(&self, address: u64) -> Result<(), BusError> {
        if self.word_device(address).is_some() {
            Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("device requires plain aligned word load/store"),
            })
        } else {
            Ok(())
        }
    }
    fn overlaps_device(&self, address: u64, size: u64) -> bool {
        (self.cgsix.is_some()
            && address < self.layout.cgsix + CGSIX_SIZE
            && address.saturating_add(size) > self.layout.cgsix)
            || self.devices.is_some()
                && self
                    .layout
                    .device_ranges()
                    .iter()
                    .any(|&(start, end)| address < end && address.saturating_add(size) > start)
    }
    fn map_check(&self, address: u64, size: usize) -> Result<(), MapError> {
        if address.checked_add(size as u64).is_none() {
            return Err(MapError::InvalidRange);
        }
        if self.overlaps_device(address, size as u64)
            || (self.dram.is_some()
                && size != 0
                && address
                    < self
                        .dram
                        .as_ref()
                        .map_or(APERTURE_BYTES, Ss5Dram::aperture_bytes))
        {
            Err(MapError::Overlap)
        } else {
            Ok(())
        }
    }
    pub fn map_ram(&mut self, address: u64, size: usize) -> Result<(), MapError> {
        self.map_check(address, size)?;
        self.memory.map_ram(address, size)
    }
    pub fn map_rom(&mut self, address: u64, bytes: &[u8]) -> Result<(), MapError> {
        self.map_check(address, bytes.len())?;
        self.memory.map_rom(address, bytes)
    }
    pub fn load_image(&mut self, address: u64, bytes: &[u8]) -> Result<(), BusError> {
        if let Some(dram) = self.dram_at(address) {
            return dram.load_image(address, bytes);
        }
        if self.overlaps_device(address, bytes.len() as u64) {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("raw image loading into devices"),
            });
        }
        self.memory.load_image(address, bytes)
    }
    fn memory_only(&self, address: u64, size: u64) -> Result<(), BusError> {
        if address % size != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if self.overlaps_device(address, size) {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported(
                    "unsupported device access width or atomic operation",
                ),
            });
        }
        Ok(())
    }
}

impl Bus for SystemBus {
    fn access_context(&self) -> AccessContext {
        self.context
    }

    fn set_access_context(&mut self, context: AccessContext) {
        self.context = context;
    }

    fn observe_memory_request(&mut self, address: u64, request: MemoryRequest) {
        let dram_aperture = self
            .dram
            .as_ref()
            .map_or(APERTURE_BYTES, Ss5Dram::aperture_bytes);
        if address < dram_aperture {
            if let Some(devices) = &mut self.devices {
                devices
                    .io_control
                    .memory_request(address, request, self.context.supervisor);
            }
        }
    }

    fn capture_sbus_write_error(&mut self, error: BusError, bytes: u8) -> bool {
        if !matches!(
            error.kind,
            BusErrorKind::SBusTimeout | BusErrorKind::SBusError
        ) || !matches!(bytes, 1 | 2 | 4)
        {
            return false;
        }
        let standalone = !self.layout.ncr_windows;
        if let Some(devices) = &mut self.devices {
            devices.io_control.sbus_write_error(
                error,
                bytes,
                self.context.supervisor,
                self.context.initiator,
            );
            if standalone {
                devices.interrupts.set_source(Source::MToS, true);
            } else {
                devices.interrupts.latch_module_error();
            }
            true
        } else {
            false
        }
    }
    fn take_io_tlb_flush(&mut self) -> Option<IoTlbFlush> {
        if !self.layout.ncr_windows {
            return None;
        }
        self.devices.as_mut()?.io_control.take_flush()
    }

    fn take_standalone_io_tlb_flush(&mut self) -> Option<IoTlbFlush> {
        if self.layout.ncr_windows {
            return None;
        }
        self.devices.as_mut()?.io_control.take_flush()
    }

    fn read8(&mut self, address: u64) -> Result<u8, BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            let word = mxcc.borrow().read_word(register & !3);
            return Ok((word >> ((3 - (register & 3)) * 8)) as u8);
        }
        if let Some(card) = self.sx_at(address) {
            return card
                .read(address, 1)
                .map(|v| v as u8)
                .map_err(|kind| BusError { address, kind });
        }

        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            return card
                .read(address - cgsix_base, 1)
                .map(|v| v as u8)
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 1)?;
        if self.devices.is_some()
            && (self.layout.macio_id..self.layout.macio_id + 4).contains(&address)
        {
            return Ok((0xfe81_0103u32 >> ((3 - (address - self.layout.macio_id)) * 8)) as u8);
        }
        self.check_floppy_access(address, 1)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read8(address);
        }
        if self.zero_at(address) {
            return Ok(0);
        }
        if let (Some(base), Some(devices)) = (self.layout.cs4231, self.devices.as_ref()) {
            if (base..base + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .read_byte(address - base)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.reject_narrow_word_device(address)?;
        if let Some(devices) = &mut self.devices {
            if (self.layout.esp..self.layout.esp + 0x40).contains(&address) {
                let result = if devices.scsi_dma.reset_asserted() {
                    Ok(0)
                } else {
                    devices.esp.read(address - self.layout.esp)
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == AFX_PRESENCE_BASE {
                // Original OBP afx-slot-empty? tests bit 0. The motherboard
                // presence port exists even when the AFX slot is empty.
                return Ok(0);
            }
            if (self.layout.floppy..self.layout.floppy + 0x10_0000).contains(&address) {
                let result = devices.floppy.read(address - self.layout.floppy);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == self.layout.auxiliary {
                return Ok(devices.auxiliary.read());
            }
            if address == self.layout.auxiliary_power {
                return Ok(devices.auxiliary.read_power());
            }
            if (self.layout.keyboard_serial..self.layout.keyboard_serial + 8).contains(&address) {
                let result = devices
                    .keyboard_serial
                    .read(address - self.layout.keyboard_serial);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (self.layout.serial..self.layout.serial + 8).contains(&address) {
                let result = devices.serial.read(address - self.layout.serial);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (self.layout.nvram..self.layout.nvram + NVRAM_SIZE as u64).contains(&address) {
                return devices
                    .nvram
                    .read((address - self.layout.nvram) as usize)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        let result = self.memory.read8(address);
        if result.is_ok() {
            self.account_prom_read(address, 1);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write8(&mut self, address: u64, value: u8) -> Result<(), BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            let shift = (3 - (register & 3)) * 8;
            let old = mxcc.borrow().read_word(register & !3);
            let merged = (old & !(0xff << shift)) | (u32::from(value) << shift);
            let mut state = mxcc.borrow().clone();
            let result = state.write_word(self, register & !3, merged);
            *mxcc.borrow_mut() = state;
            return result.map_err(|error| BusError { address, ..error });
        }
        if let Some(card) = self.sx_at(address) {
            let result = card
                .write(address, u32::from(value), 0xff)
                .map_err(|kind| BusError { address, kind });
            self.sync_sx_irq();
            return result;
        }

        if self.prom_writes_ignored(address) {
            return Ok(());
        }
        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            let shift = (3 - address % 4) * 8;
            return card
                .write(
                    (address - cgsix_base) & !3,
                    u32::from(value) << shift,
                    255 << shift,
                )
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 1)?;
        self.check_floppy_access(address, 1)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write8(address, value);
        }
        if self.zero_at(address) {
            return Ok(());
        }
        if let (Some(base), Some(devices)) = (self.layout.cs4231, self.devices.as_mut()) {
            if (base..base + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .write_byte(address - base, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.reject_narrow_word_device(address)?;
        if let Some(devices) = &mut self.devices {
            if (self.layout.esp..self.layout.esp + 0x40).contains(&address) {
                let result = if devices.scsi_dma.reset_asserted() {
                    Ok(())
                } else {
                    let result = devices.esp.write(address - self.layout.esp, value);
                    if result.is_ok() && address == self.layout.esp + 12 && value & 0x7f == 3 {
                        devices.scsi.reset();
                    }
                    result
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == AFX_PRESENCE_BASE {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("write to AFX presence port"),
                });
            }
            if (self.layout.floppy..self.layout.floppy + 0x10_0000).contains(&address) {
                let result = devices.floppy.write(address - self.layout.floppy, value);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if address == self.layout.auxiliary {
                devices.auxiliary.write(value);
                return Ok(());
            }
            if address == self.layout.auxiliary_power {
                return devices
                    .auxiliary
                    .write_power(value)
                    .map_err(|kind| BusError { address, kind });
            }
            if (self.layout.keyboard_serial..self.layout.keyboard_serial + 8).contains(&address) {
                let result = devices
                    .keyboard_serial
                    .write(address - self.layout.keyboard_serial, value);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (self.layout.serial..self.layout.serial + 8).contains(&address) {
                let result = devices.serial.write(address - self.layout.serial, value);
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
            if (self.layout.nvram..self.layout.nvram + NVRAM_SIZE as u64).contains(&address) {
                return devices
                    .nvram
                    .write((address - self.layout.nvram) as usize, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory
            .write8(address, value)
            .map_err(|error| self.physical_error(error))
    }
    fn read16(&mut self, address: u64) -> Result<u16, BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            if address & 1 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let word = mxcc.borrow().read_word(register & !3);
            return Ok((word >> ((2 - (register & 2)) * 8)) as u16);
        }
        if let Some(card) = self.sx_at(address) {
            return card
                .read(address, 2)
                .map(|v| v as u16)
                .map_err(|kind| BusError { address, kind });
        }

        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            return card
                .read(address - cgsix_base, 2)
                .map(|v| v as u16)
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 2)?;
        if (self.layout.ethernet..self.layout.ethernet + 4).contains(&address) {
            if let Some(devices) = &mut self.devices {
                return if devices.ethernet_dma.reset_asserted() {
                    Ok(0)
                } else {
                    devices
                        .ethernet
                        .read(address - self.layout.ethernet)
                        .map_err(|kind| BusError { address, kind })
                };
            }
        }
        if self.devices.is_some()
            && (self.layout.macio_id..self.layout.macio_id + 4).contains(&address)
        {
            return Ok((0xfe81_0103u32 >> ((2 - (address - self.layout.macio_id)) * 8)) as u16);
        }
        self.check_floppy_access(address, 2)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read16(address);
        }
        if self.zero_at(address) {
            if address & 1 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok(0);
        }
        if let (Some(base), Some(devices)) = (self.layout.cs4231, self.devices.as_ref()) {
            if (base..base + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .read_half(address - base)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory_only(address, 2)?;
        let result = self.memory.read16(address);
        if result.is_ok() {
            self.account_prom_read(address, 2);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write16(&mut self, address: u64, value: u16) -> Result<(), BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            if address & 1 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let shift = (2 - (register & 2)) * 8;
            let old = mxcc.borrow().read_word(register & !3);
            let merged = (old & !(0xffff << shift)) | (u32::from(value) << shift);
            let mut state = mxcc.borrow().clone();
            let result = state.write_word(self, register & !3, merged);
            *mxcc.borrow_mut() = state;
            return result.map_err(|error| BusError { address, ..error });
        }
        if let Some(card) = self.sx_at(address) {
            let result = card
                .write(address, u32::from(value), 0xffff)
                .map_err(|kind| BusError { address, kind });
            self.sync_sx_irq();
            return result;
        }

        if self.prom_writes_ignored(address) {
            return Ok(());
        }
        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            if address & 1 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let shift = (2 - address % 4) * 8;
            return card
                .write(
                    (address - cgsix_base) & !3,
                    u32::from(value) << shift,
                    65535 << shift,
                )
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 2)?;
        if (self.layout.ethernet..self.layout.ethernet + 4).contains(&address) {
            if let Some(devices) = &mut self.devices {
                let result = if devices.ethernet_dma.reset_asserted() {
                    Ok(())
                } else {
                    devices
                        .ethernet
                        .write(address - self.layout.ethernet, value)
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
        }
        self.check_floppy_access(address, 2)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write16(address, value);
        }
        if self.zero_at(address) {
            if address & 1 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok(());
        }
        if let (Some(base), Some(devices)) = (self.layout.cs4231, self.devices.as_mut()) {
            if (base..base + 0x40).contains(&address) {
                return devices
                    .cs4231
                    .write_half(address - base, value)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        self.memory_only(address, 2)?;
        self.memory
            .write16(address, value)
            .map_err(|error| self.physical_error(error))
    }
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            if address & 3 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok(mxcc.borrow().read_word(register));
        }
        if let Some(card) = self.sx_at(address) {
            return card
                .read(address, 4)
                .map_err(|kind| BusError { address, kind });
        }

        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            return card
                .read(address - cgsix_base, 4)
                .map_err(|kind| BusError { address, kind });
        }
        self.check_macio_access(address, 4)?;
        self.check_floppy_access(address, 4)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read32(address);
        }
        if self.zero_at(address) {
            if address & 3 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok(0);
        }
        if address & 3 != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if let Some(devices) = &mut self.devices {
            if (self.layout.esp..self.layout.esp + 0x40).contains(&address) {
                let result = if devices.scsi_dma.reset_asserted() {
                    Ok(0)
                } else {
                    devices.esp.read(address - self.layout.esp).map(u32::from)
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
        }
        if let Some((device, offset)) = self.word_device(address) {
            self.flush_device_time();
            let initiator = self.context.initiator;
            let devices = self.devices.as_mut().expect("decoded installed device");
            devices.sync_interrupts();
            let result = match device {
                WordDevice::IoControl => devices.io_control.read_for(offset, initiator),
                WordDevice::Cs4231 => devices.cs4231.read(offset),
                WordDevice::Ecc => devices.ecc.read(offset),
                WordDevice::Timer(system, processor) => {
                    if system {
                        devices.timers.read(true, offset)
                    } else {
                        devices.timers.read_cpu(processor, offset)
                    }
                }
                WordDevice::Interrupt(system, processor) => {
                    if system {
                        devices.interrupts.read(true, offset)
                    } else {
                        devices.interrupts.read_cpu(processor, offset)
                    }
                }
                WordDevice::Reset => Ok(devices.reset.read()),
                WordDevice::MacioId => Ok(0xfe81_0103),
                WordDevice::ScsiDma => devices.scsi_dma.read(offset),
                WordDevice::EthernetDma => devices.ethernet_dma.read(offset),
            };
            devices.sync_interrupts();
            return result.map_err(|kind| BusError { address, kind });
        }
        self.memory_only(address, 4)?;
        let result = self.memory.read32(address);
        if result.is_ok() {
            self.account_prom_read(address, 4);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            if address & 3 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let merged = if mask == u32::MAX {
                value
            } else {
                let old = mxcc.borrow().read_word(register);
                (old & !mask) | (value & mask)
            };
            let mut state = mxcc.borrow().clone();
            let result = state.write_word(self, register, merged);
            *mxcc.borrow_mut() = state;
            return result.map_err(|error| BusError { address, ..error });
        }
        if let Some(card) = self.sx_at(address) {
            let result = card
                .write(address, value, mask)
                .map_err(|kind| BusError { address, kind });
            self.sync_sx_irq();
            return result;
        }

        if self.prom_writes_ignored(address) {
            return Ok(());
        }
        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            let result = card
                .write(address - cgsix_base, value, mask)
                .map_err(|kind| BusError { address, kind });
            self.sync_cgsix_irq();
            return result;
        }
        if mask != u32::MAX
            && self.devices.is_some()
            && (self.layout.ethernet..self.layout.ethernet + 4).contains(&address)
        {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("masked Ethernet register store"),
            });
        }
        self.check_macio_access(address, 4)?;
        if mask != u32::MAX
            && self.devices.is_some()
            && (self.layout.floppy..self.layout.floppy + 0x10_0000).contains(&address)
        {
            return Err(BusError {
                address,
                kind: BusErrorKind::Unsupported("masked floppy access"),
            });
        }
        self.check_floppy_access(address, 4)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write32_masked(address, value, mask);
        }
        if self.zero_at(address) {
            if address & 3 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok(());
        }
        if address & 3 != 0 {
            return Err(BusError {
                address,
                kind: BusErrorKind::Misaligned,
            });
        }
        if let Some(devices) = &mut self.devices {
            if (self.layout.esp..self.layout.esp + 0x40).contains(&address) {
                if mask != u32::MAX {
                    return Err(BusError {
                        address,
                        kind: BusErrorKind::Unsupported("masked ESP register store"),
                    });
                }
                let result = if devices.scsi_dma.reset_asserted() {
                    Ok(())
                } else {
                    let result = devices.esp.write(address - self.layout.esp, value as u8);
                    if result.is_ok() && address == self.layout.esp + 12 && value & 0x7f == 3 {
                        devices.scsi.reset();
                    }
                    result
                };
                devices.sync_interrupts();
                return result.map_err(|kind| BusError { address, kind });
            }
        }
        if let Some((device, offset)) = self.word_device(address) {
            self.flush_device_time();
            if mask != u32::MAX {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("masked device word store"),
                });
            }
            let release_m_to_s = !self.layout.ncr_windows && offset == 0x1000;
            let devices = self.devices.as_mut().expect("decoded installed device");
            let result = match device {
                WordDevice::IoControl => {
                    let result = devices.io_control.write(offset, value);
                    if result.is_ok() && release_m_to_s {
                        devices.interrupts.set_source(Source::MToS, false);
                    }
                    result
                }
                WordDevice::Cs4231 => devices.cs4231.write(offset, value),
                WordDevice::Ecc => devices.ecc.write(offset, value),
                WordDevice::MacioId => Err(BusErrorKind::Unsupported("write to MACIO ID")),
                WordDevice::ScsiDma => {
                    let result = devices.scsi_dma.write(offset, value);
                    if result.is_ok() && devices.scsi_dma.reset_asserted() {
                        devices.esp.reset();
                    }
                    result
                }
                WordDevice::Timer(system, processor) => {
                    if system {
                        devices.timers.write(true, offset, value)
                    } else {
                        devices.timers.write_cpu(processor, offset, value)
                    }
                }
                WordDevice::EthernetDma => {
                    let result = devices.ethernet_dma.write(offset, value);
                    if result.is_ok() && devices.ethernet_dma.reset_asserted() {
                        devices.ethernet.reset();
                    }
                    result
                }
                WordDevice::Interrupt(system, processor) => {
                    if system {
                        devices.interrupts.write(true, offset, value)
                    } else {
                        devices.interrupts.write_cpu(processor, offset, value)
                    }
                }
                WordDevice::Reset => {
                    devices.reset.write(value);
                    Ok(())
                }
            };
            devices.sync_interrupts();
            return result.map_err(|kind| BusError { address, kind });
        }
        self.memory_only(address, 4)?;
        self.memory
            .write32_masked(address, value, mask)
            .map_err(|error| self.physical_error(error))
    }
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let value = mxcc.borrow().read_u64(register);
            return Ok([(value >> 32) as u32, value as u32]);
        }
        if let Some(card) = self.sx_at(address) {
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok([
                card.read(address, 4)
                    .map_err(|kind| BusError { address, kind })?,
                card.read(address + 4, 4).map_err(|kind| BusError {
                    address: address + 4,
                    kind,
                })?,
            ]);
        }
        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            let offset = address - cgsix_base;
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            if offset >= 32768 && !(VRAM_OFFSET..VRAM_OFFSET + VRAM_BYTES as u64).contains(&offset)
            {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("CGsix register doubleword"),
                });
            }
            return Ok([
                card.read(offset, 4)
                    .map_err(|kind| BusError { address, kind })?,
                card.read(offset + 4, 4).map_err(|kind| BusError {
                    address: address + 4,
                    kind,
                })?,
            ]);
        }
        self.check_macio_access(address, 8)?;
        self.check_floppy_access(address, 8)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.read_doubleword(address);
        }
        if let Some(processor) = self.processor_timer_at(address) {
            if let Some(devices) = &self.devices {
                return devices
                    .timers
                    .read_user_doubleword_for(processor)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        if self.zero_window_overlaps(address, 8) {
            return Err(BusError {
                address,
                kind: if address & 7 != 0 {
                    BusErrorKind::Misaligned
                } else {
                    BusErrorKind::Unmapped
                },
            });
        }
        self.memory_only(address, 8)?;
        let result = self.memory.read_doubleword(address);
        if result.is_ok() {
            self.account_prom_read(address, 8);
        }
        result.map_err(|error| self.physical_error(error))
    }
    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        if let Some((mxcc, register)) = self.mxcc_at(address) {
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let value = (u64::from(words[0]) << 32) | u64::from(words[1]);
            let mut state = mxcc.borrow().clone();
            let result = state.write_u64(self, register, value);
            *mxcc.borrow_mut() = state;
            return result.map_err(|error| BusError { address, ..error });
        }
        if self.prom_writes_ignored(address) {
            return Ok(());
        }
        if let Some(card) = self.sx_at(address) {
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            let result = card
                .write_doubleword(address, words)
                .map_err(|kind| BusError { address, kind });
            self.sync_sx_irq();
            return result;
        }
        let cgsix_base = self.layout.cgsix;
        if let Some(card) = self.cgsix_at(address) {
            let offset = address - cgsix_base;
            if address & 7 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            if !(VRAM_OFFSET..VRAM_OFFSET + VRAM_BYTES as u64).contains(&offset) {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Unsupported("CGsix register doubleword"),
                });
            }
            card.write(offset, words[0], u32::MAX)
                .map_err(|kind| BusError { address, kind })?;
            return card
                .write(offset + 4, words[1], u32::MAX)
                .map_err(|kind| BusError {
                    address: address + 4,
                    kind,
                });
        }
        self.check_macio_access(address, 8)?;
        self.check_floppy_access(address, 8)?;
        if let Some(dram) = self.dram_at(address) {
            return dram.write_doubleword(address, words);
        }
        if let Some(processor) = self.processor_timer_at(address) {
            if let Some(devices) = &mut self.devices {
                return devices
                    .timers
                    .write_user_doubleword_for(processor, words)
                    .map_err(|kind| BusError { address, kind });
            }
        }
        if self.zero_window_overlaps(address, 8) {
            return Err(BusError {
                address,
                kind: if address & 7 != 0 {
                    BusErrorKind::Misaligned
                } else {
                    BusErrorKind::Unmapped
                },
            });
        }
        self.memory_only(address, 8)?;
        self.memory
            .write_doubleword(address, words)
            .map_err(|error| self.physical_error(error))
    }
    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        if self.prom_writes_ignored(address) {
            return self
                .memory
                .read32(address)
                .map_err(|error| self.physical_error(error));
        }
        if let Some(dram) = self.dram_at(address) {
            return dram.swap32(address, value);
        }
        if self.zero_at(address) {
            if address & 3 != 0 {
                return Err(BusError {
                    address,
                    kind: BusErrorKind::Misaligned,
                });
            }
            return Ok(0);
        }
        self.memory_only(address, 4)?;
        self.memory
            .swap32(address, value)
            .map_err(|error| self.physical_error(error))
    }
    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        if self.prom_writes_ignored(address) {
            return self
                .memory
                .read8(address)
                .map_err(|error| self.physical_error(error));
        }
        if let Some(dram) = self.dram_at(address) {
            return dram.ldstub8(address);
        }
        if self.zero_at(address) {
            return Ok(0);
        }
        if let Some(devices) = &mut self.devices {
            if (self.layout.nvram..self.layout.nvram + NVRAM_SIZE as u64).contains(&address) {
                let offset = (address - self.layout.nvram) as usize;
                let old = devices
                    .nvram
                    .read(offset)
                    .map_err(|kind| BusError { address, kind })?;
                devices
                    .nvram
                    .write(offset, 0xff)
                    .map_err(|kind| BusError { address, kind })?;
                return Ok(old);
            }
        }
        self.memory_only(address, 1)?;
        self.memory
            .ldstub8(address)
            .map_err(|error| self.physical_error(error))
    }
}
