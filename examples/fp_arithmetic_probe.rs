//! Line-based guest arithmetic probe for tools/check_fp_arithmetic.py.
use helios::bus::Bus;
use helios::cpu::{PSR_EF, PSR_ET, PSR_S};
use helios::machine::Machine;
use std::io::{self, BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let single_opf = match std::env::args().nth(1).as_deref().unwrap_or("div") {
        "add" => 0x41,
        "sub" => 0x45,
        "mul" => 0x49,
        "div" => 0x4d,
        _ => return Err("operation must be add, sub, mul or div".into()),
    };
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let words: Vec<_> = line.split_whitespace().collect();
        if words.len() != 5 {
            return Err("expected: precision operand_a operand_b rounding trap_mask".into());
        }
        let double = match words[0] {
            "32" => false,
            "64" => true,
            _ => return Err("precision must be 32 or 64".into()),
        };
        let a = u64::from_str_radix(words[1], 16)?;
        let b = u64::from_str_radix(words[2], 16)?;
        let rounding = u32::from_str_radix(words[3], 16)?;
        let mask = u32::from_str_radix(words[4], 16)?;
        if rounding > 3 || mask > 31 || (!double && (a > u32::MAX.into() || b > u32::MAX.into())) {
            return Err("input outside the selected field/format".into());
        }
        let mut machine = Machine::bare_metal(4096, 0)?;
        let opf = single_opf + u32::from(double);
        let program = [
            0x8188_6000u32,
            0x0100_0000,
            0x0100_0000,
            0x0100_0000,
            0xc108_a000,
            0x0100_0000,
            0x0100_0000,
            0x0100_0000,
            0x91a0_0002 | (opf << 5),
        ];
        let bytes: Vec<_> = program.into_iter().flat_map(u32::to_be_bytes).collect();
        machine.load_image(0, &bytes)?;
        machine.cpu.write_reg(1, PSR_S | PSR_EF | PSR_ET);
        machine.cpu.write_reg(2, 0x100);
        machine
            .bus
            .write32(0x100, (rounding << 30) | (mask << 23) | 0xc7b)?;
        for (index, value) in [(0, a), (2, b)] {
            machine.cpu.fpu.write_reg(
                index,
                if double {
                    (value >> 32) as u32
                } else {
                    value as u32
                },
            );
            if double {
                machine.cpu.fpu.write_reg(index + 1, value as u32);
            }
        }
        machine.cpu.fpu.write_reg(8, 0xdec0_adde);
        machine.cpu.fpu.write_reg(9, 0xa5a5_5a5a);
        for _ in 0..9 {
            machine.step()?;
        }
        writeln!(
            output,
            "{:08x} {:08x} {:08x} {}",
            machine.cpu.fpu.read_reg(8),
            machine.cpu.fpu.read_reg(9),
            machine.cpu.fpu.fsr(),
            u8::from(machine.cpu.fpu.deferred_entry().is_some())
        )?;
    }
    Ok(())
}
