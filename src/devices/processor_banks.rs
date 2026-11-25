use crate::diagnostic::{Digest, Fingerprint};

/// Sun-4M's two-bit processor number selects a hardware bank. An empty
/// connector leaves its number unused even when a higher bank is installed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ProcessorBanks<T>([Option<T>; 4]);

impl<T> ProcessorBanks<T> {
    pub(super) fn install(&mut self, processor: u8) -> bool
    where
        T: Default,
    {
        let Some(slot @ None) = self.0.get_mut(usize::from(processor)) else {
            return false;
        };
        *slot = Some(T::default());
        true
    }

    pub(super) fn get(&self, processor: u8) -> Option<&T> {
        self.0.get(usize::from(processor))?.as_ref()
    }

    pub(super) fn get_mut(&mut self, processor: u8) -> Option<&mut T> {
        self.0.get_mut(usize::from(processor))?.as_mut()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (u8, &T)> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| slot.as_ref().map(|bank| (id as u8, bank)))
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = (u8, &mut T)> {
        self.0
            .iter_mut()
            .enumerate()
            .filter_map(|(id, slot)| slot.as_mut().map(|bank| (id as u8, bank)))
    }
}

impl<T: Fingerprint> Fingerprint for ProcessorBanks<T> {
    fn fingerprint(&self, digest: &mut Digest) {
        // Preserve the existing checkpoint encoding of the ordered bank map.
        digest.u64(self.iter().count() as u64);
        for (processor, bank) in self.iter() {
            digest.u8(processor);
            digest.fingerprint(bank);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn sparse_banks_preserve_hardware_numbers_and_checkpoint_encoding() {
        for population in 0u8..16 {
            let mut slots = ProcessorBanks::<u32>::default();
            let mut reference = BTreeMap::new();
            for id in (0u8..4).rev().filter(|id| population & (1 << id) != 0) {
                assert!(slots.install(id));
                let value = 0xf000_0000 | u32::from(id);
                *slots.get_mut(id).unwrap() = value;
                assert!(!slots.install(id));
                reference.insert(id, value);
            }
            for id in 4..=u8::MAX {
                assert!(!slots.install(id));
            }
            for id in 0..=u8::MAX {
                assert_eq!(slots.get(id), reference.get(&id));
            }
            let mut actual = Digest::new();
            let mut expected = Digest::new();
            slots.fingerprint(&mut actual);
            reference.fingerprint(&mut expected);
            assert_eq!(actual.finish(), expected.finish());
        }
    }
}
