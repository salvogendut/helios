//! PCI HME descriptor rings. Descriptor words are little-endian, packet bytes
//! are in wire order. The CPU observes DMA writes at its next board boundary.
use super::Ultra5ResetBoard;
use crate::net::packet::checksum;

const OWN: u32 = 1 << 31;
const SOP: u32 = 1 << 30;
const EOP: u32 = 1 << 29;
const FIFO: usize = 2048;
const RX_TO_HOST: u32 = 1 << 16;
const NO_RX_DESCRIPTOR: u32 = 1 << 17;
const MISSING_EOP: u32 = 1 << 22;
const HOST_TO_TX: u32 = 1 << 24;
const TX_ALL: u32 = 1 << 25;
const SENT_FRAME: u32 = 1 << 8;
const GOT_FRAME: u32 = 1;
const TX_ERROR: u32 = 1 << 26;
const RX_ERROR: u32 = 1 << 18;
const XIF_LOOPBACK: u32 = 1 << 1;

impl Ultra5ResetBoard {
    /// Connect the internal PHY to a virtual Ethernet segment.
    pub fn set_ethernet_link(&mut self, connected: bool) {
        self.hme.link = connected;
        if !connected {
            self.hme.outgoing.clear();
        }
    }

    /// Take one transmitted Ethernet frame, without its wire FCS.
    pub fn take_ethernet_frame(&mut self) -> Option<Vec<u8>> {
        self.hme.outgoing.pop_front()
    }

    // A descriptor or one HME FIFO can span at most two 8 KiB I/O pages.
    // Resolve both before changing memory, including DMA permissions.
    fn hme_transfer(&mut self, va: u32, bytes: &mut [u8], write: bool) -> Result<(), String> {
        debug_assert!(bytes.len() <= FIFO);
        if bytes.is_empty() {
            return Ok(());
        }
        let first = bytes.len().min(8192 - (va as usize & 8191));
        let pa = self.dma_page(va, write)?;
        let next = if first < bytes.len() {
            Some(
                self.dma_page(
                    va.checked_add(first as u32)
                        .ok_or("HME DMA address overflow")?,
                    write,
                )?,
            )
        } else {
            None
        };
        self.dram
            .dma_bytes(pa, self.mem_control0, &mut bytes[..first], write)?;
        if write {
            self.dma_writes.push((pa, first));
        }
        if let Some(pa) = next {
            self.dram
                .dma_bytes(pa, self.mem_control0, &mut bytes[first..], write)?;
            if write {
                self.dma_writes.push((pa, bytes.len() - first));
            }
        }
        Ok(())
    }

    fn hme_descriptor(&mut self, ring: u32, index: u32) -> Result<(u32, u32, u32), String> {
        let va = ring
            .checked_add(index * 8)
            .ok_or("HME descriptor address overflow")?;
        let mut data = [0; 8];
        self.hme_transfer(va, &mut data, false)?;
        let flags = u32::from_le_bytes(data[..4].try_into().unwrap());
        let buffer = u32::from_le_bytes(data[4..].try_into().unwrap());
        if flags & OWN != 0 {
            self.dma_page(va, true)?;
        }
        Ok((va, flags, buffer))
    }

    pub(super) fn hme_transmit(&mut self) {
        if !self.pci_functions[1].master_enabled()
            || self.hme.reg(0x2004) & 1 == 0
            || self.hme.reg(0x620c) & 1 == 0
        {
            return;
        }
        if self.hme_tx_ring().is_err() {
            self.hme.assembling = None;
            self.hme.status |= TX_ERROR;
        }
    }

    fn hme_tx_ring(&mut self) -> Result<(), String> {
        let count = (self.hme.reg(0x202c) + 1) * 16;
        let ring = self.hme.reg(0x2008);
        for _ in 0..count {
            let (desc, flags, addr) = self.hme_descriptor(ring, self.hme.tx_index)?;
            if flags & OWN == 0 {
                break;
            }
            let len = (flags & 0x3fff) as usize;
            let mut frame = if flags & SOP != 0 {
                if self.hme.assembling.is_some() {
                    self.hme.status |= MISSING_EOP;
                }
                self.hme.checksum_offsets = (flags & (1 << 28) != 0).then_some((
                    ((flags >> 14) & 63) as usize,
                    ((flags >> 20) & 255) as usize,
                ));
                Vec::new()
            } else {
                self.hme
                    .assembling
                    .take()
                    .ok_or("HME missing start of packet")?
            };
            let start = frame.len();
            if start + len > FIFO {
                return Err("HME TX FIFO overflow".into());
            }
            frame.resize(start + len, 0);
            self.hme_transfer(addr, &mut frame[start..], false)?;
            if flags & EOP != 0 {
                if let Some((from, to)) = self.hme.checksum_offsets {
                    let sum = checksum(frame.get(from..).ok_or("HME TX checksum start")?);
                    frame
                        .get_mut(to..to + 2)
                        .ok_or("HME TX checksum offset")?
                        .copy_from_slice(&sum.to_be_bytes());
                }
            }
            self.hme_transfer(desc, &mut (flags & !OWN).to_le_bytes(), true)?;
            self.hme.tx_index = (self.hme.tx_index + 1) % count;
            self.hme.status |= HOST_TO_TX;
            if flags & EOP != 0 {
                if self.hme.reg(0x6000) & XIF_LOOPBACK != 0 {
                    self.hme_accept_rx(&frame);
                } else if self.hme.carrier() && self.hme.outgoing.len() < 256 {
                    frame.resize(frame.len().max(60), 0);
                    self.hme.outgoing.push_back(frame);
                }
                self.hme.status |= TX_ALL | SENT_FRAME;
                self.hme.assembling = None;
            } else {
                self.hme.assembling = Some(frame);
            }
        }
        Ok(())
    }

    /// Offer one wire frame (without its FCS) to the guest receive DMA engine.
    pub fn receive_ethernet_frame(&mut self, frame: &[u8]) -> bool {
        self.hme.carrier() && self.hme.reg(0x6000) & XIF_LOOPBACK == 0 && self.hme_accept_rx(frame)
    }

    fn hme_accept_rx(&mut self, frame: &[u8]) -> bool {
        if !self.pci_functions[1].master_enabled()
            || self.hme.reg(0x4000) & 1 == 0
            || self.hme.reg(0x630c) & 1 == 0
            || frame.len() < 14
            || frame.len() > FIFO - 4
            || !self.hme.accepts(frame)
        {
            return false;
        }
        match self.hme_receive(frame) {
            Ok(accepted) => accepted,
            Err(_) => {
                self.hme.status |= RX_ERROR;
                false
            }
        }
    }

    fn hme_receive(&mut self, frame: &[u8]) -> Result<bool, String> {
        let cfg = self.hme.reg(0x4000);
        let (desc, flags, buffer) = self.hme_descriptor(self.hme.reg(0x4004), self.hme.rx_index)?;
        if flags & OWN == 0 {
            self.hme.status |= NO_RX_DESCRIPTOR;
            return Ok(false);
        }
        let mut packet = frame.to_vec();
        packet.resize(packet.len().max(60), 0);
        let sum = checksum(
            packet
                .get(((cfg >> 16 & 127) * 2) as usize..)
                .unwrap_or_default(),
        );
        if self.hme.reg(0x630c) & 0x100 != 0 {
            let fcs = !crate::devices::ethernet::crc(&packet);
            packet.extend_from_slice(&fcs.to_le_bytes());
        }
        let offset = (cfg >> 3) & 7;
        let capacity = ((flags >> 16) & 0x3fff) as usize;
        let len = packet.len().min(capacity);
        let overflow = u32::from(len < packet.len()) << 30;
        self.hme_transfer(
            buffer
                .checked_add(offset)
                .ok_or("HME RX address overflow")?,
            &mut packet[..len],
            true,
        )?;
        let result = overflow | ((len as u32) << 16) | u32::from(sum);
        self.hme_transfer(desc, &mut result.to_le_bytes(), true)?;
        self.hme.rx_index = (self.hme.rx_index + 1) % (32 << ((cfg >> 9) & 3));
        self.hme.status |= RX_TO_HOST | GOT_FRAME;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ultra5::{BootFlash, ResetNvram, FLASH_SIZE, RESET_NVRAM_SIZE};

    const RING: u32 = 0x1000_4000;
    const BUFFER: u32 = 0x1000_6000;
    fn fixture() -> Ultra5ResetBoard {
        let mut b = Ultra5ResetBoard::new(
            BootFlash::new(vec![0; FLASH_SIZE]).unwrap(),
            ResetNvram::new(vec![0; RESET_NVRAM_SIZE]).unwrap(),
        );
        b.mem_control0 = 1 << 12;
        b.pbm.write(0x2028, 1).unwrap();
        b.pci_functions[1].write(4, [6]);
        b.set_ethernet_link(true);
        b
    }
    fn descriptor(b: &mut Ultra5ResetBoard, slot: u32, flags: u32, buf: u32) {
        b.dram
            .dma_bytes(
                u64::from(RING + slot * 8),
                b.mem_control0,
                &mut flags.to_le_bytes(),
                true,
            )
            .unwrap();
        b.dram
            .dma_bytes(
                u64::from(RING + slot * 8 + 4),
                b.mem_control0,
                &mut buf.to_le_bytes(),
                true,
            )
            .unwrap();
    }
    fn read<const N: usize>(b: &mut Ultra5ResetBoard, addr: u32) -> [u8; N] {
        let mut out = [0; N];
        b.dram
            .dma_bytes(u64::from(addr), b.mem_control0, &mut out, false)
            .unwrap();
        out
    }

    #[test]
    fn tx_spans_two_kicks_wraps_descriptors_and_writes_the_packet_checksum() {
        let mut b = fixture();
        for (off, val) in [(0x2004, 1), (0x620c, 1), (0x2008, RING)] {
            assert!(b.hme.write(off, val));
        }
        b.hme.tx_index = 15;
        descriptor(&mut b, 15, 0xd000_0000 | (2 << 20) | 4, BUFFER);
        b.dram
            .dma_bytes(
                u64::from(BUFFER),
                b.mem_control0,
                &mut [0x12, 0x34, 0, 0, 0x56, 0x78],
                true,
            )
            .unwrap();
        b.hme_transmit();
        assert_eq!(b.hme.tx_index, 0);
        assert!(b.take_ethernet_frame().is_none());
        descriptor(&mut b, 0, 0xa000_0002, BUFFER + 4);
        b.hme_transmit();
        let f = b.take_ethernet_frame().unwrap();
        assert_eq!(&f[..6], &[0x12, 0x34, 0x97, 0x53, 0x56, 0x78]);
        assert_eq!(f.len(), 60);
        assert_eq!(read::<4>(&mut b, RING), 0x2000_0002u32.to_le_bytes());
        assert_eq!(b.hme.tx_index, 1);
    }

    #[test]
    fn xif_loopback_routes_completed_tx_to_rx_even_with_the_link_unplugged() {
        let mut b = fixture();
        let rx_ring = RING + 0x800;
        let rx_buffer = BUFFER + 0x800;
        for (off, value) in [
            (0x2004, 1),
            (0x2008, RING),
            (0x4000, 0x201),
            (0x4004, rx_ring),
            (0x6000, 3),
            (0x620c, 0x401),
            (0x630c, 1),
        ] {
            assert!(b.hme.write(off, value));
        }
        let mut frame = [0x5a; 60];
        frame[..6].fill(0xff);
        b.dram
            .dma_bytes(u64::from(BUFFER), b.mem_control0, &mut frame, true)
            .unwrap();
        descriptor(&mut b, (rx_ring - RING) / 8, OWN | (64 << 16), rx_buffer);
        assert!(!b.receive_ethernet_frame(&frame));
        b.set_ethernet_link(false);
        descriptor(&mut b, 0, OWN | SOP | 24, BUFFER);
        b.hme_transmit();
        assert_eq!(b.hme.rx_index, 0);
        descriptor(&mut b, 1, OWN | EOP | 36, BUFFER + 24);
        b.hme_transmit();
        assert_eq!(b.hme.rx_index, 1);
        assert_eq!(u32::from_le_bytes(read(&mut b, rx_ring)) >> 16, 60);
        assert_eq!(read::<60>(&mut b, rx_buffer), frame);
        assert_eq!(
            b.hme.status,
            RX_TO_HOST | GOT_FRAME | HOST_TO_TX | TX_ALL | SENT_FRAME
        );
        assert!(b.take_ethernet_frame().is_none());

        b.set_ethernet_link(true);
        b.hme.write(0x6000, 1);
        descriptor(&mut b, 2, OWN | SOP | EOP | 60, BUFFER);
        b.hme_transmit();
        assert_eq!(b.take_ethernet_frame().as_deref(), Some(frame.as_slice()));
        assert_eq!(b.hme.rx_index, 1);
    }

    #[test]
    fn rx_uses_byte_offset_and_irq_mask_and_reports_starvation() {
        let mut b = fixture();
        let mut frame = [0; 60];
        frame[..6].fill(255);
        frame[14] = 0x42;
        b.hme.write(0x4000, 0x70011); // checksum at byte 14, two alignment bytes
        b.hme.write(0x4004, RING);
        b.hme.write(0x630c, 1);
        descriptor(&mut b, 0, 0x8100_0000, BUFFER - 4);
        assert!(b.receive_ethernet_frame(&frame));
        assert_eq!(read::<4>(&mut b, RING), [0xff, 0xbd, 60, 0]);
        assert_eq!(read::<2>(&mut b, BUFFER - 4), [0; 2]);
        assert_eq!(read::<60>(&mut b, BUFFER - 2), frame);
        assert!(!b.hme.interrupt());
        b.hme.write(0x104, !(1 << 16));
        b.pbm.write(0x1008, 1 << 31).unwrap();
        b.sample_interrupts();
        assert_eq!(b.pbm.pending_vector(), Some(0x21));
        assert_eq!(b.hme.read(0x100), Some(0x10001));
        assert!(!b.hme.interrupt());
        assert!(!b.receive_ethernet_frame(&frame));
        assert_eq!(b.hme.read(0x100), Some(1 << 17));
    }

    #[test]
    fn rx_filters_and_pci_master_enable_protect_the_ring() {
        let mut b = fixture();
        b.hme.write(0x4000, 1);
        b.hme.write(0x4004, RING);
        b.hme.write(0x630c, 1);
        b.hme.write(0x6320, 0x0800);
        b.hme.write(0x631c, 0x20c0);
        b.hme.write(0x6318, 0xffee);
        descriptor(&mut b, 0, 0x8100_0000, BUFFER);
        let mut f = [0x40; 60];
        assert!(!b.receive_ethernet_frame(&f));
        f[..6].copy_from_slice(&[8, 0, 32, 192, 255, 238]);
        b.pci_functions[1].write(4, [2]);
        assert!(!b.receive_ethernet_frame(&f));
        b.pci_functions[1].write(4, [6]);
        b.set_ethernet_link(false);
        assert!(!b.receive_ethernet_frame(&f));
        assert_eq!(read::<4>(&mut b, RING), 0x8100_0000u32.to_le_bytes());
        b.set_ethernet_link(true);
        assert!(b.receive_ethernet_frame(&f));
        assert_eq!(read::<60>(&mut b, BUFFER), f);
    }

    #[test]
    fn iommu_rejects_second_rx_page_before_writing_payload_or_ownership() {
        let mut b = fixture();
        let tsb = 0x1000_0000;
        b.pbm.write(0x2028, 0x80).unwrap();
        b.pbm.write(0x208, tsb).unwrap();
        b.pbm.write(0x200, 1).unwrap();
        // IOVA ff000000 -> ring (writable), ff002000 -> data (writable);
        // the next page is unmapped.
        for (slot, pa) in [(0, RING), (1, BUFFER)] {
            b.dram
                .write(
                    tsb + slot * 8,
                    b.mem_control0,
                    (1u64 << 63) | u64::from(pa) | 2,
                )
                .unwrap();
        }
        b.hme.write(0x4000, 1);
        b.hme.write(0x4004, 0xff00_0000);
        b.hme.write(0x630c, 1);
        descriptor(&mut b, 0, 0x8100_0000, 0xff00_3ffc);
        assert!(!b.receive_ethernet_frame(&[255; 60]));
        assert_eq!(b.hme.read(0x100), Some(1 << 18));
        assert_eq!(read::<4>(&mut b, BUFFER + 0x1ffc), [0; 4]);
        assert_eq!(read::<4>(&mut b, RING), 0x8100_0000u32.to_le_bytes());
        assert!(b.dma_writes.is_empty());
    }
}
