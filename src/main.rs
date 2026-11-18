//! anyquake: show and hide any application window on the current screen
//! with a global hotkey.

// A GUI application on Windows: no console window opens when it is started
// from the Start menu or on login. Output of the command line commands goes
// to the console of the calling shell, see `attach_parent_console`.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod autostart;
mod config;
mod controller;
mod daemon;
mod geometry;
mod hotkey;
mod icon;
mod ipc;
mod launch;
mod logging;
mod platform;
mod settings;
mod state;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: anyquake [COMMAND]

Commands:
  (none)     Start AnyQuake and open the settings window
             (only opens the settings if AnyQuake is already running)
  daemon     Start AnyQuake in the background (used for autostart)
  settings   Open the settings window
  toggle     Show / hide the configured application
  reload     Re-read the configuration file
  quit       Stop the running AnyQuake instance

Environment:
  RUST_LOG=debug          More verbose logging
  ANYQUAKE_BACKEND=x11    Force a backend on Linux (kwin, gnome, x11)
";

fn main() -> ExitCode {
    #[cfg(windows)]
    attach_parent_console();
    logging::init();
    // Errors in the file are reported by the daemon / settings window.
    if let Ok(Some(config)) = config::Config::load() {
        logging::configure(&config.debug);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => start_daemon(true),
        ["daemon"] => start_daemon(false),
        ["settings"] => settings::run(),
        [command @ ("toggle" | "reload" | "quit")] => send_command(command),
        ["-h" | "--help" | "help"] => {
            print!("{USAGE}");
            Ok(())
        }
        ["-V" | "--version"] => {
            println!("anyquake {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Windows GUI programs have no console. When started from a shell, write
/// output (usage, logs) to that shell's console instead. Redirected output
/// (pipes, files) works without this.
#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // Fails when there is no parent console (Start menu, autostart): fine.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

fn start_daemon(open_settings: bool) -> anyhow::Result<()> {
    // Single instance: if a daemon is already running, starting anyquake
    // again just opens the settings window of the running instance.
    match ipc::send(if open_settings { "settings" } else { "ping" }) {
        Ok(true) => {
            log::info!("AnyQuake is already running");
            return Ok(());
        }
        Ok(false) => {}
        // A daemon answered with an error: it is running.
        Err(e) => {
            log::info!("AnyQuake is already running ({e:#})");
            return Ok(());
        }
    }
    platform::run_daemon(open_settings)
}

fn send_command(command: &str) -> anyhow::Result<()> {
    if !ipc::send(command)? {
        anyhow::bail!("AnyQuake is not running");
    }
    Ok(())
}
