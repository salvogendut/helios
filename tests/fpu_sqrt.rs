use helios::bus::Bus;
use helios::cpu::{Step, PSR_EF, PSR_ET, PSR_S};
use helios::fpu::FpuState;
use helios::machine::Machine;

const NOP: u32 = 0x0100_0000;
const START: u32 = 32;
const FSR_BASE: u32 = 0x0008_0000;
const SENTINEL: u64 = 0xdec0_adde_a5a5_5a5a;

fn sqrt_op(double: bool, rd: u32, rs2: u32) -> u32 {
    0x81a0_0000 | (rd << 25) | (if double { 0x2a } else { 0x29 } << 5) | rs2
}

fn put(m: &mut Machine, address: u32, words: &[u32]) {
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    m.load_image(address, &bytes).unwrap();
}

fn setup(program: &[u32], fsr: u32) -> Machine {
    let mut m = Machine::bare_metal(4096, 0).unwrap();
    put(
        &mut m,
        0,
        &[0x8188_6000, NOP, NOP, NOP, 0xc108_a000, NOP, NOP, NOP],
    );
    put(&mut m, START, program);
    m.cpu.write_reg(1, PSR_S | PSR_EF | PSR_ET);
    m.cpu.write_reg(2, 0x200);
    m.bus.write32(0x200, fsr).unwrap();
    for _ in 0..8 {
        m.step().unwrap();
    }
    m
}

fn write(m: &mut Machine, double: bool, index: u8, bits: u64) {
    if double {
        m.cpu.fpu.write_reg(index, (bits >> 32) as u32);
        m.cpu.fpu.write_reg(index + 1, bits as u32);
    } else {
        m.cpu.fpu.write_reg(index, bits as u32);
    }
}

fn read(m: &Machine, double: bool, index: u8) -> u64 {
    let high = u64::from(m.cpu.fpu.read_reg(index));
    if double {
        (high << 32) | u64::from(m.cpu.fpu.read_reg(index + 1))
    } else {
        high
    }
}

fn check(double: bool, source: u64, expected: u64, flags: u32, rounding: u32) {
    let mut m = setup(&[sqrt_op(double, 8, 0)], rounding << 30);
    write(&mut m, double, 0, source);
    write(&mut m, true, 8, SENTINEL);
    assert!(matches!(
        m.step().unwrap(),
        Step::Executed { pc: START, .. }
    ));
    assert_eq!(read(&m, double, 8), expected, "source={source:x}");
    assert_eq!(
        m.cpu.fpu.fsr(),
        FSR_BASE | (rounding << 30) | flags | (flags << 5)
    );
    assert_eq!(m.cpu.fpu.state(), FpuState::Execute);
    assert_eq!(m.cpu.fpu.deferred_entry(), None);
}

#[test]
fn square_root_handles_special_values_and_exact_results() {
    for (double, source, expected) in [
        (true, 0x4010_0000_0000_0000, 0x4000_0000_0000_0000), // sqrt(4) = 2
        (true, 0x3fd0_0000_0000_0000, 0x3fe0_0000_0000_0000), // sqrt(0.25) = 0.5
        (false, 0x4080_0000, 0x4000_0000),                    // sqrt(4) = 2
        (false, 0x3e80_0000, 0x3f00_0000),                    // sqrt(0.25) = 0.5
        (true, 0, 0),                                         // +0
        (true, 0x8000_0000_0000_0000, 0x8000_0000_0000_0000), // -0
        (false, 0x7f80_0000, 0x7f80_0000),                    // +inf
        (true, 0x7ff0_0000_0000_0000, 0x7ff0_0000_0000_0000), // +inf
        (true, 0x0000_0000_0000_0001, 0x1e60_0000_0000_0000), // sqrt(2^-1074)
    ] {
        check(double, source, expected, 0, 0);
    }
}

#[test]
fn square_root_rounds_inexact_values_and_softwares_negative_inputs() {
    check(true, 0x4000_0000_0000_0000, 0x3ff6_a09e_667f_3bcd, 1, 0); // sqrt(2)
    check(true, 0x4008_0000_0000_0000, 0x3ffb_b67a_e858_4caa, 1, 0); // sqrt(3)
    check(false, 0x4000_0000, 0x3fb5_04f3, 1, 0); // sqrt(2)
    check(true, 0x3fb9_9999_9999_999a, 0x3fd4_3d13_6248_490f, 1, 0); // sqrt(0.1)
                                                                     // Negative finite values produce the positive canonical quiet NaN and NV.
    check(true, 0xbff0_0000_0000_0000, 0x7fff_ffff_ffff_ffff, 16, 0);
    check(false, 0xbf80_0000, 0x7fff_ffff, 16, 0);
}

#[test]
fn square_root_directed_rounding_uses_the_destination_grid() {
    // sqrt(2): nearest is ...3bcd, toward zero ...3bcc, toward +inf ...3bcd.
    check(true, 0x4000_0000_0000_0000, 0x3ff6_a09e_667f_3bcc, 1, 1);
    check(true, 0x4000_0000_0000_0000, 0x3ff6_a09e_667f_3bcd, 1, 2);
    // sqrt(0.3): nearest is ...378c, toward zero ...378b, toward +inf ...378c.
    check(false, 0x3e99_999a, 0x3f0c_378b, 1, 1);
    check(false, 0x3e99_999a, 0x3f0c_378c, 1, 2);
}
