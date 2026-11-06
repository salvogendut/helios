use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_S};
use helios::devices::{nvram::Nvram, serial::Channel};
use helios::machine::{
    demo_image, serial_demo_image, trap_demo_image, Machine, DEFAULT_RAM_BYTES, TARGET,
};
use std::error::Error;
use std::io::Write;
use std::process::ExitCode;

const HELP: &str = "Helios — Sun SPARC emulator foundation

Usage:
  helios info
  helios demo [--trace]
  helios trap-demo [--trace]
  helios serial-demo [--trace]
  helios run FILE [--load-address N] [--entry N] [--steps N] [--ram-mib N] [--trace]
  helios boot PROM [--steps N] [--ram-mib N] [--trace]
                   [--serial-in FILE] [--nvram FILE] [--nvram-out NEW_FILE]

FILE is a raw big-endian SPARC instruction/data image, loaded into RAM.
Addresses accept decimal or 0x-prefixed hexadecimal. Entry defaults to the load
address (default 0). Defaults: 1000 CPU steps, 64 MiB RAM. Each completed
instruction or trap entry counts as one step. --trace prints instructions and
traps. Unsupported behavior and CPU error state exit with status 1.
Reaching the step budget exits successfully; it does not mean guest halt.

boot loads a raw PROM (up to 1 MiB) and starts the CPU/MMU at reset.
boot and serial-demo send TTYA bytes to stdout and diagnostics to stderr.
--serial-in supplies bytes as the guest enables and drains its receiver.
NVRAM images contain 8192 bytes; --nvram-out creates a new file without overwriting.
The RTC advances only through the library's explicit time API at this stage.
Polled serial and NVRAM are installed; firmware/OS boot is not yet supported.";

fn number(value: &str) -> Result<u64, Box<dyn Error>> {
    Ok(if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)?
    } else {
        value.parse()?
    })
}

fn report(console: bool, message: std::fmt::Arguments<'_>) {
    if console {
        eprintln!("{message}");
    } else {
        println!("{message}");
    }
}

fn execute(
    machine: &mut Machine,
    steps: u64,
    trace: bool,
    console_input: Option<&[u8]>,
) -> Result<(), Box<dyn Error>> {
    let mut instructions = 0u64;
    let mut traps = 0u64;
    let mut input_position = 0;
    let console = console_input.is_some();
    for _ in 0..steps {
        if let (Some(input), Some(devices)) = (console_input, machine.bus.devices_mut()) {
            while let Some(&byte) = input.get(input_position) {
                if !devices.serial.receive(Channel::A, byte) {
                    break;
                }
                input_position += 1;
            }
        }
        let step = machine.step();
        if console {
            if let Some(devices) = machine.bus.devices_mut() {
                let bytes = devices.serial.take_output(Channel::A);
                if !bytes.is_empty() {
                    let mut stdout = std::io::stdout().lock();
                    stdout.write_all(&bytes)?;
                    stdout.flush()?;
                }
            }
        }
        let step = step.map_err(|fault| {
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
                    report(
                        console,
                        format_args!("pc={pc:08x} npc={npc:08x} instruction={instruction:08x}"),
                    );
                }
            }
            Step::TrapTaken {
                fault,
                trap_type,
                vector,
            } => {
                traps += 1;
                if trace {
                    report(
                        console,
                        format_args!("trap={trap_type:02x} vector={vector:08x} {fault}"),
                    );
                }
            }
        }
    }
    report(console, format_args!(
        "Step budget reached ({steps}: {instructions} instructions, {traps} traps); PC={:#010x}, nPC={:#010x}",
        machine.cpu.pc(),
        machine.cpu.npc()
    ));
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
            println!("Helios {}\nTarget: {TARGET}\nStatus: integer subset, SRMMU, reset/PROM, polled serial and NVRAM/RTC; no firmware or OS boot", env!("CARGO_PKG_VERSION"));
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
            execute(&mut machine, 43, trace, None)?;
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
            execute(&mut machine, 14, trace, None)?;
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
        Some("serial-demo") => {
            let remaining: Vec<_> = args.collect();
            let trace = match remaining.as_slice() {
                [] => false,
                [option] if option == "--trace" => true,
                _ => return Err("usage: helios serial-demo [--trace]".into()),
            };
            let mut machine = Machine::sun4m(8192, &serial_demo_image())?;
            execute(&mut machine, 256, trace, Some(b"!"))?;
            if machine.cpu.read_reg(3) != 0x80
                || machine.cpu.read_reg(5) != 0
                || machine.cpu.read_reg(7) != 1
            {
                return Err(
                    "serial demo failed: IDPROM type/checksum or completion marker differs".into(),
                );
            }
            eprintln!(
                "Serial demo passed: SS-5 IDPROM verified; TTYA transmitted and echoed input"
            );
        }
        Some(command @ ("run" | "boot")) => {
            let boot = command == "boot";
            let file = args
                .next()
                .ok_or("run/boot requires a raw image filename")?;
            let mut load = 0u32;
            let mut entry = None;
            let mut steps = 1000;
            let mut ram_bytes = DEFAULT_RAM_BYTES;
            let mut trace = false;
            let mut serial_in = None;
            let mut nvram_in = None;
            let mut nvram_out = None;
            while let Some(option) = args.next() {
                if option == "--trace" {
                    trace = true;
                    continue;
                }
                if boot && matches!(option.as_str(), "--serial-in" | "--nvram" | "--nvram-out") {
                    let path = args
                        .next()
                        .ok_or_else(|| format!("{option} requires a filename"))?;
                    match option.as_str() {
                        "--serial-in" => serial_in = Some(path),
                        "--nvram" => nvram_in = Some(path),
                        _ => nvram_out = Some(path),
                    }
                    continue;
                }
                if (boot && matches!(option.as_str(), "--load-address" | "--entry"))
                    || !matches!(
                        option.as_str(),
                        "--load-address" | "--entry" | "--steps" | "--ram-mib"
                    )
                {
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
            let mut machine = if boot {
                let nvram = if let Some(path) = nvram_in {
                    Nvram::from_image(&std::fs::read(path)?)?
                } else {
                    Nvram::default()
                };
                Machine::sun4m_with_nvram(ram_bytes, &bytes, nvram)?
            } else {
                let mut machine = Machine::bare_metal(ram_bytes, entry)?;
                machine.load_image(load, &bytes)?;
                machine
            };
            let input = serial_in
                .map(std::fs::read)
                .transpose()?
                .unwrap_or_default();
            let mut output = nvram_out
                .map(|path| {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                })
                .transpose()?;
            let result = execute(
                &mut machine,
                steps,
                trace,
                if boot { Some(&input) } else { None },
            );
            if let Some(file) = &mut output {
                file.write_all(
                    &machine
                        .bus
                        .devices()
                        .expect("boot installs devices")
                        .nvram
                        .image(),
                )?;
            }
            result?;
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
