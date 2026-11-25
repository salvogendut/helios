//! An IP-only smoltcp interface. The caller supplies one validated IP packet
//! at a time and wraps the stack's replies in the guest's Ethernet header.
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;

#[derive(Default)]
pub(super) struct Port {
    pub input: Option<Vec<u8>>,
    pub output: Vec<Vec<u8>>,
}

impl Device for Port {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = 1500;
        caps
    }

    fn receive(&mut self, _: Instant) -> Option<(Rx, Tx<'_>)> {
        self.input
            .take()
            .map(|packet| (Rx(packet), Tx(&mut self.output)))
    }
    fn transmit(&mut self, _: Instant) -> Option<Tx<'_>> {
        Some(Tx(&mut self.output))
    }
}

pub(super) struct Rx(Vec<u8>);
impl phy::RxToken for Rx {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, f: F) -> R {
        f(&mut self.0)
    }
}

pub(super) struct Tx<'a>(&'a mut Vec<Vec<u8>>);
impl phy::TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0; len];
        let result = f(&mut packet);
        self.0.push(packet);
        result
    }
}
