//! Dependency-free fixed-work performance harness. Guest boot workloads are
//! supplied separately by the local profiling harness.
use helios::{
    bus::Bus,
    cpu::Step,
    machine::{Machine, SchedulerError},
};
use std::time::Instant;

#[cfg(target_os = "linux")]
fn host_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let close = stat.rfind(')')?;
    let fields: Vec<_> = stat[close + 2..].split_whitespace().collect();
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}

#[cfg(not(target_os = "linux"))]
fn host_ticks() -> Option<u64> {
    None
}

fn host_clock_hz() -> Option<u64> {
    let text = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())?;
    text.trim().parse::<u64>().ok().filter(|&hz| hz > 0)
}

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

fn fixed(mode: &str, iterations: u32, decode_cache: bool, jit: bool) {
    assert!(iterations != 0, "iterations must be nonzero");
    let workload = workload(mode, iterations);
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.cpu.set_decode_cache_enabled(decode_cache);
    machine.load_image(0, &workload.code).unwrap();
    for _ in 0..workload.init_steps {
        assert!(matches!(machine.step().unwrap(), Step::Executed { .. }));
    }
    let cpu_start = host_ticks();
    let start = Instant::now();
    let mut traps = 0u64;
    let mut interrupts = 0u64;
    #[cfg(feature = "jit")]
    let mut jit_stats = None;
    if jit {
        #[cfg(feature = "jit")]
        {
            let stats = machine.run_jit(workload.loop_steps).unwrap();
            assert_eq!(stats.executed, workload.loop_steps, "{stats:?}");
            assert_eq!(stats.stopped, None, "{stats:?}");
            jit_stats = Some(stats);
        }
        #[cfg(not(feature = "jit"))]
        panic!("build with --features jit");
    } else {
        for _ in 0..workload.loop_steps {
            match machine.step().unwrap() {
                Step::Executed { .. } => {}
                Step::TrapTaken { .. } => traps += 1,
                Step::InterruptTaken { .. } => interrupts += 1,
                Step::SoftwareReset { .. } => panic!("unexpected reset"),
            }
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
    let host_cpu_s = host_clock_hz().and_then(|hz| {
        cpu_start
            .zip(host_ticks())
            .map(|(start, end)| (end.saturating_sub(start)) as f64 / hz as f64)
    });
    let cpu_util = host_cpu_s
        .and_then(|cpu| (elapsed.as_secs_f64() > 0.0).then_some(cpu / elapsed.as_secs_f64()));
    let (cpu_text, util_text) = match (host_cpu_s, cpu_util) {
        (Some(cpu), Some(util)) => (format!("{cpu:.6}"), format!("{util:.3}")),
        _ => ("unavailable".into(), "unavailable".into()),
    };
    #[cfg(feature = "jit")]
    let mut jit_text = String::new();
    #[cfg(not(feature = "jit"))]
    let jit_text = "";
    #[cfg(feature = "jit")]
    if let Some(stats) = jit_stats {
        jit_text = format!(
            " jit_blocks={} jit_instructions={} jit_calls={} jit_fallback={} jit_compile_ns={} jit_steady_ns_per_step={:.2}",
            stats.compiled_blocks,
            stats.compiled_instructions,
            stats.compile_calls,
            stats.fallback_steps,
            stats.compile_ns,
            (elapsed.as_nanos() as f64 - stats.compile_ns as f64).max(0.0)
                / workload.loop_steps as f64
        );
    }
    println!(
        "workload=fixed/{} iterations={iterations} steps={} executed={} traps={} interrupts={} decode_cache={} jit={}{} wall_s={:.6} host_cpu_s={} cpu_util={} ns_per_step={:.2} checksum=ok",
        workload.name,
        workload.loop_steps,
        workload.loop_steps,
        traps,
        interrupts,
        if decode_cache { "on" } else { "off" },
        if jit { "on" } else { "off" },
        jit_text,
        elapsed.as_secs_f64(),
        cpu_text,
        util_text,
        elapsed.as_nanos() as f64 / workload.loop_steps as f64
    );
}

#[derive(Debug, PartialEq, Eq)]
struct CompareState {
    steps: u64,
    executed: u64,
    traps: u64,
    interrupts: u64,
    resets: u64,
    pc: u32,
    npc: u32,
    psr: u32,
    registers: Vec<u32>,
    memory: Option<u32>,
}

fn compare_step(machine: &mut Machine, scheduled: bool) -> Step {
    if scheduled {
        match machine.step_scheduled() {
            Ok(outcome) => outcome.step,
            Err(SchedulerError::Cpu { fault, .. }) => panic!("scheduled CPU fault: {fault}"),
            Err(error) => panic!("scheduled step failed: {error}"),
        }
    } else {
        machine.step().unwrap()
    }
}

fn compare_run(mode: &str, iterations: u32, scheduled: bool) -> (f64, CompareState) {
    let workload = workload(mode, iterations);
    let mut machine = Machine::bare_metal(0x10000, 0).unwrap();
    machine.load_image(0, &workload.code).unwrap();
    for _ in 0..workload.init_steps {
        assert!(matches!(
            compare_step(&mut machine, scheduled),
            Step::Executed { .. }
        ));
    }
    let start = Instant::now();
    let mut state = CompareState {
        steps: 0,
        executed: 0,
        traps: 0,
        interrupts: 0,
        resets: 0,
        pc: 0,
        npc: 0,
        psr: 0,
        registers: Vec::new(),
        memory: None,
    };
    for _ in 0..workload.loop_steps {
        state.steps += 1;
        match compare_step(&mut machine, scheduled) {
            Step::Executed { .. } => state.executed += 1,
            Step::TrapTaken { .. } => state.traps += 1,
            Step::InterruptTaken { .. } => state.interrupts += 1,
            Step::SoftwareReset { .. } => state.resets += 1,
        }
    }
    let wall_s = start.elapsed().as_secs_f64();
    assert_eq!(state.traps, 0, "compare workload took a trap");
    assert_eq!(state.interrupts, 0, "compare workload took an interrupt");
    assert_eq!(state.resets, 0, "compare workload reset");
    for &(register, expected) in &workload.expected {
        assert_eq!(
            machine.cpu.read_reg(register as u8),
            expected,
            "{mode}: compare final %g{register}"
        );
    }
    state.pc = machine.cpu.pc();
    state.npc = machine.cpu.npc();
    state.psr = machine.cpu.psr();
    state.registers = (0..32).map(|i| machine.cpu.read_reg(i)).collect();
    state.memory = workload
        .expected_memory
        .map(|(address, _)| machine.bus.read32(address).unwrap());
    if let (Some((_, expected)), Some(actual)) = (workload.expected_memory, state.memory) {
        assert_eq!(actual, expected, "{mode}: compare final memory checksum");
    }
    (wall_s, state)
}

fn compare(mode: &str, iterations: u32) {
    assert!(iterations != 0, "iterations must be nonzero");
    let (step_wall, step_state) = compare_run(mode, iterations, false);
    let (scheduled_wall, scheduled_state) = compare_run(mode, iterations, true);
    assert_eq!(step_state, scheduled_state, "{mode}: entry paths diverged");
    println!(
        "compare/{mode} iterations={iterations} steps={} executed={} traps={} interrupts={} resets={} state=match step_ns_per_step={:.2} scheduled_ns_per_step={:.2}",
        step_state.steps,
        step_state.executed,
        step_state.traps,
        step_state.interrupts,
        step_state.resets,
        step_wall * 1e9 / step_state.steps as f64,
        scheduled_wall * 1e9 / scheduled_state.steps as f64,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("compare") => {
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
            compare(mode, iterations);
        }
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
            let mode_flag = args.get(4).map(String::as_str);
            let decode_cache = mode_flag == Some("cache");
            let jit = mode_flag == Some("jit");
            fixed(mode, iterations, decode_cache, jit);
        }
        _ => {
            eprintln!(
                "usage: perf fixed <alu|memory> [iterations] [cache|jit]\n\
                 perf compare <alu|memory> [iterations]\n\
                 Guest boot/scanout workloads use the supplied ROM, NVRAM and disk media."
            );
            std::process::exit(2);
        }
    }
}
