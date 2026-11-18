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

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.byte(u8::from(value));
    }

    pub(crate) fn fingerprint<T: Fingerprint + ?Sized>(&mut self, value: &T) {
        value.fingerprint(self);
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

pub(crate) trait Fingerprint {
    fn fingerprint(&self, digest: &mut Digest);
}

macro_rules! fingerprint_uint {
    ($($ty:ty),* $(,)?) => {
        $(impl Fingerprint for $ty {
            fn fingerprint(&self, digest: &mut Digest) {
                digest.bytes(&self.to_le_bytes());
            }
        })*
    };
}
fingerprint_uint!(u8, u16, u32, u64, i32, usize);

impl Fingerprint for bool {
    fn fingerprint(&self, digest: &mut Digest) {
        digest.byte(u8::from(*self));
    }
}

impl<T: Fingerprint> Fingerprint for Option<T> {
    fn fingerprint(&self, digest: &mut Digest) {
        digest.byte(u8::from(self.is_some()));
        if let Some(value) = self {
            value.fingerprint(digest);
        }
    }
}

impl<T: Fingerprint> Fingerprint for Vec<T> {
    fn fingerprint(&self, digest: &mut Digest) {
        digest.u64(self.len() as u64);
        for value in self {
            value.fingerprint(digest);
        }
    }
}

impl<T: Fingerprint> Fingerprint for std::collections::VecDeque<T> {
    fn fingerprint(&self, digest: &mut Digest) {
        digest.u64(self.len() as u64);
        for value in self {
            value.fingerprint(digest);
        }
    }
}

impl<T: Fingerprint, const N: usize> Fingerprint for [T; N] {
    fn fingerprint(&self, digest: &mut Digest) {
        digest.u64(N as u64);
        for value in self {
            value.fingerprint(digest);
        }
    }
}

impl<A: Fingerprint, B: Fingerprint> Fingerprint for (A, B) {
    fn fingerprint(&self, digest: &mut Digest) {
        self.0.fingerprint(digest);
        self.1.fingerprint(digest);
    }
}

impl<K: Fingerprint + Ord, V: Fingerprint> Fingerprint for std::collections::BTreeMap<K, V> {
    fn fingerprint(&self, digest: &mut Digest) {
        digest.u64(self.len() as u64);
        for (key, value) in self {
            key.fingerprint(digest);
            value.fingerprint(digest);
        }
    }
}
