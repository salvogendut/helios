//! Single-transfer SBus DVMA interface for future device engines.
use crate::bus::{Bus, BusError, MemoryRequest, MemoryRequestKind};
use crate::machine::Machine;
use crate::mmu::Width;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaFault {
    OutOfRange,
    InvalidPte,
    WriteProtected,
    InvalidBypassAddress,
    Misaligned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaError {
    /// The arbiter has not granted this master a transaction; retry after SBAE.
    NotGranted {
        master: u8,
    },
    /// SBus error acknowledge to the initiating device, not a CPU MMU fault.
    Fault {
        address: u32,
        kind: DmaFault,
    },
    /// Physical table/target failure. Unsupported/WouldBlock remain host stops.
    Bus(BusError),
    Unsupported(&'static str),
}

impl fmt::Display for DmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DVMA: {self:?}")
    }
}

impl std::error::Error for DmaError {}

impl Machine {
    /// Service the ESP and at most one DMA2 byte through the shared IOMMU.
    /// Arbitration denial retains the byte; a memory fault latches D_ERR_PEND.
    pub fn service_scsi(&mut self) -> Result<(), DmaError> {
        let Some(devices) = self.bus.devices_mut() else {
            return Ok(());
        };
        if devices.scsi_dma.reset_asserted() {
            return Ok(());
        }
        let error = |kind| {
            DmaError::Bus(BusError {
                address: crate::system_bus::ESP_BASE,
                kind,
            })
        };
        let request = devices.esp.poll(&mut devices.scsi).map_err(error)?;
        devices.scsi_dma.set_request(request.is_some());
        let Some(request) = request else {
            return Ok(());
        };
        let Some(address) = devices
            .scsi_dma
            .transfer_address(request.write.is_some())
            .map_err(error)?
        else {
            return Ok(());
        };
        let result = match request.write {
            Some(byte) => self
                .dma_write(5, address, Width::Byte, u32::from(byte))
                .map(|()| 0),
            None => self.dma_read(5, address, Width::Byte),
        };
        let devices = self.bus.devices_mut().expect("installed SCSI");
        match result {
            Ok(value) => {
                devices.esp.validate_dma(value as u8).map_err(error)?;
                devices.esp.complete_dma(value as u8);
                devices.scsi_dma.complete();
            }
            Err(DmaError::NotGranted { .. }) => {}
            Err(DmaError::Fault { .. }) => devices.scsi_dma.memory_error(),
            Err(DmaError::Bus(e)) if !e.kind.is_host_stop() => devices.scsi_dma.memory_error(),
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// Advance at most one MAC DMA transfer per CPU boundary. Device work is
    /// retained across arbiter denial and host stops, without moving CPU state.
    pub fn service_ethernet(&mut self) -> Result<(), DmaError> {
        let Some(devices) = self.bus.devices_mut() else {
            return Ok(());
        };
        if devices.ethernet_dma.reset_asserted() {
            return Ok(());
        }
        let Some(request) = devices.ethernet.request() else {
            return Ok(());
        };
        let address = devices.ethernet_dma.address(request.address);
        // microSPARC-II's onboard NCR89C100 is SBus master 5.
        let result = match request.write {
            Some(value) => self.dma_write(5, address, request.width, value).map(|()| 0),
            None => self.dma_read(5, address, request.width),
        };
        let devices = self.bus.devices_mut().expect("installed Ethernet");
        match result {
            Ok(value) => devices.ethernet.complete(value).map_err(|kind| {
                DmaError::Bus(BusError {
                    address: u64::from(address),
                    kind,
                })
            }),
            Err(DmaError::NotGranted { .. }) => Ok(()),
            Err(DmaError::Fault { .. }) => {
                devices.ethernet_dma.memory_error();
                devices.ethernet.memory_error();
                Ok(())
            }
            Err(DmaError::Bus(error)) if !error.kind.is_host_stop() => {
                devices.ethernet_dma.memory_error();
                devices.ethernet.memory_error();
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Read one naturally aligned byte, halfword or word from guest RAM.
    /// No CPU execution or time advancement; values remain guest bit patterns.
    pub fn dma_read(&mut self, master: u8, address: u32, width: Width) -> Result<u32, DmaError> {
        let physical = self.dma_address(master, address, width, false)?;
        match width {
            Width::Byte => self.bus.read8(physical).map(u32::from),
            Width::Half => self.bus.read16(physical).map(u32::from),
            Width::Word => self.bus.read32(physical),
        }
        .map_err(DmaError::Bus)
    }

    /// Write one transfer. Denied mappings and failed target writes leave RAM
    /// unchanged. Multi-transfer device engines must track partial completion.
    pub fn dma_write(
        &mut self,
        master: u8,
        address: u32,
        width: Width,
        value: u32,
    ) -> Result<(), DmaError> {
        let physical = self.dma_address(master, address, width, true)?;
        match width {
            Width::Byte => self.bus.write8(physical, value as u8),
            Width::Half => self.bus.write16(physical, value as u16),
            Width::Word => self.bus.write32(physical, value),
        }
        .map_err(DmaError::Bus)
    }

    fn dma_address(
        &mut self,
        master: u8,
        address: u32,
        width: Width,
        write: bool,
    ) -> Result<u64, DmaError> {
        let alignment = match width {
            Width::Byte => 1,
            Width::Half => 2,
            Width::Word => 4,
        };
        if address % alignment != 0 {
            return Err(DmaError::Fault {
                address,
                kind: DmaFault::Misaligned,
            });
        }
        self.bus.set_cpu_supervisor(self.cpu.psr() & 0x80 != 0);
        let mapping = self
            .bus
            .devices()
            .ok_or(DmaError::Unsupported("no SBus/IOMMU installed"))?
            .io_control
            .mapping(master, address)?;
        let physical = self
            .cpu
            .mmu
            .translate_io(&mut self.bus, address, write, mapping)?;
        if physical >= 0x1000_0000 {
            return Err(DmaError::Unsupported("non-DRAM DVMA target"));
        }
        let kind = match (write, width) {
            (false, _) => MemoryRequestKind::Read,
            (true, Width::Byte) => MemoryRequestKind::WriteByte,
            (true, Width::Half) => MemoryRequestKind::WriteHalf,
            (true, Width::Word) => MemoryRequestKind::WriteWord,
        };
        self.bus.observe_memory_request(
            physical,
            MemoryRequest {
                kind,
                cpu: false,
                boot_mode: self.cpu.mmu.control() & crate::mmu::PCR_BM != 0,
                cacheable: false,
            },
        );
        Ok(physical)
    }
}
