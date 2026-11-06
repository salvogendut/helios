//! Single-transfer SBus DVMA interface for future device engines.
use crate::bus::{Bus, BusError};
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
        Ok(physical)
    }
}
