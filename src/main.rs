use helios::bus::Bus;
use helios::machine::{demo_image, Machine, DEFAULT_RAM_BYTES, TARGET};
use std::error::Error;
use std::process::ExitCode;

const HELP: &str = "Helios — Sun SPARC emulator foundation

Usage:
  helios info
  helios demo [--trace]
  helios run FILE [--load-address N] [--entry N] [--steps N] [--ram-mib N] [--trace]

FILE is a raw big-endian SPARC instruction/data image, loaded into RAM.
Addresses accept decimal or 0x-prefixed hexadecimal. Entry defaults to the load
address (default 0). Defaults: 1000 instructions, 64 MiB RAM. --trace prints each
completed instruction's PC, nPC and word. Faults exit with status 1.
Reaching the instruction budget exits successfully; it does not mean guest halt.

Current mode is a bare-metal interpreter subset. Firmware/OS boot is not supported.";

fn number(value: &str) -> Result<u64, Box<dyn Error>> {
    Ok(if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)?
    } else {
        value.parse()?
    })
}

fn execute(machine: &mut Machine, steps: u64, trace: bool) -> Result<(), Box<dyn Error>> {
    for _ in 0..steps {
        let step = machine.step()?;
        if trace {
            println!(
                "pc={:08x} npc={:08x} instruction={:08x}",
                step.pc, step.npc, step.instruction
            );
        }
    }
    println!(
        "Instruction budget reached ({steps}); PC={:#010x}, nPC={:#010x}",
        machine.cpu.pc(),
        machine.cpu.npc()
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help" | "-h") => println!("{HELP}"),
        Some("info") => {
            if args.next().is_some() {
                return Err("info takes no options".into());
            }
            println!("Helios {}\nTarget: {TARGET}\nStatus: bare-metal integer subset; no firmware or OS boot", env!("CARGO_PKG_VERSION"));
        }
        Some("demo") => {
            let remaining: Vec<_> = args.collect();
            let trace = match remaining.as_slice() {
                [] => false,
                [option] if option == "--trace" => true,
                _ => return Err("usage: helios demo [--trace]".into()),
            };
            let mut machine = Machine::bare_metal(4096, 0)?;
            machine.load_image(0, &demo_image())?;
            execute(&mut machine, 43, trace)?;
            let result = machine.bus.read32(0x100)?;
            if result != 55 {
                return Err(format!("demo failed: expected 55, got {result}").into());
            }
            println!("Demo passed: sum(1..=10) = {result}, stored at RAM[0x100]");
        }
        Some("run") => {
            let file = args.next().ok_or("run requires a raw image filename")?;
            let mut load = 0u32;
            let mut entry = None;
            let mut steps = 1000;
            let mut ram_bytes = DEFAULT_RAM_BYTES;
            let mut trace = false;
            while let Some(option) = args.next() {
                if option == "--trace" {
                    trace = true;
                    continue;
                }
                if !matches!(
                    option.as_str(),
                    "--load-address" | "--entry" | "--steps" | "--ram-mib"
                ) {
                    return Err(format!("unknown option: {option}").into());
                }
                let value = number(
                    &args
                        .next()
                        .ok_or_else(|| format!("{option} requires a value"))?,
                )?;
                match option.as_str() {
                    "--load-address" => load = value.try_into()?,
                    "--entry" => entry = Some(value.try_into()?),
                    "--steps" => steps = value,
                    "--ram-mib" => {
                        ram_bytes = usize::try_from(value)?
                            .checked_mul(1024 * 1024)
                            .ok_or("RAM size overflow")?
                    }
                    _ => unreachable!(),
                }
            }
            if steps == 0 {
                return Err("--steps must be greater than zero".into());
            }
            let entry = entry.unwrap_or(load);
            if entry % 4 != 0 {
                return Err("entry address must be word aligned".into());
            }
            let bytes = std::fs::read(file)?;
            if bytes.is_empty() {
                return Err("raw image is empty".into());
            }
            let mut machine = Machine::bare_metal(ram_bytes, entry)?;
            machine.load_image(load, &bytes)?;
            execute(&mut machine, steps, trace)?;
        }
        Some(command) => return Err(format!("unknown command: {command}\n{HELP}").into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("helios: {error}");
            ExitCode::FAILURE
        }
    }
}
