use helios::devices::sx::{Sx, SX_BASE, SX_MEM_BASE, VSIMM_VRAM_BASE};

const SOURCE: usize = 0x2cdc4;
const COMMAND: u64 = SX_MEM_BASE + 0xff42_cdc0;
const BYTES: [u8; 14] = [
    0x12, 0x83, 0xe4, 0x05, 0x56, 0xa7, 0x38, 0xc9, 0x4a, 0xfb, 0x6c, 0x9d, 0x2e, 0x7f,
];

fn reg(sx: &mut Sx, r: usize, value: u32) {
    sx.write(SX_BASE + 0x100 + r as u64 * 4, value, u32::MAX)
        .unwrap();
}

fn fixture() -> Sx {
    let mut sx = Sx::new();
    sx.write(SX_BASE + 0x10, u32::MAX, u32::MAX).unwrap();
    sx.write(SX_BASE, 0x1000, u32::MAX).unwrap();
    for pixel in SOURCE - 4..SOURCE + 24 {
        sx.write(VSIMM_VRAM_BASE + pixel as u64 * 4, 0xaa5a_ccdd, u32::MAX)
            .unwrap();
    }
    for (i, byte) in BYTES.into_iter().enumerate() {
        sx.write(
            VSIMM_VRAM_BASE + (SOURCE + i) as u64 * 4,
            0xaa00_ccdd | ((byte as u32) << 16),
            u32::MAX,
        )
        .unwrap();
    }
    sx
}

fn expected(sx: &Sx, delta: usize) -> Vec<u8> {
    let mut result = sx.vram_bytes().to_vec();
    for (i, byte) in BYTES.into_iter().enumerate() {
        result[(SOURCE + delta + i) * 4 + 1] = byte;
    }
    result
}

fn check(sx: &Sx, expected: &[u8], delta: usize) {
    let actual_bytes: Vec<u8> = (0..14)
        .map(|i| sx.vram_byte((SOURCE + delta + i) * 4 + 1))
        .collect();
    assert_eq!(
        actual_bytes, BYTES,
        "rightward copy by {delta} pixels must not store the prefix lane(s)"
    );
    if let Some((offset, (actual, wanted))) = sx
        .vram_bytes()
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (actual, wanted))| actual != wanted)
    {
        panic!("VRAM byte {offset:#x}: {actual:#x} != {wanted:#x}; other channels and guards must survive");
    }
}

// This is a DDX-derived copy-contract fixture, not a captured post-funnel
// store: the guest stopped before executing that store. sl_rect_copyrl_8 at
// ELF 0x1a064 computes shift = 8 * (source alignment - destination alignment),
// adds 32 for a negative shift, loads R11, funnels R10, then stores R10 at the
// destination address. Thus an unaligned store must skip the prefix lanes
// before the requested destination byte. The byte strings here are synthetic.
#[test]
fn derived_unaligned_store_skips_funnel_prefix_lanes() {
    for delta in 1..=3 {
        let mut sx = fixture();
        let wanted = expected(&sx, delta);
        let mut stream = vec![0xe1; delta];
        stream.extend(BYTES);
        stream.resize(20, 0x7e);
        for (i, bytes) in stream.chunks_exact(4).enumerate() {
            reg(
                &mut sx,
                10 + i,
                u32::from_be_bytes(bytes.try_into().unwrap()),
            );
        }
        // STPD|plane-mask, 14 bytes from R10. 0x868fc505 for delta=1.
        sx.write(COMMAND, 0x868f_c500 | (4 + delta as u32), u32::MAX)
            .unwrap();
        check(&sx, &wanted, delta);
    }
}

#[test]
fn derived_overlapping_load_funnel_store_moves_b32_pixels_right() {
    for delta in 1..=3 {
        let mut sx = fixture();
        let wanted = expected(&sx, delta);
        reg(&mut sx, 10, 0xe1e2_e3e4);
        // Captured load PA/word and captured funnel when delta=1.
        sx.write(COMMAND, 0x86d7_c584, u32::MAX).unwrap();
        let shift = (4 - delta as u32) * 8;
        sx.write(SX_BASE + 0x1020, 0xc8e2_8500 | shift, u32::MAX)
            .expect("captured copy alignment funnel");
        sx.write(COMMAND, 0x868f_c500 | (4 + delta as u32), u32::MAX)
            .unwrap();
        check(&sx, &wanted, delta);
    }
}
