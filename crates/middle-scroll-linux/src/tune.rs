//! `--tune`: an interactive prompt that changes the running daemon's engine
//! settings live through the control socket, then saves them to the config
//! file on request.

use std::io::{self, IsTerminal, Write};

use crate::control;

const HELP: &str = "\
Commands:
  get [key]          show the live settings, or one of them
  set <key> <value>  change a setting now (e.g. `set max_speed_detents_per_second 40`)
  save               write the settings changed here to the config file
  reload             drop unsaved changes and reload the config file
  pause | resume     pause or resume autoscroll
  status             show the daemon state
  help               show this help
  quit               leave (unsaved changes stay live until the next reload)

Useful keys: deadzone_units, full_speed_units, min_speed_detents_per_second,
max_speed_detents_per_second, acceleration_exponent, mode, axis_lock, inertia,
inertia_half_life_ms, invert_vertical, invert_horizontal, horizontal_scroll.";

/// Runs the prompt until `quit` or end of input. Returns false when the
/// daemon could not be reached at all.
pub fn run() -> anyhow::Result<bool> {
    // Fail early, with the usual message, when no daemon is running.
    let reply = control::send("ping")?;
    let (ok, message) = control::split_reply(&reply);
    if !ok {
        eprintln!("error: {message}");
        return Ok(false);
    }

    let interactive = io::stdin().is_terminal();
    if interactive {
        println!("Connected to the running daemon. Type `help` for commands, `quit` to leave.");
    }
    let mut buffer = String::new();
    let mut unsaved = false;
    loop {
        if interactive {
            print!("tune> ");
            io::stdout().flush()?;
        }
        buffer.clear();
        if io::stdin().read_line(&mut buffer)? == 0 {
            break;
        }
        let line = buffer.trim();
        match line {
            "" => continue,
            "quit" | "exit" | "q" => break,
            "help" | "?" => {
                println!("{HELP}");
                continue;
            }
            _ => {}
        }
        if let Err(err) = control::Command::parse(line) {
            eprintln!("error: {err} (type `help`)");
            continue;
        }
        let reply = match control::send(line) {
            Ok(reply) => reply,
            Err(err) => {
                eprintln!("error: {err}");
                return Ok(true);
            }
        };
        let (ok, message) = control::split_reply(&reply);
        if ok {
            if line.starts_with("set") {
                unsaved = true;
            } else if line == "save" || line == "reload" {
                unsaved = false;
            }
            if !message.is_empty() {
                println!("{message}");
            }
        } else {
            eprintln!("error: {message}");
        }
    }
    if unsaved && interactive {
        println!("Unsaved changes stay active until the daemon reloads its config; run `save` to keep them.");
    }
    Ok(true)
}
