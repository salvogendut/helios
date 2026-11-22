//! Bounded original-ROM diagnostic output on Ultra 5 serial port A.
use helios::ultra5::{BoardStep, BootFlash, PowerOnReset, ResetNvram, Ultra5ResetBoard};
use helios::v9::V9Cpu;
use std::{
    env, fs,
    io::{self, Write},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let [rom, nvram, steps] = args.as_slice() else {
        return Err("usage: ultra5_serial <1-MiB-flash> <8-KiB-nvram> <step-limit>".into());
    };
    let budget: usize = steps.to_str().ok_or("invalid step limit")?.parse()?;
    if budget == 0 {
        return Err("step limit must be positive".into());
    }
    let mut board = Ultra5ResetBoard::new(
        BootFlash::new(fs::read(rom)?)?,
        ResetNvram::new(fs::read(nvram)?)?,
    );
    let mut cpu = V9Cpu::from(PowerOnReset::ultra_sparc_ii());
    let mut stdout = io::stdout().lock();
    let mut retired = 0;
    let mut traps = 0;
    let mut interrupts = 0;
    for _ in 0..budget {
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
