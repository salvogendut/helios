use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn helios(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_helios"))
        .args(args)
        .output()
        .unwrap()
}

struct Image(PathBuf);

impl Image {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "helios-cli-{}-{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, bytes).unwrap();
        Self(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn demo_exits_successfully_with_43_trace_records() {
    let output = helios(&["demo", "--trace"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("pc="))
            .count(),
        43
    );
    assert!(stdout.contains("Demo passed: sum(1..=10) = 55"));
}

#[test]
fn raw_runner_uses_load_address_as_entry_and_stops_at_budget() {
    let image = Image::new(&[1, 0, 0, 0, 1, 0, 0, 0]); // two NOPs
    let output = helios(&[
        "run",
        image.path(),
        "--load-address",
        "0x100",
        "--steps",
        "2",
        "--ram-mib",
        "1",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PC=0x00000108"));
}

#[test]
fn faults_and_bad_cli_input_are_unsuccessful() {
    let image = Image::new(&[0, 0, 0, 0]);
    let output = helios(&["run", image.path(), "--steps", "1"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CPU error state: IllegalInstruction"));
    assert!(stderr.contains("PC=0x00000000, nPC=0x00000004"));
    for args in [
        vec!["run", image.path(), "--steps", "0"],
        vec!["run", image.path(), "--entry", "1"],
        vec!["run", image.path(), "--ram-mib", "257"],
        vec!["run", image.path(), "--entry", "0x100000000"],
        vec!["run", image.path(), "--steps"],
        vec!["run", image.path(), "--unknown"],
        vec!["demo", "--unknown"],
    ] {
        assert!(!helios(&args).status.success(), "{args:?}");
    }
}

#[test]
fn writable_disk_option_is_explicit_exclusive_and_never_creates_or_truncates() {
    let prom = Image::new(&[1, 0, 0, 0]);
    let disk = Image::new(&[0x5a; 512]);
    for option in ["--disk", "--disk-rw"] {
        let output = helios(&[
            "boot",
            prom.path(),
            "--steps",
            "1",
            "--ram-mib",
            "8",
            option,
            disk.path(),
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(&disk.0).unwrap(), [0x5a; 512]);
    }
    for options in [
        ["--disk", "--disk-rw"],
        ["--disk-rw", "--disk"],
        ["--disk-rw", "--disk-rw"],
    ] {
        let output = helios(&[
            "boot",
            prom.path(),
            options[0],
            disk.path(),
            options[1],
            disk.path(),
        ]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Choose one --disk or --disk-rw"));
    }
    let missing = disk.0.with_extension("absent");
    assert!(!missing.exists());
    let output = helios(&[
        "boot",
        prom.path(),
        "--disk-rw",
        missing.to_str().unwrap(),
        "--steps",
        "1",
        "--ram-mib",
        "8",
    ]);
    assert!(!output.status.success());
    assert!(!missing.exists());
    assert_eq!(std::fs::read(disk.0.as_path()).unwrap(), [0x5a; 512]);
}

#[test]
fn trap_demo_reports_trap_entry_separately_from_executed_instructions() {
    let output = helios(&["trap-demo", "--trace"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("pc="))
            .count(),
        13
    );
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("trap="))
            .count(),
        1
    );
    assert!(stdout.contains("trap=87 vector=00001870"));
    assert!(stdout.contains("Trap demo passed"));
}

#[test]
fn guest_software_reset_is_traced_and_execution_continues_within_the_budget() {
    let bytes: Vec<u8> = [
        0x031c_7c00u32, // sethi %hi(0x71f00000), g1
        0x8410_2001,    // mov 1, g2
        0xc4a0_4400,    // sta g2, [g1] ASI 0x20
    ]
    .into_iter()
    .flat_map(u32::to_be_bytes)
    .collect();
    let image = Image::new(&bytes);
    let output = helios(&[
        "boot",
        image.path(),
        "--steps",
        "9",
        "--trace",
        "--ram-mib",
        "16",
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let trace = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        trace
            .lines()
            .filter(|line| line.starts_with("reset=software"))
            .count(),
        3
    );
    assert!(trace.contains("9 instructions, 0 traps, 0 interrupts, 3 resets"));
    assert!(trace.contains("PC=0x00000000, nPC=0x00000004"));
}

#[test]
fn out_of_ram_and_empty_images_are_rejected() {
    let image = Image::new(&[1, 0, 0, 0]);
    let output = helios(&[
        "run",
        image.path(),
        "--load-address",
        "0x100000",
        "--ram-mib",
        "1",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unmapped"));
    let empty = Image::new(&[]);
    let output = helios(&["run", empty.path()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("raw image is empty"));
}

#[test]
fn boot_runner_uses_reset_prom_fetch_and_an_explicit_step_budget() {
    let image = Image::new(&[1, 0, 0, 0, 1, 0, 0, 0]);
    let output = helios(&[
        "boot",
        image.path(),
        "--steps",
        "2",
        "--ram-mib",
        "16",
        "--trace",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pc=00000000 npc=00000004 instruction=01000000"));
    assert!(stderr.contains("PC=0x00000008"));
    for args in [
        vec!["boot", image.path(), "--entry", "0"],
        vec!["boot", image.path(), "--load-address", "0"],
        vec!["boot", image.path(), "--steps", "0"],
        vec!["boot", image.path(), "--ram-mib", "1"],
        vec!["boot", image.path(), "--ram-mib", "240"],
        vec!["boot", image.path(), "--serial-in"],
        vec!["boot", image.path(), "--nvram"],
        vec!["boot", image.path(), "--nvram-out"],
        vec!["run", image.path(), "--serial-in", image.path()],
    ] {
        assert!(!helios(&args).status.success(), "{args:?}");
    }
}

#[test]
fn serial_demo_keeps_guest_bytes_separate_from_diagnostics_and_trace() {
    let output = helios(&["serial-demo", "--trace"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"HELIOS!\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("pc="))
            .count(),
        256
    );
    assert!(stderr.contains("0 traps"));
    assert!(stderr.contains("Serial demo passed"));
}

#[test]
fn timer_demo_reports_interrupts_separately_from_instructions_and_sync_traps() {
    let output = helios(&["timer-demo", "--trace"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("pc="))
            .count(),
        254
    );
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("interrupt="))
            .count(),
        2
    );
    assert!(stdout.contains("interrupt=10 vector=000011a0"));
    assert!(stdout.contains("interrupt=14 vector=000011e0"));
    assert!(stdout.contains("254 instructions, 0 traps, 2 interrupts"));
    assert!(stdout.contains("Timer demo passed"));
    assert!(!helios(&["timer-demo", "--unknown"]).status.success());
}

#[test]
fn boot_feeds_serial_input_after_guest_initialization_and_saves_nvram() {
    let prom = Image::new(&helios::machine::serial_demo_image());
    let input = Image::new(b"?");
    let mut bytes = helios::devices::nvram::Nvram::default().image();
    bytes[0x123] = 0x5a;
    let nvram = Image::new(&bytes);
    let saved = Image::new(&[]);
    std::fs::remove_file(&saved.0).unwrap();
    let output = helios(&[
        "boot",
        prom.path(),
        "--steps",
        "256",
        "--ram-mib",
        "16",
        "--serial-in",
        input.path(),
        "--nvram",
        nvram.path(),
        "--nvram-out",
        saved.path(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"HELIOS?\n");
    assert_eq!(std::fs::read(&saved.0).unwrap(), bytes);
    assert_eq!(std::fs::read(&nvram.0).unwrap(), bytes);
    assert_eq!(std::fs::read(&input.0).unwrap(), b"?");

    let output = helios(&["boot", prom.path(), "--nvram-out", nvram.path()]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(std::fs::read(&nvram.0).unwrap(), bytes);
}

#[test]
fn boot_saves_guest_nvram_changes_on_error_and_rejects_invalid_input_before_creating_output() {
    // sethi %hi(0x71200000), g1; mov 0x5a, g2; stb g2, [g1+0x123]; unimp
    let bytes: Vec<_> = [0x031c_4800u32, 0x8410_205a, 0xc428_6123, 0]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    let prom = Image::new(&bytes);
    let saved = Image::new(&[]);
    std::fs::remove_file(&saved.0).unwrap();
    let output = helios(&[
        "boot",
        prom.path(),
        "--steps",
        "4",
        "--nvram-out",
        saved.path(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("CPU error state"));
    let mut expected = helios::devices::nvram::Nvram::default().image();
    expected[0x123] = 0x5a;
    assert_eq!(std::fs::read(&saved.0).unwrap(), expected);

    std::fs::remove_file(&saved.0).unwrap();
    let invalid = Image::new(&[0; 16]);
    let output = helios(&[
        "boot",
        prom.path(),
        "--nvram",
        invalid.path(),
        "--nvram-out",
        saved.path(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exactly 8192 bytes"));
    assert!(!saved.0.exists());
}

#[test]
fn boot_cpu_count_is_validated_and_reports_both_processors() {
    let prom = Image::new(&[1, 0, 0, 0]);
    let dual = helios(&[
        "boot",
        prom.path(),
        "--machine",
        "ss20",
        "--cpus",
        "2",
        "--steps",
        "1",
        "--ram-mib",
        "16",
    ]);
    assert!(
        dual.status.success(),
        "{}",
        String::from_utf8_lossy(&dual.stderr)
    );
    assert!(String::from_utf8_lossy(&dual.stderr).contains("CPU2 counters"));
    let single = helios(&[
        "boot",
        prom.path(),
        "--machine",
        "ss20",
        "--steps",
        "1",
        "--ram-mib",
        "16",
    ]);
    assert!(single.status.success());
    assert!(!String::from_utf8_lossy(&single.stderr).contains("CPU2 counters"));
    for args in [
        vec![
            "boot",
            prom.path(),
            "--machine",
            "ss5",
            "--cpus",
            "2",
            "--steps",
            "1",
            "--ram-mib",
            "16",
        ],
        vec![
            "boot",
            prom.path(),
            "--machine",
            "ss20",
            "--cpus",
            "3",
            "--steps",
            "1",
            "--ram-mib",
            "16",
        ],
        vec![
            "run",
            prom.path(),
            "--cpus",
            "2",
            "--steps",
            "1",
            "--ram-mib",
            "16",
        ],
    ] {
        assert!(!helios(&args).status.success(), "{args:?}");
    }
}

fn review_imm(rd: u32, op3: u32, rs1: u32, value: u32) -> u32 {
    2 << 30 | rd << 25 | op3 << 19 | rs1 << 14 | 1 << 13 | value & 0x1fff
}

fn review_set32(rd: u32, value: u32) -> Vec<u32> {
    vec![
        rd << 25 | 4 << 22 | value >> 10,
        review_imm(rd, 2, rd, value & 0x3ff),
    ]
}

fn review_alt(rd: u32, op3: u32, rs1: u32, asi: u32) -> u32 {
    3 << 30 | rd << 25 | op3 << 19 | rs1 << 14 | asi << 5
}

fn review_poweroff() -> Vec<u32> {
    // AUXIO2 power-off byte store at PA 0xff1a01000 via ASI 0x2f.
    let mut words = review_set32(1, 0xf1a01000);
    words.push(review_imm(2, 2, 0, 1));
    words.push(review_alt(2, 0x15, 1, 0x2f));
    words
}

fn review_secondary(action: &[u32]) -> (Vec<u32>, usize) {
    let mut words = review_set32(1, 0x01c00f04);
    words.push(review_alt(3, 0x10, 1, 2));
    words.push(review_imm(3, 0x26, 3, 24));
    words.push(review_imm(0, 0x14, 3, 10));
    let branch = words.len();
    words.extend([0, 0x0100_0000]);
    words.extend(review_set32(1, 0xe0001008));
    words.extend([review_imm(2, 2, 0, 4), review_alt(2, 0x14, 1, 0x2f)]);
    words.extend([0x1080_0000, 0x0100_0000]);
    words[branch] = 1 << 25 | 2 << 22 | (words.len() - branch) as u32;
    let entry = words.len() * 4;
    words.extend(action);
    (words, entry)
}

fn review_run(model: &str, cpus: usize, words: &[u32], trace: bool) -> Output {
    let image = Image::new(
        &words
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .collect::<Vec<_>>(),
    );
    let cpu_text = cpus.to_string();
    let mut args = vec![
        "boot",
        image.path(),
        "--machine",
        model,
        "--cpus",
        &cpu_text,
        "--steps",
        "100",
        "--ram-mib",
        "16",
    ];
    if trace {
        args.push("--trace");
    }
    helios(&args)
}

#[test]
fn guest_power_off_is_successful_through_the_scheduled_cli_path() {
    for model in ["ss20", "axil320"] {
        let up = review_run(model, 1, &review_poweroff(), false);
        assert!(
            up.status.success(),
            "{model} UP: {}",
            String::from_utf8_lossy(&up.stderr)
        );
        assert!(String::from_utf8_lossy(&up.stderr).contains("System powered off."));
        let cpu0 = review_run(model, 2, &review_poweroff(), false);
        assert!(
            cpu0.status.success(),
            "{model} CPU0: {}",
            String::from_utf8_lossy(&cpu0.stderr)
        );
        assert!(String::from_utf8_lossy(&cpu0.stderr).contains("System powered off."));
        let (program, _) = review_secondary(&review_poweroff());
        let cpu2 = review_run(model, 2, &program, false);
        assert!(
            cpu2.status.success(),
            "{model} CPU2: {}",
            String::from_utf8_lossy(&cpu2.stderr)
        );
        assert!(String::from_utf8_lossy(&cpu2.stderr).contains("System powered off."));
    }
    let prom = Image::new(
        &review_poweroff()
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .collect::<Vec<_>>(),
    );
    let nvram = std::env::temp_dir().join(format!("helios-poweroff-{}.nvram", std::process::id()));
    let _ = std::fs::remove_file(&nvram);
    let output = helios(&[
        "boot",
        prom.path(),
        "--machine",
        "ss20",
        "--cpus",
        "2",
        "--steps",
        "100",
        "--ram-mib",
        "16",
        "--nvram-out",
        nvram.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&nvram).unwrap().len(), 8192);
    let _ = std::fs::remove_file(nvram);
}

#[test]
fn scheduled_guest_aer_counts_match_trace_and_secondary_error_is_tagged() {
    let (program, _) = review_secondary(&[0x1080_0000, 0x0100_0000]);
    for model in ["ss20", "axil320"] {
        let output = review_run(model, 2, &program, true);
        assert!(
            output.status.success(),
            "{model}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut total = 0;
        for processor in [0, 2] {
            let retired = stderr
                .lines()
                .filter(|line| {
                    line.starts_with("pc=") && line.ends_with(&format!("cpu={processor}"))
                })
                .count();
            assert!(retired > 0, "{model} CPU{processor}: {stderr}");
            assert!(
                stderr.contains(&format!("CPU{processor} counters=({retired}, 0, 0, 0)")),
                "{model} CPU{processor}: {stderr}"
            );
            total += retired;
        }
        assert_eq!(total, 100, "both CPUs must share the CLI step budget");

        let (error_program, pc) = review_secondary(&[0]);
        let error = review_run(model, 2, &error_program, false);
        assert!(!error.status.success(), "{model}");
        let stderr = String::from_utf8_lossy(&error.stderr);
        assert!(stderr.contains("processor 2"), "{stderr}");
        assert!(stderr.contains("IllegalInstruction"), "{stderr}");
        assert!(stderr.contains(&format!("PC=0x{pc:08x}")), "{stderr}");
    }
}

#[test]
fn boot_cpu_count_rejects_missing_zero_four_and_malformed_values_in_any_order() {
    let prom = Image::new(&[1, 0, 0, 0]);
    for args in [
        vec!["boot", prom.path(), "--cpus"],
        vec!["boot", prom.path(), "--cpus", "0", "--steps", "1"],
        vec!["boot", prom.path(), "--cpus", "3", "--steps", "1"],
        vec!["boot", prom.path(), "--cpus", "4", "--steps", "1"],
        vec!["boot", prom.path(), "--cpus", "two", "--steps", "1"],
        vec!["boot", prom.path(), "--cpus", "2", "--machine", "ss5"],
    ] {
        let output = helios(&args);
        assert!(!output.status.success(), "{args:?}");
    }
    let output = helios(&[
        "boot",
        prom.path(),
        "--cpus",
        "2",
        "--machine",
        "axil320",
        "--steps",
        "1",
        "--ram-mib",
        "16",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
