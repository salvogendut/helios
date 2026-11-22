//! Bounded original-ROM diagnostic output on Ultra 5 serial port A.
use helios::ultra5::{BoardStep, BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard};
use helios::v9::V9Cpu;
use std::{
    env, fs,
    io::{self, Read, Write},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let interactive = args.last().is_some_and(|arg| arg == "--interactive");
    let [rom, nvram, steps] = &args[..args.len() - usize::from(interactive)] else {
        return Err(
            "usage: ultra5_serial <1-MiB-flash> <8-KiB-nvram> <step-limit> [--interactive]".into(),
        );
    };
    let budget: u64 = steps.to_str().ok_or("invalid step limit")?.parse()?;
    if budget == 0 {
        return Err("step limit must be positive".into());
    }
    let mut board = Ultra5ResetBoard::new(
        BootFlash::new(fs::read(rom)?)?,
        ResetNvram::new(fs::read(nvram)?)?,
    );
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    let mut stdout = io::stdout().lock();
    let (tx, rx) = std::sync::mpsc::sync_channel(32);
    if interactive {
        std::thread::spawn(move || {
            for byte in io::stdin().lock().bytes() {
                let Ok(b) = byte else { break };
                if tx.send(if b == b'\n' { b'\r' } else { b }).is_err() {
                    break;
                }
            }
        });
    }
    let mut retired = 0u64;
    let mut traps = 0u64;
    let mut interrupts = 0u64;
    let mut pending = None;
    let mut next_poll = 0;
    let mut steps = 0;
    while steps < budget {
        if interactive && steps >= next_poll {
            let byte = pending.take().or_else(|| rx.try_recv().ok());
            if let Some(b) = byte {
                let delivered = if b == 0x1d {
                    board.receive_serial_break()
                } else {
                    board.receive_serial_a(b)
                };
                if !delivered {
                    pending = Some(b);
                }
            }
            // One 8N1 byte at 9600 baud is about 10,417 board outcomes.
            next_poll = steps + if byte.is_some() { 10417 } else { 100_000 };
        }
        if steps % 257 == 0 {
            let limit = if interactive {
                (next_poll - steps).min(budget - steps)
            } else {
                budget - steps
            };
            let repeated = board.repeat_countdown(&mut cpu, limit);
            if repeated != 0 {
                retired += repeated;
                steps += repeated;
                continue;
            }
        }
        steps += 1;
        let outcome = board.step_outcome(&mut cpu).map_err(|error| {
            format!(
                "retired={retired} traps={traps} interrupts={interrupts} PC={:#018x} nPC={:#018x}: {error}",
                cpu.pc(),
                cpu.npc()
            )
        })?;
        match outcome {
            BoardStep::Retired(_) => retired += 1,
            BoardStep::TrapTaken { .. } => traps += 1,
            BoardStep::InterruptTaken { .. } => interrupts += 1,
        }
        let [port_a, _] = board.take_serial_output();
        if !port_a.is_empty() {
            stdout.write_all(&port_a)?;
            stdout.flush()?;
        }
    }
    eprintln!(
        "retired={retired} traps={traps} interrupts={interrupts} PC={:#018x} nPC={:#018x}",
        cpu.pc(),
        cpu.npc()
    );
    Ok(())
}
