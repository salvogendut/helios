use helios::bus::{Bus, MemoryBus};
use helios::mmu::{MmuConfig, Srmmu, Width, PCR_EN};

const VA: u32 = 0x0104_0000;
const DATA: u32 = 0x1234_5678;
// The existing reference view fills only zero words. Ordinary reads set R,
// clear M in the diagnostic PTE, and retain nonzero diagnostic edits.
const DIAG_PTE: u32 = 0xae;
const KEYS: [u32; 6] = [0x1000, 0x1100, 0x1200, 4, 0x104, 0x204];

fn store(m: &mut Srmmu, b: &mut MemoryBus, address: u32, asi: u8, value: u32) {
    m.store(b, address, asi, Width::Word, value).unwrap();
}
fn load(m: &mut Srmmu, b: &mut MemoryBus, address: u32, asi: u8) -> u32 {
    m.load(b, address, asi, Width::Word).unwrap()
}
fn fixture(context: u32) -> (Srmmu, MemoryBus) {
    let mut b = MemoryBus::default();
    b.map_ram(0, 0x10_0000).unwrap();
    for c in 0..3 {
        b.write32(0x1000 + c * 4, 0x201).unwrap();
    }
    for region in 0..3 {
        b.write32(0x2000 + region * 4, 0x8e).unwrap();
    }
    b.write32(0x40000, DATA).unwrap();
    b.write32(0x80000, DATA).unwrap();
    let mut m = Srmmu::reset_with(MmuConfig::ss20());
    store(&mut m, &mut b, 0x100, 4, 0x100);
    store(&mut m, &mut b, 0x200, 4, context);
    // Freeze architectural replacement in slot zero, keeping its normal-fill
    // view separate from the region-one view exercised here.
    store(&mut m, &mut b, 0x1000, 4, (63 << 7) | 0x40);
    store(&mut m, &mut b, 0, 4, PCR_EN);
    (m, b)
}
fn view(m: &mut Srmmu, b: &mut MemoryBus) -> [u32; 6] {
    KEYS.map(|key| load(m, b, key, 6))
}
fn expected(context: u32) -> [u32; 6] {
    [VA, context, DIAG_PTE, VA, context, DIAG_PTE]
}

#[test]
fn warm_hits_keep_both_diagnostic_views_and_guest_data() {
    let (mut m, mut b) = fixture(1);
    assert_eq!(load(&mut m, &mut b, VA, 11), DATA);
    assert_eq!(view(&mut m, &mut b), expected(1));
    for _ in 0..20 {
        assert_eq!(load(&mut m, &mut b, VA, 11), DATA);
        assert_eq!(view(&mut m, &mut b), expected(1));
    }
    assert_eq!(b.read32(0x2004).unwrap(), DIAG_PTE);
}

#[test]
fn zero_context_can_fill_later_without_overwriting_nonzero_context() {
    let (mut m, mut b) = fixture(0);
    load(&mut m, &mut b, VA, 11);
    assert_eq!(view(&mut m, &mut b), expected(0));
    store(&mut m, &mut b, 0x200, 4, 1);
    load(&mut m, &mut b, VA, 11);
    assert_eq!(view(&mut m, &mut b), expected(1));
    store(&mut m, &mut b, 0x200, 4, 2);
    load(&mut m, &mut b, VA, 11);
    assert_eq!(view(&mut m, &mut b), expected(1));
}

#[test]
fn zero_tag_can_fill_later_and_nonzero_region_alias_is_retained() {
    let (mut m, mut b) = fixture(0);
    load(&mut m, &mut b, 0, 11);
    assert_eq!(load(&mut m, &mut b, 0, 6), 0);
    load(&mut m, &mut b, 0x0100_0000, 11);
    assert_eq!(load(&mut m, &mut b, 0, 6), 0x0100_0000);
    load(&mut m, &mut b, VA, 11);
    assert_eq!(view(&mut m, &mut b), expected(0));
    load(&mut m, &mut b, 0x0204_0000, 11);
    assert_eq!(view(&mut m, &mut b), expected(0));
}

#[test]
fn word_diagnostic_edits_refill_zero_and_retain_nonzero_aliases() {
    let (mut m, mut b) = fixture(1);
    load(&mut m, &mut b, VA, 11);
    for key in KEYS {
        store(&mut m, &mut b, key, 6, 0);
        assert_eq!(load(&mut m, &mut b, key, 6), 0);
        assert_eq!(load(&mut m, &mut b, VA, 11), DATA);
        assert_eq!(view(&mut m, &mut b), expected(1));
    }
    store(&mut m, &mut b, 0x1000, 6, 0xdead_0000);
    store(&mut m, &mut b, 4, 6, 0xcafe_0000);
    assert_eq!(load(&mut m, &mut b, VA, 11), DATA);
    let mut edited = expected(1);
    edited[0] = 0xdead_0000;
    edited[3] = 0xcafe_0000;
    assert_eq!(view(&mut m, &mut b), edited);
}

#[test]
fn doubleword_diagnostic_edits_invalidate_both_alias_paths() {
    for address in [0, 0x1000] {
        let (mut m, mut b) = fixture(1);
        load(&mut m, &mut b, VA, 11);
        m.store_doubleword(&mut b, address, 6, [0, 0]).unwrap();
        let cleared_key = if address == 0 { 4 } else { 0x1000 };
        assert_eq!(load(&mut m, &mut b, cleared_key, 6), 0);
        load(&mut m, &mut b, VA, 11);
        assert_eq!(view(&mut m, &mut b), expected(1));
    }
}

#[test]
fn every_cpu_flush_scope_allows_a_cleared_diagnostic_word_to_refill() {
    for flush in [VA, VA | 0x100, VA | 0x200, 0x300, 0x400] {
        let (mut m, mut b) = fixture(1);
        load(&mut m, &mut b, VA, 11);
        // This explicit diagnostic value becomes zero when its V bit is
        // cleared by demap. Warm the view before demap to expose stale caches.
        store(&mut m, &mut b, 0x1200, 6, 0x20);
        load(&mut m, &mut b, VA, 11);
        assert_eq!(load(&mut m, &mut b, 0x1200, 6), 0x20);
        store(&mut m, &mut b, flush, 3, 0);
        assert_eq!(load(&mut m, &mut b, 0x1200, 6), 0, "{flush:#x}");
        load(&mut m, &mut b, VA, 11);
        assert_eq!(view(&mut m, &mut b), expected(1), "{flush:#x}");
    }
}

#[test]
fn instruction_reads_do_not_fill_data_views_and_reset_clears_views() {
    let (mut m, mut b) = fixture(1);
    assert_eq!(load(&mut m, &mut b, VA, 9), DATA);
    assert_eq!(view(&mut m, &mut b), [0; 6]);
    assert_eq!(load(&mut m, &mut b, VA, 11), DATA);
    assert_eq!(view(&mut m, &mut b), expected(1));
    m = Srmmu::reset_with(MmuConfig::ss20());
    assert_eq!(view(&mut m, &mut b), [0; 6]);
    store(&mut m, &mut b, 0x100, 4, 0x100);
    store(&mut m, &mut b, 0x200, 4, 1);
    store(&mut m, &mut b, 0, 4, PCR_EN);
    assert_eq!(load(&mut m, &mut b, VA, 11), DATA);
    assert_eq!(view(&mut m, &mut b), expected(1));
}

#[cfg(feature = "perf-counters")]
#[test]
fn warm_view_avoids_fill_work_but_counts_each_translation() {
    let (mut m, mut b) = fixture(0);
    let before = m.translation_counters();
    load(&mut m, &mut b, VA, 11);
    let first = m.translation_counters();
    assert_eq!(first.diagnostic_updates - before.diagnostic_updates, 1);
    for _ in 0..10 {
        load(&mut m, &mut b, VA, 11);
    }
    let warm = m.translation_counters();
    assert_eq!(warm.data_requests - first.data_requests, 10);
    assert_eq!(warm.fast_hits - first.fast_hits, 10);
    assert_eq!(warm.diagnostic_updates, first.diagnostic_updates);
    assert_eq!(view(&mut m, &mut b), expected(0));
}
