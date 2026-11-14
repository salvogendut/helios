//! Standalone sun4m IOMMU translation cache for SS-20/Axil.
//!
//! Sun-4M System Architecture section 7.2 defines a one-level IOPTE table, a
//! 16-entry LRU translation cache, page/all flushes and 36-bit physical PPNs.
//! The SS-5 microSPARC-II instead shares its CPU/I/O TLB and remains in
//! `Srmmu::translate_io`. This owner has no CPU context, diagnostics, fast
//! caches or replacement state.

use crate::bus::{Bus, IoTlbFlush, MemoryRequest, MemoryRequestKind};
use crate::devices::io_control::IoMapping;
use crate::dma::{DmaError, DmaFault};

const CACHE_ENTRIES: usize = 16;
const IOPTE_PPN: u32 = 0xffff_ff00;
const IOPTE_WRITE: u32 = 1 << 2;
const IOPTE_VALID: u32 = 1 << 1;
const DRAM_LIMIT: u64 = 0x1000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CacheEntry {
    page: u32,
    pte: u32,
    used: u64,
}

/// Authoritative I/O translation state for the MBus profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandaloneIommu {
    entries: [Option<CacheEntry>; CACHE_ENTRIES],
    clock: u64,
}

impl Default for StandaloneIommu {
    fn default() -> Self {
        Self {
            entries: [None; CACHE_ENTRIES],
            clock: 0,
        }
    }
}

impl StandaloneIommu {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Translate one naturally aligned DVMA address. The caller has already
    /// applied the IOMMU control/slot mapping and transfer-width alignment.
    pub(crate) fn translate(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        write: bool,
        mapping: IoMapping,
    ) -> Result<u64, DmaError> {
        // Device requests have no CPU privilege or boot-mode attribute.
        bus.set_cpu_supervisor(false);
        self.sync_flushes(bus);
        let pte_address = match mapping {
            IoMapping::Bypass(physical) => return Ok(physical),
            IoMapping::PageTable(physical) => physical,
        };
        let page = address & !0xfff;
        let pte = if let Some(pte) = self.lookup(page) {
            pte
        } else {
            if pte_address >= DRAM_LIMIT {
                return Err(DmaError::Unsupported("I/O page table outside DRAM"));
            }
            bus.observe_memory_request(
                pte_address,
                MemoryRequest {
                    kind: MemoryRequestKind::Read,
                    cpu: false,
                    boot_mode: false,
                    cacheable: false,
                },
            );
            let pte = bus.read32(pte_address).map_err(DmaError::Bus)?;
            if pte & IOPTE_VALID == 0 {
                return Err(DmaError::Fault {
                    address,
                    kind: DmaFault::InvalidPte,
                });
            }
            self.insert(page, pte);
            pte
        };
        if write && pte & IOPTE_WRITE == 0 {
            return Err(DmaError::Fault {
                address,
                kind: DmaFault::WriteProtected,
            });
        }
        Ok((u64::from(pte & IOPTE_PPN) << 4) | u64::from(address & 0xfff))
    }

    fn lookup(&mut self, page: u32) -> Option<u32> {
        self.clock = self.clock.wrapping_add(1);
        let now = self.clock;
        for entry in self.entries.iter_mut().flatten() {
            if entry.page == page {
                entry.used = now;
                return Some(entry.pte);
            }
        }
        None
    }

    fn insert(&mut self, page: u32, pte: u32) {
        self.clock = self.clock.wrapping_add(1);
        let candidate = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.as_ref().map(|entry| (entry.used, index)))
            .min_by_key(|&(used, _)| used)
            .map(|(_, index)| index);
        let index = if let Some(index) = self.entries.iter().position(Option::is_none) {
            index
        } else {
            candidate.unwrap_or(0)
        };
        self.entries[index] = Some(CacheEntry {
            page,
            pte,
            used: self.clock,
        });
    }

    fn sync_flushes(&mut self, bus: &mut impl Bus) {
        while let Some(flush) = bus.take_standalone_io_tlb_flush() {
            match flush {
                IoTlbFlush::All => self.entries.fill(None),
                IoTlbFlush::Page(page) => {
                    // Sun-4M 7.2.4.2: FA bits 30:12 select the page; bit 31 is
                    // reserved. Normalize both the queued tag and cache page.
                    let page = page & 0x7fff_f000;
                    for entry in &mut self.entries {
                        if entry.is_some_and(|entry| entry.page & 0x7fff_f000 == page) {
                            *entry = None;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::MemoryBus;

    #[test]
    fn page_table_bus_failure_remains_a_device_dma_error() {
        let mut bus = MemoryBus::default();
        let mut iommu = StandaloneIommu::default();
        let result = iommu.translate(&mut bus, 0xff00_0000, false, IoMapping::PageTable(0x1000));
        assert!(matches!(
            result,
            Err(DmaError::Bus(error))
                if error.address == 0x1000 && error.kind == crate::bus::BusErrorKind::Unmapped
        ));
    }

    #[test]
    fn bypass_does_not_populate_the_standalone_cache() {
        let mut bus = MemoryBus::default();
        let mut iommu = StandaloneIommu::default();
        assert_eq!(
            iommu.translate(&mut bus, 0xff00_0000, false, IoMapping::Bypass(0x8000)),
            Ok(0x8000)
        );
        assert!(iommu.entries.iter().all(Option::is_none));
    }
}
