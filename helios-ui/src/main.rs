//! Desktop presentation only; the worker owns all guest state and guest time.
mod keys;

use helios::devices::{
    cgsix::{Cgsix, HEIGHT, WIDTH},
    keyboard::Keyboard,
    nvram::Nvram,
    scsi::MediaKind,
    serial::Channel,
};
use helios::machine::{Machine, MachineProfile, DEFAULT_RAM_BYTES};
use minifb::{InputCallback, Key, Window, WindowOptions};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const TITLE_SUFFIX: &str = "Pause = Sun Stop (hold with A)";

struct Display {
    pixels: Vec<u32>,
    stop_title: Option<String>,
}

fn stop_title(error: &helios::cpu::CpuFault) -> String {
    use helios::{
        bus::{BusError, BusErrorKind},
        cpu::FaultKind,
        mmu::MemoryError,
    };
    match error.kind {
        FaultKind::UnsupportedMemory(MemoryError::Bus(BusError {
            kind: BusErrorKind::Unsupported(reason),
            ..
        })) => format!("HELIOS — Stopped: {reason}"),
        _ => format!("HELIOS — Stopped at PC={:08x}; see terminal", error.pc),
    }
}

struct HostKeys {
    down: [bool; Key::Count as usize],
    events: VecDeque<(u8, bool)>,
}

impl HostKeys {
    fn event(&mut self, key: Key, down: bool) {
        if key as usize >= self.down.len() || self.down[key as usize] == down {
            return;
        }
        let Some(code) = keys::station(key) else {
            return;
        };
        let before = keys::KEYS
            .iter()
            .any(|&k| keys::station(k) == Some(code) && self.down[k as usize]);
        self.down[key as usize] = down;
        let after = keys::KEYS
            .iter()
            .any(|&k| keys::station(k) == Some(code) && self.down[k as usize]);
        if before != after {
            self.events.push_back((code, after));
        }
    }
    fn release_all(&mut self) {
        for &key in keys::KEYS {
            self.event(key, false);
        }
    }
}

struct Input(Rc<RefCell<HostKeys>>);
impl InputCallback for Input {
    fn add_char(&mut self, _character: u32) {}
    fn set_key_state(&mut self, key: Key, down: bool) {
        self.0.borrow_mut().event(key, down);
    }
}

fn worker(
    mut machine: Machine,
    input: mpsc::Receiver<(u8, bool)>,
    quit: mpsc::Receiver<()>,
    display: Arc<Mutex<Display>>,
    nvram_out: Option<PathBuf>,
) -> Result<(), String> {
    let mut pending = None;
    let mut frame_at = Instant::now();
    let mut pixels = vec![0; WIDTH * HEIGHT];
    let mut result = Ok(());
    let mut title = None;
    'running: loop {
        if quit.try_recv().is_ok() {
            break;
        }
        for _ in 0..4096 {
            if pending.is_none() {
                pending = input.try_recv().ok();
            }
            if let Some((code, down)) = pending {
                if machine
                    .bus
                    .devices_mut()
                    .unwrap()
                    .keyboard
                    .as_mut()
                    .unwrap()
                    .key(code, down)
                {
                    pending = None;
                }
            }
            if let Err(error) = machine.step() {
                title = Some(stop_title(&error));
                result = Err(format!("Emulation stopped: {error:?}"));
                break 'running;
            }
        }
        let serial = machine
            .bus
            .devices_mut()
            .unwrap()
            .serial
            .take_output(Channel::A);
        if let Err(error) = std::io::stdout().write_all(&serial) {
            title = Some("HELIOS — Stopped: serial output failed".into());
            result = Err(error.to_string());
            break;
        }
        if frame_at.elapsed() >= Duration::from_millis(33) {
            let _ = machine.framebuffer_pixels(&mut pixels);
            display.lock().unwrap().pixels.copy_from_slice(&pixels);
            frame_at = Instant::now();
        }
    }
    let _ = machine.framebuffer_pixels(&mut pixels);
    {
        let mut frame = display.lock().unwrap();
        frame.pixels = pixels;
        frame.stop_title = title;
    }
    if let Some(path) = nvram_out {
        let image = machine.bus.devices().unwrap().nvram.image();
        let saved = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| file.write_all(&image));
        if let Err(error) = saved {
            result = Err(format!(
                "Saving {}: {error}; run result: {result:?}",
                path.display()
            ));
            display.lock().unwrap().stop_title =
                Some("HELIOS — Stopped: saving NVRAM failed; see terminal".into());
        }
    }
    if let Err(error) = &result {
        eprintln!("{error}");
    }
    result
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let usage = "Usage: helios-ui SS5_PROM TURBOGX_ROM [OPTIONS]\n       helios-ui --machine ss20 SS20_PROM [OPTIONS]\n       helios-ui --machine axil320 AXIL_PROM [OPTIONS]\nOptions: [--nvram IMAGE] [--nvram-out NEW_IMAGE] [--disk RAW_IMAGE | --disk-rw RAW_IMAGE] [--cdrom ISO_IMAGE]\nDisk at SCSI ID 3 (512-byte blocks); --disk-rw permits guest writes to an existing image.\n--disk and CD-ROM at ID 6 (2048-byte sectors) remain read-only. Images are never created or truncated.";
    let Some(first) = args.next() else {
        return Err(usage.into());
    };
    if first == "--help" || first == "-h" {
        println!("{usage}\nClose the window to exit. Escape is sent to the guest.\nGuest keyboard layout: Sun Type 5 US. Hold Pause and A for Stop-A.");
        return Ok(());
    }
    let (profile, prom, turbo_rom) = if first == "--machine" {
        let name = args.next().ok_or(usage)?;
        let profile = match name.to_str() {
            Some("ss20") => MachineProfile::Ss20,
            Some("axil320") => MachineProfile::Axil320,
            _ => return Err(usage.into()),
        };
        (profile, args.next().ok_or(usage)?, None)
    } else {
        (MachineProfile::Ss5, first, Some(args.next().ok_or(usage)?))
    };
    let mut nvram = profile.default_nvram();
    let mut nvram_out = None;
    let mut disk = None;
    let mut disk_writable = false;
    let mut cdrom = None;
    while let Some(option) = args.next() {
        match option.to_str() {
            Some("--nvram") => {
                let path = args.next().ok_or("Missing --nvram path")?;
                nvram = Nvram::from_image(&std::fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            }
            Some("--nvram-out") => {
                let path = PathBuf::from(args.next().ok_or("Missing --nvram-out path")?);
                if path.exists() {
                    return Err(format!("Refusing to replace {}", path.display()));
                }
                nvram_out = Some(path);
            }
            Some("--disk" | "--disk-rw") => {
                if disk
                    .replace(PathBuf::from(args.next().ok_or("Missing disk path")?))
                    .is_some()
                {
                    return Err("Choose one --disk or --disk-rw image".into());
                }
                disk_writable = option == "--disk-rw";
            }
            Some("--cdrom") => {
                if cdrom
                    .replace(PathBuf::from(args.next().ok_or("Missing --cdrom path")?))
                    .is_some()
                {
                    return Err("--cdrom supplied more than once".into());
                }
            }
            _ => return Err(usage.into()),
        }
    }
    // Construct shared CPU/bus device state on the thread that owns it.
    let build_machine = move || -> Result<Machine, String> {
        let mut machine = Machine::with_profile(
            profile,
            DEFAULT_RAM_BYTES,
            &std::fs::read(prom).map_err(|e| e.to_string())?,
            nvram,
        )
        .map_err(|e| e.to_string())?;
        for (id, kind, path) in [(3, MediaKind::Disk, disk), (6, MediaKind::Cdrom, cdrom)] {
            if let Some(path) = path {
                let scsi = &mut machine.bus.devices_mut().unwrap().scsi;
                let result = if kind == MediaKind::Disk && disk_writable {
                    scsi.attach_disk_rw(id, &path)
                } else {
                    scsi.attach(id, kind, &path)
                };
                result.map_err(|error| format!("SCSI ID {id}, {}: {error}", path.display()))?;
            }
        }
        if let Some(rom) = turbo_rom {
            machine
                .bus
                .install_cgsix(
                    Cgsix::new(&std::fs::read(rom).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("{e:?}"))?,
                )
                .map_err(|e| e.to_string())?;
        }
        machine.bus.devices_mut().unwrap().keyboard = Some(Keyboard::default());
        Ok(machine)
    };
    let title = format!("HELIOS — {} — {TITLE_SUFFIX}", profile.target());
    let mut window = Window::new(
        &title,
        WIDTH,
        HEIGHT,
        WindowOptions {
            resize: true,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    window.set_target_fps(60);
    let input = Rc::new(RefCell::new(HostKeys {
        down: [false; Key::Count as usize],
        events: VecDeque::new(),
    }));
    window.set_input_callback(Box::new(Input(input.clone())));
    eprint!("{}", include_str!("../../sun_ascii.txt"));
    eprintln!(
        "{title}\n128 MiB RAM, 1152×900 framebuffer. Firmware startup may take a few minutes."
    );
    let display = Arc::new(Mutex::new(Display {
        pixels: vec![0; WIDTH * HEIGHT],
        stop_title: None,
    }));
    let (input_tx, input_rx) = mpsc::sync_channel(256);
    let (quit_tx, quit_rx) = mpsc::channel();
    let shared = display.clone();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let machine = match build_machine() {
            Ok(machine) => machine,
            Err(error) => {
                let _ = ready_tx.send(Err(error.clone()));
                return Err(error);
            }
        };
        if ready_tx.send(Ok(())).is_err() {
            return Ok(());
        }
        worker(machine, input_rx, quit_rx, shared, nvram_out)
    });
    if let Err(error) = ready_rx
        .recv()
        .unwrap_or_else(|_| Err("Machine worker stopped during startup".into()))
    {
        let _ = handle.join();
        return Err(error);
    }
    let mut pixels = vec![0; WIDTH * HEIGHT];
    let mut window_error = None;
    while window.is_open() {
        {
            let frame = display.lock().unwrap();
            pixels.copy_from_slice(&frame.pixels);
            if let Some(title) = &frame.stop_title {
                window.set_title(title);
            }
        }
        if let Err(error) = window.update_with_buffer(&pixels, WIDTH, HEIGHT) {
            window_error = Some(error.to_string());
            break;
        }
        let mut input = input.borrow_mut();
        // Focus loss releases every guest key. No host autorepeat: OBP owns it.
        if !window.is_active() {
            input.release_all();
        }
        let events = &mut input.events;
        while let Some(event) = events.front() {
            match input_tx.try_send(*event) {
                Ok(()) => {
                    events.pop_front();
                }
                Err(mpsc::TrySendError::Full(_)) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    events.clear();
                    break;
                }
            }
        }
        if events.len() > 4096 {
            window_error = Some("Keyboard input queue exceeded its limit".into());
            break;
        }
    }
    let _ = quit_tx.send(());
    let result = handle
        .join()
        .map_err(|_| "Emulator worker panicked".to_string())?;
    if let Some(error) = window_error {
        return Err(error);
    }
    result
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("helios-ui: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn keys() -> HostKeys {
        HostKeys {
            down: [false; Key::Count as usize],
            events: VecDeque::new(),
        }
    }

    #[test]
    fn unsupported_esp_command_is_identified_in_the_window_title() {
        use helios::{
            bus::{BusError, BusErrorKind},
            cpu::{CpuFault, FaultKind},
            mmu::MemoryError,
        };
        let error = CpuFault {
            pc: 0xffd16898,
            npc: 0xffd1689c,
            instruction: None,
            kind: FaultKind::UnsupportedMemory(MemoryError::Bus(BusError {
                address: 0x7880000c,
                kind: BusErrorKind::Unsupported(
                    "ESP command outside asynchronous initiator subset",
                ),
            })),
        };
        assert_eq!(
            stop_title(&error),
            "HELIOS — Stopped: ESP command outside asynchronous initiator subset"
        );
    }

    #[test]
    fn quick_taps_are_preserved_and_host_repeat_is_suppressed() {
        let mut k = keys();
        k.event(Key::A, true);
        k.event(Key::A, true);
        k.event(Key::A, false);
        k.event(Key::A, true);
        k.event(Key::A, false);
        assert_eq!(
            k.events.into_iter().collect::<Vec<_>>(),
            [(0x4d, true), (0x4d, false), (0x4d, true), (0x4d, false)]
        );
    }
    #[test]
    fn shared_control_station_remains_down_until_both_host_keys_release() {
        let mut k = keys();
        k.event(Key::LeftCtrl, true);
        k.event(Key::RightCtrl, true);
        k.event(Key::LeftCtrl, false);
        assert_eq!(k.events.iter().copied().collect::<Vec<_>>(), [(0x4c, true)]);
        k.release_all();
        assert_eq!(
            k.events.into_iter().collect::<Vec<_>>(),
            [(0x4c, true), (0x4c, false)]
        );
    }
    #[test]
    fn focus_loss_releases_stop_and_modifiers_without_reordering_pending_input() {
        let mut k = keys();
        k.event(Key::Pause, true);
        k.event(Key::A, true);
        k.event(Key::LeftShift, true);
        k.release_all();
        k.release_all();
        assert_eq!(
            k.events.into_iter().collect::<Vec<_>>(),
            [
                (1, true),
                (0x4d, true),
                (0x63, true),
                (1, false),
                (0x4d, false),
                (0x63, false)
            ]
        );
    }
}
