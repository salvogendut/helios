use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_S};
use helios::machine::{demo_image, trap_demo_image, Machine, DEFAULT_RAM_BYTES, TARGET};
use std::error::Error;
use std::process::ExitCode;

const HELP: &str = "Helios — Sun SPARC emulator foundation

Usage:
  helios info
  helios demo [--trace]
  helios trap-demo [--trace]
  helios run FILE [--load-address N] [--entry N] [--steps N] [--ram-mib N] [--trace]

FILE is a raw big-endian SPARC instruction/data image, loaded into RAM.
Addresses accept decimal or 0x-prefixed hexadecimal. Entry defaults to the load
address (default 0). Defaults: 1000 CPU steps, 64 MiB RAM. Each completed
instruction or trap entry counts as one step. --trace prints instructions and
traps. Unsupported behavior and CPU error state exit with status 1.
Reaching the step budget exits successfully; it does not mean guest halt.

Current mode is a bare-metal interpreter subset. Firmware/OS boot is not supported.";

fn number(value: &str) -> Result<u64, Box<dyn Error>> {
    Ok(if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)?
    } else {
        value.parse()?
    })
}

fn execute(machine: &mut Machine, steps: u64, trace: bool) -> Result<(), Box<dyn Error>> {
    let mut instructions = 0u64;
    let mut traps = 0u64;
    for _ in 0..steps {
        let step = machine.step().map_err(|fault| {
            let reason = if machine.cpu.error_state().is_some() {
                "CPU error state"
            } else {
                "emulator stop"
            };
            format!("{reason}: {fault}")
        })?;
        match step {
            Step::Executed {
                pc,
                npc,
                instruction,
            } => {
                instructions += 1;
                if trace {
                    println!("pc={pc:08x} npc={npc:08x} instruction={instruction:08x}");
                }
            }
            Step::TrapTaken {
                fault,
                trap_type,
                vector,
            } => {
                traps += 1;
                if trace {
                    println!("trap={trap_type:02x} vector={vector:08x} {fault}");
                }
            }
        }
    }
    println!(
        "Step budget reached ({steps}: {instructions} instructions, {traps} traps); PC={:#010x}, nPC={:#010x}",
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
        Some("trap-demo") => {
            let remaining: Vec<_> = args.collect();
            let trace = match remaining.as_slice() {
                [] => false,
                [option] if option == "--trace" => true,
                _ => return Err("usage: helios trap-demo [--trace]".into()),
            };
            let mut machine = Machine::bare_metal(8192, 0)?;
            machine.load_image(0, &trap_demo_image())?;
            execute(&mut machine, 14, trace)?;
            if machine.cpu.read_reg(2) != 1
                || machine.cpu.read_reg(3) != 1
                || machine.cpu.pc() != 0x2c
                || machine.cpu.current_window() != 0
                || machine.cpu.psr() & (PSR_S | PSR_ET) != PSR_ET
            {
                return Err("trap demo failed: incorrect handler or user return state".into());
            }
            println!("Trap demo passed: user TA 7 -> supervisor handler -> user RETT return");
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
