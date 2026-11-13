//! anyquake: show and hide any application window on the current screen
//! with a global hotkey.

mod autostart;
mod config;
mod controller;
mod daemon;
mod geometry;
mod hotkey;
mod icon;
mod ipc;
mod launch;
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
  ANYQUAKE_BACKEND=x11    Force a backend on Linux (kwin, x11)
";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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
