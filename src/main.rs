use libusb;
use stlink;

use coresight::dap_access::DAPAccess;
use probe::debug_probe::DebugProbe;

use rustyline::error::ReadlineError;
use rustyline::Editor;

enum REPLResult {
    Connect { n: u8 },
    Continue,
    Disconnect,
    Exit,
    Help,
    Info,
    Reset,
}

fn repl(rl: &mut rustyline::Editor<()>, probe: &mut Option<impl DebugProbe>) -> REPLResult {
    let context = libusb::Context::new().unwrap();
    let plugged_devices = stlink::get_all_plugged_devices(&context);

    let readline = rl.readline(&format!(
        "{} >> ",
        probe.as_ref().map_or("(Not connected)", |p| p.get_name())
    ));
    match readline {
        Ok(line) => {
            rl.add_history_entry(line.as_ref());
            match line.split_whitespace().collect::<Vec<&str>>().split_first() {
                Some((&"connect", rest)) => {
                    if rest.len() > 0 {
                        rest[0].parse::<u8>().ok().map_or_else(
                            || {
                                println!("Invalid probe id '{}'", rest[0]);
                                REPLResult::Continue
                            },
                            |n| REPLResult::Connect { n },
                        )
                    } else {
                        println!("Need to supply probe id");
                        REPLResult::Continue
                    }
                }
                Some((&"disconnect", _)) => REPLResult::Disconnect,
                Some((&"help", _)) => REPLResult::Help,
                Some((&"info", _)) => REPLResult::Info,
                Some((&"list", _)) => match &plugged_devices {
                    Ok(connected_devices) => {
                        println!("The following devices were found:");
                        connected_devices
                            .iter()
                            .enumerate()
                            .for_each(|(num, link)| {
                                println!(
                                    "[{}]: PID = {}, version = {}",
                                    num, link.1.usb_pid, link.1.version_name
                                );
                            });
                        REPLResult::Continue
                    }
                    Err(_) => REPLResult::Continue,
                },
                Some((&"reset", _)) => REPLResult::Reset,
                Some((&"exit", _)) | Some((&"quit", _)) => REPLResult::Exit,
                _ => {
                    println!("Sorry, I don't know what '{}' is, try 'help'?", line);
                    REPLResult::Continue
                }
            }
        }
        Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => REPLResult::Exit,
        Err(err) => {
            println!("Error: {:?}", err);
            REPLResult::Continue
        }
    }
}

fn connect(n: u8) -> Option<stlink::STLink> {
    stlink::STLink::new_from_connected(|mut devices| {
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
    .ok()
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

fn show_info(device: &mut stlink::STLink) -> Result<(), &str> {
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

fn reset(device: &mut stlink::STLink) -> Result<(), &str> {
    device.target_reset().ok();
    Ok(())
}

fn main() {
    let mut probe: Option<stlink::STLink> = None;
    let mut rl = Editor::<()>::new();

    println!("Probemeister at your service!");

    rl.load_history("history.txt").ok();

    loop {
        match repl(&mut rl, &mut probe) {
            REPLResult::Help => {
                println!("The following commands are available:");
                println!("\tconnect <n>\t- connect to a debugging probe (STLink only for now)");
                println!("\tdisconnect\t- disconnect from a debugging probe");
                println!("\texit\t\t- exit");
                println!("\tinfo\t\t- show information about connected probe");
                println!("\tquit\t\t- exit");
                println!("\treset\t\t- reset the target");
            }
            REPLResult::Info => {
                probe.as_mut().map_or_else(
                    || println!("Not connected, did you mean to 'connect' first?"),
                    |mut probe| {
                        show_info(&mut probe).ok();
                    },
                );
            }
            REPLResult::Disconnect => {
                probe = None;
            }
            REPLResult::Connect { n } => {
                probe = connect(n);
            }
            REPLResult::Reset => {
                probe.as_mut().map(|mut p| reset(&mut p).ok());
            }
            REPLResult::Exit => break,
            _ => (),
        }
    }

    rl.save_history("history.txt").unwrap();
}
