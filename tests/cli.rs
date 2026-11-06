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
