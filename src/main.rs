use libusb;
use stlink;

use coresight::dap_access::DAPAccess;
use memory::MI;
use probe::debug_probe::DebugProbe;

use rustyline::error::ReadlineError;
use rustyline::Editor;

enum REPLDisconnected {
    Connect { n: u8 },
    Continue,
    Exit,
    Help,
    List,
}

enum REPLConnected {
    Continue,
    Disconnect,
    Dump { loc: u32, words: u32 },
    Exit,
    Help,
    Info,
    Reset,
}

enum Probe {
    NoProbe,
    STLink { probe: stlink::STLink },
}

fn unconnected_repl(rl: &mut rustyline::Editor<()>, prompt: &str) -> REPLDisconnected {
    match rl.readline(prompt) {
        Ok(line) => {
            rl.add_history_entry(line.as_ref());
            match line.split_whitespace().collect::<Vec<&str>>().split_first() {
                Some((&"connect", rest)) => {
                    if !rest.is_empty() {
                        rest[0].parse::<u8>().ok().map_or_else(
                            || {
                                println!("Invalid probe id '{}'", rest[0]);
                                REPLDisconnected::Continue
                            },
                            |n| REPLDisconnected::Connect { n },
                        )
                    } else {
                        println!("Need to supply probe id");
                        REPLDisconnected::Continue
                    }
                }
                Some((&"help", _)) => REPLDisconnected::Help,
                Some((&"list", _)) => REPLDisconnected::List,
                Some((&"exit", _)) | Some((&"quit", _)) => REPLDisconnected::Exit,
                _ => {
                    println!("Sorry, I don't know what '{}' is, try 'help'?", line);
                    REPLDisconnected::Continue
                }
            }
        }
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => REPLDisconnected::Exit,
        Err(err) => {
            println!("Error: {:?}", err);
            REPLDisconnected::Continue
        }
    }
}

fn connected_repl(rl: &mut rustyline::Editor<()>, prompt: &str) -> REPLConnected {
    match rl.readline(prompt) {
        Ok(line) => {
            rl.add_history_entry(line.as_ref());
            match line.split_whitespace().collect::<Vec<&str>>().split_first() {
                Some((&"disconnect", _)) => REPLConnected::Disconnect,
                Some((&"dump", rest)) => match rest.len() {
                    1..=2 => {
                        let words = if rest.len() == 2 {
                            rest[1].parse::<u32>().unwrap_or_else(|_| {
                                println!(
                                    "Cannot parse '{}' as number of words, will use 1 instead",
                                    rest[1]
                                );
                                1
                            })
                        } else {
                            1
                        };

                        u32::from_str_radix(rest[0], 16).ok().map_or_else(
                            || {
                                println!("Cannot parse '{}' as address", rest[0]);
                                REPLConnected::Continue
                            },
                            |loc| REPLConnected::Dump { loc, words },
                        )
                    }
                    _ => {
                        println!("Usage: dump <loc> [n]");
                        REPLConnected::Continue
                    }
                },
                Some((&"help", _)) => REPLConnected::Help,
                Some((&"info", _)) => REPLConnected::Info,
                Some((&"reset", _)) => REPLConnected::Reset,
                Some((&"exit", _)) | Some((&"quit", _)) => REPLConnected::Exit,
                _ => {
                    println!("Sorry, I don't know what '{}' is, try 'help'?", line);
                    REPLConnected::Continue
                }
            }
        }
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => REPLConnected::Exit,
        Err(err) => {
            println!("Error: {:?}", err);
            REPLConnected::Continue
        }
    }
}

fn connect(n: u8) -> Probe {
    let probe = stlink::STLink::new_from_connected(|mut devices| {
        if devices.len() <= n as usize {
            println!("The probe device with the given id '{}' was not found", n);
            Err(libusb::Error::NotFound)
        } else {
            Ok(devices.remove(n as usize).0)
        }
    })
    .map(|mut device| {
        device.attach(probe::protocol::WireProtocol::Swd).ok();
        device
    })
    .unwrap();

    Probe::STLink { probe }
}

// revision | partno | designer | reserved
// 4 bit    | 16 bit | 11 bit   | 1 bit
fn parse_target_id(value: u32) -> (u8, u16, u16, u8) {
    (
        (value >> 28) as u8,
        (value >> 12) as u16,
        ((value >> 1) & 0x07FF) as u16,
        (value & 0x01) as u8,
    )
}

fn dump_memory<D: DebugProbe + MI>(device: &mut D, loc: u32, words: u32) -> Result<(), &str> {
    let mut data = vec![0 as u32; words as usize];

    device
        .read_block(loc, &mut data.as_mut_slice())
        .or_else(|_| Err("Failed to read block from target"))?;

    for word in 0..words {
        if word % 4 == 0 {
            print!("0x{:08x?}: ", loc + 4 * word);
        }

        print!("{:08x} ", data[word as usize]);

        if word % 4 == 3 {
            println!();
        }
    }

    if words % 4 != 0 {
        println!();
    }

    Ok(())
}

fn list_probes() {
    let context = libusb::Context::new().unwrap();
    let plugged_devices = stlink::get_all_plugged_devices(&context);

    println!("The following ST-Link devices were found:");
    if let Ok(devices) = plugged_devices {
        devices.iter().enumerate().for_each(|(num, link)| {
            println!(
                "[{}]: PID = {}, version = {}",
                num, link.1.usb_pid, link.1.version_name
            );
        });
    }
}

fn show_info<D: DebugProbe + DAPAccess>(device: &mut D) -> Result<(), &str> {
    let version = device
        .get_version()
        .or_else(|_| Err("Could not get version"))?;

    println!("Device information:");
    println!("Hardware Version: {:?}", version.0);
    println!("JTAG Version: {:?}", version.1);

    device
        .write_register(0xFFFF, 0x2, 0x2)
        .or_else(|_| Err(""))?;

    let target_info = device.read_register(0xFFFF, 0x4).or_else(|_| Err(""))?;
    let target_info = parse_target_id(target_info);
    println!("Target Identification Register (TARGETID):");
    println!(
        "\tRevision = {}, Part Number = {}, Designer = {}",
        target_info.0, target_info.3, target_info.2
    );

    let target_info = device.read_register(0xFFFF, 0x0).or_else(|_| Err(""))?;
    let target_info = parse_target_id(target_info);
    println!("\nIdentification Code Register (IDCODE):");
    println!(
        "\tProtocol = {},\n\tPart Number = {},\n\tJEDEC Manufacturer ID = {:x}",
        if target_info.0 == 0x4 {
            "JTAG-DP"
        } else if target_info.0 == 0x3 {
            "SW-DP"
        } else {
            "Unknown Protocol"
        },
        target_info.1,
        target_info.2
    );

    if target_info.3 != 1
        || !(target_info.0 == 0x3 || target_info.0 == 0x4)
        || !(target_info.1 == 0xBA00 || target_info.1 == 0xBA02)
    {
        return Err("The IDCODE register has not-expected contents.");
    }
    Ok(())
}

fn reset<D: DebugProbe>(device: &mut D) -> Result<(), &str> {
    device.target_reset().ok();
    Ok(())
}

fn main() {
    let mut connected: Probe = Probe::NoProbe;
    let mut rl = Editor::<()>::new();

    println!("Probemeister at your service!");

    rl.load_history("history.txt").ok();

    loop {
        match connected {
            Probe::NoProbe => match unconnected_repl(&mut rl, "(Not connected) >> ") {
                REPLDisconnected::Help => {
                    println!("The following commands are available:");
                    println!("\tconnect <n>\t- connect to a debugging probe (STLink only for now)");
                    println!("\texit\t\t- exit");
                    println!("\tquit\t\t- exit");
                }
                REPLDisconnected::Connect { n } => {
                    connected = connect(n);
                }
                REPLDisconnected::Exit => break,
                REPLDisconnected::Continue => (),
                REPLDisconnected::List => list_probes(),
            },
            Probe::STLink { ref mut probe } => {
                let prompt = format!("{} >> ", probe.get_name(),);

                match connected_repl(&mut rl, &prompt) {
                    REPLConnected::Help => {
                        println!("The following commands are available:");
                        println!("\tdisconnect\t- disconnect from a debugging probe");
                        println!("\tdump <loc> [n]\t- dump n words of data at address loc from the target");
                        println!("\texit\t\t- exit");
                        println!("\tinfo\t\t- show information about connected probe");
                        println!("\tquit\t\t- exit");
                        println!("\treset\t\t- reset the target");
                    }
                    REPLConnected::Info => {
                        show_info(probe).ok();
                    }
                    REPLConnected::Dump { loc, words } => {
                        dump_memory(probe, loc, words)
                            .map_err(|e| println!("{}", e))
                            .ok();
                    }
                    REPLConnected::Disconnect => {
                        connected = Probe::NoProbe;
                    }
                    REPLConnected::Reset => {
                        reset(probe).ok();
                    }
                    REPLConnected::Exit => break,
                    REPLConnected::Continue => (),
                }
            }
        }
    }

    rl.save_history("history.txt").unwrap();
}
