//! Dependency-free fixed-work performance harness. Guest boot workloads are
//! supplied separately by the local profiling harness.
use helios::{bus::Bus, cpu::Step, machine::Machine};
use std::time::Instant;

fn sethi(rd: u32, value: u32) -> u32 {
    0x0100_0000 | (rd << 25) | ((value >> 10) & 0x3f_ffff)
}
fn or_imm(rd: u32, rs1: u32, value: u32) -> u32 {
    0x8000_0000 | (rd << 25) | (2 << 19) | (rs1 << 14) | 0x2000 | (value & 0x1fff)
}
fn image(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

struct Workload {
    code: Vec<u8>,
    init_steps: u64,
    loop_steps: u64,
    expected: Vec<(usize, u32)>,
    expected_memory: Option<(u64, u32)>,
    name: &'static str,
}

fn workload(mode: &str, iterations: u32) -> Workload {
    match mode {
        "alu" => {
            let mut checksum = 0u32;
            for value in 1..=iterations {
                checksum ^= value;
            }
            Workload {
                code: image(&[
                    or_imm(2, 0, 0),
                    or_imm(5, 0, 0),
                    sethi(4, iterations),
                    or_imm(4, 4, iterations & 0x3ff),
                    0x8400_a001, // add %g2, 1, %g2
                    0x8a19_4002, // xor %g5, %g2, %g5
                    0x88a1_2001, // subcc %g4, 1, %g4
                    0x12bf_fffd, // bne loop
                    0x0100_0000, // nop
                    0x3080_0000, // ba,a .
                    0x0100_0000,
                ]),
                init_steps: 4,
                loop_steps: u64::from(iterations) * 5,
                expected: vec![(2, iterations), (4, 0), (5, checksum)],
                expected_memory: None,
                name: "alu",
            }
        }
        "memory" => Workload {
            code: image(&[
                or_imm(1, 0, 0x800),
                or_imm(2, 0, 0),
                or_imm(3, 0, 0),
                sethi(4, iterations),
                or_imm(4, 4, iterations & 0x3ff),
                0xc400_6000, // ld [%g1], %g2
                0x8400_a001, // add %g2, 1, %g2
                0xc420_6000, // st %g2, [%g1]
                0x8600_e001, // add %g3, 1, %g3
                0x88a1_2001, // subcc %g4, 1, %g4
                0x12bf_fffb, // bne loop
                0x0100_0000, // nop
                0x3080_0000, // ba,a .
                0x0100_0000,
            ]),
            init_steps: 5,
            loop_steps: u64::from(iterations) * 7,
            expected: vec![(2, iterations), (3, iterations), (4, 0)],
            expected_memory: Some((0x800u64, iterations)),
            name: "memory",
        },
        _ => panic!("fixed mode must be alu or memory"),
    }
}

fn fixed(mode: &str, iterations: u32) {
    assert!(iterations != 0, "iterations must be nonzero");
    let workload = workload(mode, iterations);
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(0, &workload.code).unwrap();
    for _ in 0..workload.init_steps {
        assert!(matches!(machine.step().unwrap(), Step::Executed { .. }));
    }
    let start = Instant::now();
    let mut traps = 0u64;
    let mut interrupts = 0u64;
    for _ in 0..workload.loop_steps {
        match machine.step().unwrap() {
            Step::Executed { .. } => {}
            Step::TrapTaken { .. } => traps += 1,
            Step::InterruptTaken { .. } => interrupts += 1,
            Step::SoftwareReset { .. } => panic!("unexpected reset"),
        }
    }
    let elapsed = start.elapsed();
    assert_eq!(traps, 0, "fixed workload took a trap");
    assert_eq!(interrupts, 0, "fixed workload took an interrupt");
    for &(register, expected) in &workload.expected {
        assert_eq!(
            machine.cpu.read_reg(register as u8),
            expected,
            "{mode}: final %g{register}"
        );
    }
    if let Some((address, expected)) = workload.expected_memory {
        assert_eq!(
            machine.bus.read32(address).unwrap(),
            expected,
            "{mode}: final memory checksum"
        );
    }
    println!(
        "workload=fixed/{} iterations={iterations} steps={} executed={} traps={} interrupts={} wall_s={:.6} ns_per_step={:.2} checksum=ok",
        workload.name,
        workload.loop_steps,
        workload.loop_steps,
        traps,
        interrupts,
        elapsed.as_secs_f64(),
        elapsed.as_nanos() as f64 / workload.loop_steps as f64
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("fixed") => {
            let mode = args.get(2).map(String::as_str).unwrap_or("alu");
            let iterations = match args.get(3) {
                Some(value) => value.parse().unwrap_or_else(|_| {
                    eprintln!("iterations must be a positive integer");
                    std::process::exit(2);
                }),
                None => 1_000_000,
            };
            if iterations == 0 {
                eprintln!("iterations must be nonzero");
                std::process::exit(2);
            }
            fixed(mode, iterations);
        }
        _ => {
            eprintln!(
                "usage: perf fixed <alu|memory> [iterations]\n\
                 Guest boot/scanout workloads use the supplied ROM, NVRAM and disk media."
            );
            std::process::exit(2);
        }
    }
}
