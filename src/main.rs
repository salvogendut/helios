use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_S};
use helios::devices::{nvram::Nvram, scsi::MediaKind, serial::Channel};
use helios::machine::{
    demo_image, serial_demo_image, timer_demo_image, trap_demo_image, Machine, CPU_CLOCK_HZ,
    DEFAULT_RAM_BYTES, TARGET,
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
  helios timer-demo [--trace]
  helios run FILE [--load-address N] [--entry N] [--steps N] [--ram-mib N] [--trace]
  helios boot PROM [--steps N] [--ram-mib N] [--trace]
                   [--serial-in FILE] [--nvram FILE] [--nvram-out NEW_FILE]
                   [--disk RAW_IMAGE] [--cdrom ISO_IMAGE]

FILE is a raw big-endian SPARC instruction/data image, loaded into RAM.
Addresses accept decimal or 0x-prefixed hexadecimal. Entry defaults to the load
address (default 0). Defaults: 1000 CPU steps, 128 MiB RAM. Each completed
instruction, synchronous trap or interrupt entry counts as one step. --trace
prints all three separately. Software resets have their own trace/count; their
triggering stores also count as instructions. Unsupported behavior and CPU error
state exit with status 1.
Reaching the step budget exits successfully; it does not mean guest halt.

boot loads a raw PROM (up to 1 MiB) and starts the CPU/MMU at reset.
Its RAM must fit eight 8 MiB or 32 MiB DSIMMs; low slots receive larger modules first.
boot and serial-demo send TTYA bytes to stdout and diagnostics to stderr.
--serial-in supplies bytes as the guest enables and drains its receiver.
NVRAM images contain 8192 bytes; --nvram-out creates a new file without overwriting.
--disk attaches a read-only 512-byte/block raw disk at SCSI ID 3, LUN 0.
--cdrom attaches a read-only 2048-byte/sector CD image at SCSI ID 6, LUN 0.
sun4m clocks advance 100 ns per completed step, a functional scheduling convention.
The library can also advance device time explicitly. Polled serial, NVRAM/RTC,
both SCCs, counter timers, interrupt control and reset/status are installed;
original OBP reaches its monitor; OS boot remains unverified.";

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
    let mut interrupts = 0u64;
    let mut resets = 0u64;
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
            Step::SoftwareReset {
                pc,
                npc,
                instruction,
            } => {
                resets += 1;
                if instruction.is_some() {
                    instructions += 1;
                }
                if trace {
                    let trigger = instruction.map_or_else(
                        || "bus-request".to_owned(),
                        |word| format!("instruction={word:08x}"),
                    );
                    report(
                        console,
                        format_args!("reset=software pc={pc:08x} npc={npc:08x} {trigger}"),
                    );
                }
            }
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
            Step::InterruptTaken {
                pc,
                npc,
                level,
                vector,
            } => {
                interrupts += 1;
                if trace {
                    report(
                        console,
                        format_args!(
                            "interrupt={level} vector={vector:08x} pc={pc:08x} npc={npc:08x}"
                        ),
                    );
                }
            }
        }
    }
    report(console, format_args!(
        "Step budget reached ({steps}: {instructions} instructions, {traps} traps, {interrupts} interrupts, {resets} resets); PC={:#010x}, nPC={:#010x}",
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
            println!("Helios {}\nTarget: {TARGET}\nStatus: integer subset, SRMMU, reset/PROM, polled serial, NVRAM/RTC, interrupts and counter timers; no firmware or OS boot", env!("CARGO_PKG_VERSION"));
            println!("Profile: {} MHz microSPARC-II, {} MiB RAM, TTYA/TTYB\nPlanned devices: CGsix (TurboGX), SCSI disk and CD-ROM, LANCE Ethernet", CPU_CLOCK_HZ / 1_000_000, DEFAULT_RAM_BYTES / (1024 * 1024));
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
        Some("timer-demo") => {
            let remaining: Vec<_> = args.collect();
            let trace = match remaining.as_slice() {
                [] => false,
                [option] if option == "--trace" => true,
                _ => return Err("usage: helios timer-demo [--trace]".into()),
            };
            let mut machine = Machine::sun4m(8192, &timer_demo_image())?;
            execute(&mut machine, 256, trace, None)?;
            if machine.cpu.read_reg(5) != 1
                || machine.cpu.read_reg(7) != 1
                || machine.cpu.read_reg(3) < 2
                || machine.cpu.current_window() != 0
                || machine.cpu.psr() & (PSR_S | PSR_ET) != PSR_S | PSR_ET
            {
                return Err(
                    "timer demo failed: handler count or interrupted-loop return state differs"
                        .into(),
                );
            }
            println!("Timer demo passed: levels 10 and 14 acknowledged; interrupted loop resumed");
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
            let mut disk = None;
            let mut cdrom = None;
            while let Some(option) = args.next() {
                if option == "--trace" {
                    trace = true;
                    continue;
                }
                if boot
                    && matches!(
                        option.as_str(),
                        "--serial-in" | "--nvram" | "--nvram-out" | "--disk" | "--cdrom"
                    )
                {
                    let path = args
                        .next()
                        .ok_or_else(|| format!("{option} requires a filename"))?;
                    match option.as_str() {
                        "--serial-in" => serial_in = Some(path),
                        "--nvram" => nvram_in = Some(path),
                        "--nvram-out" => nvram_out = Some(path),
                        "--disk" => {
                            if disk.replace(path).is_some() {
                                return Err("--disk supplied more than once".into());
                            }
                        }
                        "--cdrom" => {
                            if cdrom.replace(path).is_some() {
                                return Err("--cdrom supplied more than once".into());
                            }
                        }
                        _ => unreachable!(),
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
                Machine::ss5_with_nvram(ram_bytes, &bytes, nvram)?
            } else {
                let mut machine = Machine::bare_metal(ram_bytes, entry)?;
                machine.load_image(load, &bytes)?;
                machine
            };
            for (id, kind, path) in [(3, MediaKind::Disk, disk), (6, MediaKind::Cdrom, cdrom)] {
                if let Some(path) = path {
                    machine
                        .bus
                        .devices_mut()
                        .expect("boot devices")
                        .scsi
                        .attach(id, kind, &path)
                        .map_err(|error| format!("SCSI ID {id}, {path}: {error}"))?;
                }
            }
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
    eprint!("{}", include_str!("../sun_ascii.txt"));
    eprintln!("HELIOS — Hardware Emulator for Legacy Implementations Of SPARC\n");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("helios: {error}");
            ExitCode::FAILURE
        }
    }
}
