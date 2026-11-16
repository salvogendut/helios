//! Deterministic read-only fingerprints for differential validation.

use std::fmt::Debug;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) struct Digest(u64);

impl Digest {
    pub(crate) fn new() -> Self {
        Self(FNV_OFFSET)
    }

    pub(crate) fn byte(&mut self, value: u8) {
        self.0 ^= u64::from(value);
        self.0 = self.0.wrapping_mul(FNV_PRIME);
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(byte);
        }
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn debug<T: Debug>(&mut self, value: &T) {
        self.bytes(format!("{value:?}").as_bytes());
    }

    pub(crate) fn finish(self) -> u64 {
        self.0
    }
}

pub(crate) fn digest_debug<T: Debug>(value: &T) -> u64 {
    let mut digest = Digest::new();
    digest.debug(value);
    digest.finish()
}
