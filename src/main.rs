use helios::bus::Bus;
use helios::cpu::{Step, PSR_ET, PSR_S};
use helios::devices::{keyboard::Keyboard, nvram::Nvram, scsi::MediaKind, serial::Channel};
use helios::machine::{
    demo_image, serial_demo_image, timer_demo_image, trap_demo_image, Machine, MachineProfile,
    CPU_CLOCK_HZ, DEFAULT_RAM_BYTES, TARGET,
};
use helios::net::{NatConfig, NatEngine, DNS_PORT};
use helios::serial_host::TcpSerial;
use std::error::Error;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "Helios — Sun SPARC emulator foundation

Usage:
  helios info
  helios demo [--trace]
  helios trap-demo [--trace]
  helios serial-demo [--trace]
  helios timer-demo [--trace]
  helios run FILE [--load-address N] [--entry N] [--steps N] [--ram-mib N] [--trace]
  helios boot PROM [--machine ss5|ss20|axil320] [--steps N] [--ram-mib N] [--trace]
                   [--serial-in FILE] [--serial-port-a PORT] [--serial-port-b PORT]
                   [--keyboard] [--nvram FILE] [--nvram-out NEW_FILE]
                   [--disk RAW_IMAGE | --disk-rw RAW_IMAGE] [--cdrom ISO_IMAGE]
                   [--net] [--net-gateway IP] [--net-client IP] [--net-mask IP]
                   [--net-dns IP] [--tftp-dir DIR]

FILE is a raw big-endian SPARC instruction/data image, loaded into RAM.
Addresses accept decimal or 0x-prefixed hexadecimal. Entry defaults to the load
address (default 0). Defaults: 1000 CPU steps, 128 MiB RAM. Each completed
instruction, synchronous trap or interrupt entry counts as one step. --trace
prints all three separately. Software resets have their own trace/count; their
triggering stores also count as instructions. Unsupported behavior and CPU error
state exit with status 1.
Reaching the step budget exits successfully; it does not mean guest halt.

boot loads a raw PROM (up to 1 MiB) and starts the CPU/MMU at reset.
--machine selects the initial profile (default ss5): SS-5, single-processor
SS-20 or the Axil 320 clone. SS-20/Axil use the high sun4m device map and a
36-bit boot window but the same device models and 128 MiB default RAM.
Its RAM must fit eight 8 MiB or 32 MiB DSIMMs; low slots receive larger modules first.
boot and serial-demo send TTYA bytes to stdout and diagnostics to stderr.
--keyboard attaches a Type 5 keyboard; with diag-switch? false this selects the
abbreviated OBP startup path, letting an SS-20 boot reach ok without full POST.
--serial-in supplies bytes as the guest enables and drains its receiver.
--serial-port-a PORT and --serial-port-b PORT expose TTYA/TTYB as telnet
consoles on 127.0.0.1; PORT 0 asks the host for a free port. Console output
still goes to stdout. NVRAM images contain 8192 bytes; --nvram-out creates a
new file without overwriting.
--disk attaches a read-only 512-byte/block raw disk at SCSI ID 3, LUN 0.
--disk-rw attaches an existing raw disk at ID 3 and permits guest writes.
Neither disk option creates or truncates an image; choose only one.
--cdrom attaches a read-only 2048-byte/sector CD image at SCSI ID 6, LUN 0.
--net enables the user-mode NAT (guest 192.168.0.2, gateway 192.168.0.1,
mask 255.255.255.0, DHCP/RARP, DNS relay and UDP). --net-gateway, --net-client,
--net-mask and --net-dns override its addresses. --tftp-dir adds the read-only
TFTP server for network boot. TCP NAT is not implemented yet.
sun4m clocks advance 100 ns per completed step, a functional scheduling convention.
The library can also advance device time explicitly. Polled serial, NVRAM/RTC,
both SCCs, counter timers, interrupt control and reset/status are installed;
original OBP and Solaris 2.6/7 serial installer prompts have been verified.";

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

#[derive(Default)]
struct SerialPorts {
    a: Option<TcpSerial>,
    b: Option<TcpSerial>,
}

fn execute(
    machine: &mut Machine,
    steps: u64,
    trace: bool,
    console_input: Option<&[u8]>,
    ports: &mut SerialPorts,
    mut network: Option<&mut NatEngine>,
) -> Result<(), Box<dyn Error>> {
    let mut instructions = 0u64;
    let mut traps = 0u64;
    let mut interrupts = 0u64;
    let mut resets = 0u64;
    let mut input_position = 0;
    let console = console_input.is_some();
    for _ in 0..steps {
        if let Some(devices) = machine.bus.devices_mut() {
            if let Some(input) = console_input {
                while let Some(&byte) = input.get(input_position) {
                    if !devices.serial.receive(Channel::A, byte) {
                        break;
                    }
                    input_position += 1;
                }
            }
            if let Some(host) = ports.a.as_mut() {
                while let Some(byte) = host.receive() {
                    if !devices.serial.receive(Channel::A, byte) {
                        break;
                    }
                }
            }
            if let Some(host) = ports.b.as_mut() {
                while let Some(byte) = host.receive() {
                    if !devices.serial.receive(Channel::B, byte) {
                        break;
                    }
                }
            }
        }
        let step = machine.step();
        if console || ports.a.is_some() || ports.b.is_some() {
            if let Some(devices) = machine.bus.devices_mut() {
                let bytes = devices.serial.take_output(Channel::A);
                if !bytes.is_empty() {
                    if let Some(host) = ports.a.as_mut() {
                        host.send(&bytes);
                    }
                    if console {
                        let mut stdout = std::io::stdout().lock();
                        stdout.write_all(&bytes)?;
                        stdout.flush()?;
                    }
                }
                if ports.b.is_some() {
                    let bytes = devices.serial.take_output(Channel::B);
                    if !bytes.is_empty() {
                        if let Some(host) = ports.b.as_mut() {
                            host.send(&bytes);
                        }
                    }
                }
            }
        }
        if step.is_ok() {
            if let Some(net) = network.as_mut() {
                if let Some(devices) = machine.bus.devices_mut() {
                    let now = std::time::Instant::now();
                    while let Some(frame) = devices.ethernet.take_frame() {
                        net.input(&frame, now);
                    }
                    net.poll(now);
                    while let Some(frame) = net.output() {
                        if !devices.ethernet.receive_frame(&frame) {
                            break;
                        }
                    }
                }
            }
        }
        let step = match step {
            Ok(step) => step,
            Err(fault) if fault.is_power_off() => {
                report(console, format_args!("System powered off."));
                return Ok(());
            }
            Err(fault) => {
                let reason = if machine.cpu.error_state().is_some() {
                    "CPU error state"
                } else {
                    "emulator stop"
                };
                return Err(format!("{reason}: {fault}").into());
            }
        };
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
    if let Some(net) = network {
        let stats = net.stats();
        eprintln!(
            "Host NAT: {} guest frames, {} host frames, {} ARP, {} RARP, {} BOOTP, {} ICMP, {} UDP, {} DNS, {} TFTP, {} dropped, {} unsupported TCP",
            stats.guest_frames,
            stats.host_frames,
            stats.arp_requests,
            stats.rarp_requests,
            stats.bootp_requests,
            stats.icmp_echoes,
            stats.udp_requests,
            stats.dns_queries,
            stats.tftp_datagrams,
            stats.dropped,
            stats.unsupported_tcp
        );
    }
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
            println!("Helios {}\nTarget: {TARGET}\nMachine profiles: ss5 (default), ss20, axil320\nStatus: integer subset, SRMMU, reset/PROM, polled serial, NVRAM/RTC, interrupts and counter timers; original SS-5/SS-20/Axil firmware prompts verified, no OS boot", env!("CARGO_PKG_VERSION"));
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
            execute(
                &mut machine,
                43,
                trace,
                None,
                &mut SerialPorts::default(),
                None,
            )?;
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
            execute(
                &mut machine,
                14,
                trace,
                None,
                &mut SerialPorts::default(),
                None,
            )?;
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
            execute(
                &mut machine,
                256,
                trace,
                Some(b"!"),
                &mut SerialPorts::default(),
                None,
            )?;
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
            execute(
                &mut machine,
                256,
                trace,
                None,
                &mut SerialPorts::default(),
                None,
            )?;
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
            let mut profile = MachineProfile::Ss5;
            let mut serial_in = None;
            let mut serial_port_a = None;
            let mut serial_port_b = None;
            let mut net = false;
            let mut net_gateway = None;
            let mut net_client = None;
            let mut net_mask = None;
            let mut net_dns = None;
            let mut tftp_dir = None;
            let mut keyboard = false;
            let mut nvram_in = None;
            let mut nvram_out = None;
            let mut disk = None;
            let mut disk_writable = false;
            let mut cdrom = None;
            while let Some(option) = args.next() {
                if option == "--trace" {
                    trace = true;
                    continue;
                }
                if boot
                    && matches!(
                        option.as_str(),
                        "--serial-in"
                            | "--nvram"
                            | "--nvram-out"
                            | "--disk"
                            | "--disk-rw"
                            | "--cdrom"
                    )
                {
                    let path = args
                        .next()
                        .ok_or_else(|| format!("{option} requires a filename"))?;
                    match option.as_str() {
                        "--serial-in" => serial_in = Some(path),
                        "--nvram" => nvram_in = Some(path),
                        "--nvram-out" => nvram_out = Some(path),
                        "--disk" | "--disk-rw" => {
                            if disk.replace(path).is_some() {
                                return Err("Choose one --disk or --disk-rw image".into());
                            }
                            disk_writable = option == "--disk-rw";
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
                if boot && option == "--keyboard" {
                    keyboard = true;
                    continue;
                }
                if boot && option == "--machine" {
                    let value = args
                        .next()
                        .ok_or_else(|| "--machine requires ss5, ss20 or axil320".to_owned())?;
                    profile = match value.as_str() {
                        "ss5" => MachineProfile::Ss5,
                        "ss20" => MachineProfile::Ss20,
                        "axil320" => MachineProfile::Axil320,
                        _ => return Err(format!("unknown machine profile: {value}").into()),
                    };
                    continue;
                }
                if boot && option == "--net" {
                    net = true;
                    continue;
                }
                if boot
                    && matches!(
                        option.as_str(),
                        "--net-gateway"
                            | "--net-client"
                            | "--net-mask"
                            | "--net-dns"
                            | "--tftp-dir"
                    )
                {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("{option} requires a value"))?;
                    match option.as_str() {
                        "--net-gateway" => net_gateway = Some(value.parse::<Ipv4Addr>()?),
                        "--net-client" => net_client = Some(value.parse::<Ipv4Addr>()?),
                        "--net-mask" => net_mask = Some(value.parse::<Ipv4Addr>()?),
                        "--net-dns" => net_dns = Some(value.parse::<Ipv4Addr>()?),
                        "--tftp-dir" => tftp_dir = Some(PathBuf::from(value)),
                        _ => unreachable!(),
                    }
                    continue;
                }
                if (boot && matches!(option.as_str(), "--load-address" | "--entry"))
                    || !matches!(
                        option.as_str(),
                        "--load-address"
                            | "--entry"
                            | "--steps"
                            | "--ram-mib"
                            | "--serial-port-a"
                            | "--serial-port-b"
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
                    "--serial-port-a" => serial_port_a = Some(value.try_into()?),
                    "--serial-port-b" => serial_port_b = Some(value.try_into()?),
                    _ => unreachable!(),
                }
            }
            if steps == 0 {
                return Err("--steps must be greater than zero".into());
            }
            if !boot && (serial_port_a.is_some() || serial_port_b.is_some()) {
                return Err(
                    "--serial-port-a and --serial-port-b require boot (sun4m devices)".into(),
                );
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
                    profile.default_nvram()
                };
                Machine::with_profile(profile, ram_bytes, &bytes, nvram)?
            } else {
                let mut machine = Machine::bare_metal(ram_bytes, entry)?;
                machine.load_image(load, &bytes)?;
                machine
            };
            if boot && keyboard {
                machine.bus.devices_mut().expect("boot devices").keyboard =
                    Some(Keyboard::default());
            }
            for (id, kind, path) in [(3, MediaKind::Disk, disk), (6, MediaKind::Cdrom, cdrom)] {
                if let Some(path) = path {
                    let scsi = &mut machine.bus.devices_mut().expect("boot devices").scsi;
                    let result = if kind == MediaKind::Disk && disk_writable {
                        scsi.attach_disk_rw(id, &path)
                    } else {
                        scsi.attach(id, kind, &path)
                    };
                    result.map_err(|error| format!("SCSI ID {id}, {path}: {error}"))?;
                }
            }
            let mut ports = SerialPorts::default();
            for (label, channel, port) in [
                ("TTYA", Channel::A, serial_port_a),
                ("TTYB", Channel::B, serial_port_b),
            ] {
                if let Some(port) = port {
                    let host = TcpSerial::bind(port)
                        .map_err(|error| format!("{label} serial port {port}: {error}"))?;
                    eprintln!("{label} serial console on telnet 127.0.0.1:{}", host.port());
                    match channel {
                        Channel::A => ports.a = Some(host),
                        Channel::B => ports.b = Some(host),
                    }
                }
            }
            let mut network = if net
                || net_gateway.is_some()
                || net_client.is_some()
                || net_mask.is_some()
                || net_dns.is_some()
                || tftp_dir.is_some()
            {
                let mut config = NatConfig::default();
                if let Some(address) = net_gateway {
                    config.gateway_ip = address;
                }
                if let Some(address) = net_client {
                    config.client_ip = address;
                }
                if let Some(address) = net_mask {
                    config.netmask = address;
                }
                if let Some(address) = net_dns {
                    config.dns = SocketAddrV4::new(address, DNS_PORT);
                }
                if let Some(path) = tftp_dir {
                    if !path.is_dir() {
                        return Err(
                            format!("--tftp-dir {} is not a directory", path.display()).into()
                        );
                    }
                    config.tftp_dir = Some(path);
                }
                eprintln!(
                    "Host NAT: guest {} gateway {} mask {} DNS {}",
                    config.client_ip, config.gateway_ip, config.netmask, config.dns
                );
                if let Some(path) = &config.tftp_dir {
                    eprintln!("TFTP root on {}:69: {}", config.gateway_ip, path.display());
                }
                Some(NatEngine::new(config))
            } else {
                None
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
                &mut ports,
                network.as_mut(),
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
