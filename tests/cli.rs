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
