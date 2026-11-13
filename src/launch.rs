//! Starting the target application and other helper processes.

use anyhow::{Context, Result, bail};
use std::process::{Command, Stdio};

/// Splits a command line into program and arguments.
///
/// On Unix this follows POSIX shell quoting rules. On Windows backslashes
/// are literal (paths!) and only double quotes group words.
pub fn split_command(command: &str) -> Option<Vec<String>> {
    if cfg!(windows) {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut has_word = false;
        for c in command.chars() {
            match c {
                '"' => {
                    in_quotes = !in_quotes;
                    has_word = true;
                }
                c if c.is_whitespace() && !in_quotes => {
                    if has_word {
                        args.push(std::mem::take(&mut current));
                        has_word = false;
                    }
                }
                c => {
                    current.push(c);
                    has_word = true;
                }
            }
        }
        if in_quotes {
            return None;
        }
        if has_word {
            args.push(current);
        }
        Some(args)
    } else {
        shlex::split(command)
    }
}

/// Returns the lower-cased file stem of the command's program, e.g.
/// `kitty` for `/usr/bin/kitty --single-instance`.
pub fn program_stem(command: &str) -> String {
    let program = split_command(command)
        .and_then(|a| a.into_iter().next())
        .unwrap_or_default();
    let file = program.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.eq_ignore_ascii_case("exe") => stem,
        _ => file,
    };
    stem.to_lowercase()
}

/// Spawns `command` fully detached from anyquake, so it keeps running when
/// anyquake exits.
pub fn spawn_detached(command: &str) -> Result<()> {
    let args = split_command(command).context("invalid quoting in command")?;
    let Some((program, args)) = args.split_first() else {
        bail!("no command configured");
    };

    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group: Ctrl+C in a terminal running anyquake must not
        // kill the managed application.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting '{command}'"))?;
    log::info!("started '{command}' (pid {})", child.id());
    // Reap the child when it exits so no zombie stays around.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Starts the settings window as a separate process. The caller is
/// responsible for reaping the returned child.
pub fn spawn_settings() -> Result<std::process::Child> {
    spawn_self(&["settings"]).context("starting settings window")
}

/// Starts the anyquake executable with the given arguments.
pub fn spawn_self(args: &[&str]) -> Result<std::process::Child> {
    let exe = std::env::current_exe().context("locating anyquake executable")?;
    Ok(Command::new(exe).args(args).stdin(Stdio::null()).spawn()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        assert_eq!(program_stem("/usr/bin/Kitty --x"), "kitty");
        assert_eq!(program_stem("konsole"), "konsole");
        assert_eq!(program_stem("'/opt/My App/app.bin' -a"), "app.bin");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths() {
        assert_eq!(program_stem(r#""C:\Program Files\WT\wt.exe" -w 0"#), "wt");
    }
}
