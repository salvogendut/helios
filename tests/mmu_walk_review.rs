use helios::bus::{Bus, BusError, BusErrorKind, MemoryBus};
use helios::machine::Machine;
use helios::mmu::{MemoryError, Srmmu, Width, PCR_EN};

const VA: u32 = 0x1234_5000;
const OLD: u32 = 0x1111_1111;
const NEW: u32 = 0x2222_2222;
const PEER: u32 = 0x3333_3333;

struct FaultBus {
    inner: MemoryBus,
    fail_once: Option<(u64, BusErrorKind)>,
    fail_always: Option<(u64, BusErrorKind)>,
    reads: Vec<u64>,
}

impl Bus for FaultBus {
    fn read32(&mut self, address: u64) -> Result<u32, BusError> {
        self.reads.push(address);
        if let Some((at, kind)) = self.fail_once {
            if at == address {
                self.fail_once = None;
                return Err(BusError { address, kind });
            }
        }
        if let Some((at, kind)) = self.fail_always {
            if at == address {
                return Err(BusError { address, kind });
            }
        }
        self.inner.read32(address)
    }
    fn write32_masked(&mut self, address: u64, value: u32, mask: u32) -> Result<(), BusError> {
        self.inner.write32_masked(address, value, mask)
    }
    fn read_doubleword(&mut self, address: u64) -> Result<[u32; 2], BusError> {
        self.inner.read_doubleword(address)
    }
    fn write_doubleword(&mut self, address: u64, words: [u32; 2]) -> Result<(), BusError> {
        self.inner.write_doubleword(address, words)
    }
    fn swap32(&mut self, address: u64, value: u32) -> Result<u32, BusError> {
        self.inner.swap32(address, value)
    }
    fn ldstub8(&mut self, address: u64) -> Result<u8, BusError> {
        self.inner.ldstub8(address)
    }
}

fn reg(m: &mut Srmmu, b: &mut impl Bus, n: u32, value: u32) {
    m.store(b, n << 8, 4, Width::Word, value).unwrap();
}

fn fixture(enabled: bool) -> (Srmmu, FaultBus) {
    let mut inner = MemoryBus::default();
    inner.map_ram(0, 0x20000).unwrap();
    // Both trees already exist. The leaf is pre-referenced to avoid an R-bit
    // difference accidentally forcing a full walk instead of the cached path.
    for (address, word) in [
        (0x1000, 0x201),
        (0x2048, 0x301),
        (0x3034, 0x401),
        (0x4014, 0x8ae),
        (0x4018, 0xaae),
        (0x7048, 0x601),
        (0x6034, 0x501),
        (0x5014, 0x9ae),
        (0x5018, 0xaae),
        (0x8000, OLD),
        (0x9000, NEW),
        (0xa000, PEER),
    ] {
        inner.write32(address, word).unwrap();
    }
    let mut b = FaultBus {
        inner,
        fail_once: None,
        fail_always: None,
        reads: Vec::new(),
    };
    let mut m = Srmmu::default();
    reg(&mut m, &mut b, 1, 0x100);
    reg(&mut m, &mut b, 0, PCR_EN);
    reg(&mut m, &mut b, 0x10, 0); // Architectural fills only replace slot 0.
    m.set_walk_cache_enabled(enabled);
    (m, b)
}

fn prime_and_evict(m: &mut Srmmu, b: &mut FaultBus) {
    for _ in 0..2 {
        assert_eq!(m.load(b, VA, 11, Width::Word), Ok(OLD));
        assert_eq!(m.load(b, VA + 0x1000, 11, Width::Word), Ok(PEER));
    }
    // The old translation is no longer in the architectural TLB.
    assert_eq!(
        m.load(b, 0x300, 6, Width::Word).unwrap() & 0xffff_f000,
        VA + 0x1000
    );
    for slot in 1..64 {
        assert_eq!(m.load(b, 0x300 + slot * 4, 6, Width::Word).unwrap() & 8, 0);
    }
    b.reads.clear();
}

#[test]
fn ancestor_retarget_after_real_tlb_eviction_matches_uncached_walk() {
    let mut outputs = Vec::new();
    for (address, value) in [(0x1000, 0x701), (0x2048, 0x601), (0x3034, 0x501)] {
        for enabled in [false, true] {
            let (mut m, mut b) = fixture(enabled);
            prime_and_evict(&mut m, &mut b);
            // An ordinary CPU bypass write changes the context, L1 or L2
            // pointer. No architectural PTP entry exists in this fixture.
            m.store(&mut b, address, 0x20, Width::Word, value).unwrap();
            let result = m.load(&mut b, VA, 11, Width::Word);
            outputs.push((address, enabled, result));
        }
    }
    assert!(
        outputs.iter().all(|(_, _, r)| *r == Ok(NEW)),
        "{outputs:#x?}"
    );
}

#[test]
fn diagnostic_tlb_invalidation_cannot_resurrect_old_ancestor() {
    let mut outputs = Vec::new();
    for enabled in [false, true] {
        let (mut m, mut b) = fixture(enabled);
        assert_eq!(m.load(&mut b, VA, 11, Width::Word), Ok(OLD));
        m.store(&mut b, 0x3034, 0x20, Width::Word, 0x501).unwrap();
        // Explicitly invalidate the sole valid TLB slot through ASI 6.
        m.store(&mut b, 0x300, 6, Width::Word, 0).unwrap();
        outputs.push((enabled, m.load(&mut b, VA, 11, Width::Word)));
    }
    assert!(outputs.iter().all(|(_, r)| *r == Ok(NEW)), "{outputs:#x?}");
}

fn leaf_error(kind: BusErrorKind) {
    let mut results = Vec::new();
    for enabled in [false, true] {
        let (mut m, mut b) = fixture(enabled);
        prime_and_evict(&mut m, &mut b);
        b.fail_once = Some((0x4014, kind));
        let result = m.load(&mut b, VA, 11, Width::Word);
        results.push((result, m.fault_status(), b.reads));
    }
    assert!(results[0].0.is_err());
    assert_eq!(
        results[1].0, results[0].0,
        "off/on result, status and bus read sequence: {results:#x?}"
    );
    assert_eq!(results[1].1, results[0].1);
}

#[test]
fn one_shot_leaf_bus_fault_is_not_swallowed_by_cache_validation() {
    leaf_error(BusErrorKind::SBusTimeout);
}

#[test]
fn leaf_host_stop_is_not_swallowed_by_cache_validation() {
    leaf_error(BusErrorKind::Unsupported("review injected PTE read stop"));
}

#[test]
fn ancestor_bus_faults_are_preserved_after_tlb_eviction() {
    let mut results = Vec::new();
    for address in [0x1000, 0x2048, 0x3034] {
        for enabled in [false, true] {
            let (mut m, mut b) = fixture(enabled);
            prime_and_evict(&mut m, &mut b);
            b.fail_always = Some((address, BusErrorKind::SBusError));
            results.push((
                address,
                enabled,
                m.load(&mut b, VA, 11, Width::Word),
                m.fault_status(),
            ));
        }
    }
    assert!(
        results
            .iter()
            .all(|(_, _, r, _)| matches!(r, Err(MemoryError::Translation(_)))),
        "{results:#x?}"
    );
}

#[test]
fn host_walk_cache_toggle_is_not_an_architectural_snapshot_difference() {
    let mut a = Machine::bare_metal(0x1000, 0).unwrap();
    let b = Machine::bare_metal(0x1000, 0).unwrap();
    assert!(a
        .diagnostic_snapshot()
        .differences(&b.diagnostic_snapshot())
        .is_empty());
    a.cpu.mmu.set_walk_cache_enabled(false);
    assert!(
        a.diagnostic_snapshot()
            .differences(&b.diagnostic_snapshot())
            .is_empty(),
        "host-only cache policy makes identical guest state compare unequal"
    );
}

#[test]
fn leaf_retarget_and_explicit_flush_controls_still_work() {
    for enabled in [false, true] {
        let (mut m, mut b) = fixture(enabled);
        prime_and_evict(&mut m, &mut b);
        m.store(&mut b, 0x4014, 0x20, Width::Word, 0x9ae).unwrap();
        assert_eq!(m.load(&mut b, VA, 11, Width::Word), Ok(NEW));

        let (mut m, mut b) = fixture(enabled);
        prime_and_evict(&mut m, &mut b);
        m.store(&mut b, 0x3034, 0x20, Width::Word, 0x501).unwrap();
        m.store(&mut b, VA, 3, Width::Word, 0).unwrap();
        assert_eq!(m.load(&mut b, VA, 11, Width::Word), Ok(NEW));
    }
}
