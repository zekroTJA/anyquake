//! Local socket used to control the running daemon (single instance,
//! `anyquake toggle`, settings window → daemon reload, ...).
//!
//! Protocol: the client sends one command per line, the server answers with
//! one line: `ok`, `ok <data>` (for queries) or `error: <message>`.

use crate::daemon::{Event, ScreenInfo};
use anyhow::{Context, Result, bail};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerOptions, Name, Stream};
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::Sender;

fn socket_name() -> Result<Name<'static>> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        // A file in the per-user runtime directory is only reachable by the
        // current user. macOS has no runtime dir, but $TMPDIR is per-user.
        let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        let path = dir.join("anyquake.sock");
        Ok(path.to_fs_name::<GenericFilePath>()?.into_owned())
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        let user = std::env::var("USERNAME").unwrap_or_default();
        Ok(format!("anyquake-{user}")
            .to_ns_name::<GenericNamespaced>()?
            .into_owned())
    }
}

/// Sends a command to a running daemon. Returns `Ok(false)` if no daemon is
/// running.
pub fn send(command: &str) -> Result<bool> {
    Ok(query(command)?.is_some())
}

/// The connected screens as reported by the running daemon. `Ok(None)` if no
/// daemon is running.
pub fn screens() -> Result<Option<Vec<ScreenInfo>>> {
    match query("screens")? {
        Some(data) => Ok(Some(
            serde_json::from_str(&data).context("unexpected answer from daemon")?,
        )),
        None => Ok(None),
    }
}

/// Sends a command to a running daemon and returns the data of its answer
/// (empty for plain commands). `Ok(None)` if no daemon is running.
fn query(command: &str) -> Result<Option<String>> {
    let stream = match Stream::connect(socket_name()?) {
        Ok(stream) => stream,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e).context("connecting to anyquake daemon"),
    };
    let mut stream = BufReader::new(stream);
    stream
        .get_mut()
        .write_all(format!("{command}\n").as_bytes())?;
    let mut answer = String::new();
    stream.read_line(&mut answer)?;
    let answer = answer.trim();
    if answer == "ok" {
        return Ok(Some(String::new()));
    }
    match answer.strip_prefix("ok ") {
        Some(data) => Ok(Some(data.to_owned())),
        None => bail!("{}", answer.strip_prefix("error: ").unwrap_or(answer)),
    }
}

/// Translates a command into an event and waits for the result where the
/// daemon reports one. Returns the data to answer with (usually none).
fn handle_command(line: &str, emit: &impl Fn(Event)) -> Result<String, String> {
    let line = line.trim();
    if line == "screens" {
        let (tx, rx) = std::sync::mpsc::channel();
        emit(Event::QueryScreens(tx));
        let screens = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "no answer from daemon".to_owned())??;
        // JSON without pretty printing has no line breaks.
        return serde_json::to_string(&screens).map_err(|e| e.to_string());
    }
    if let Some(selector) = line.strip_prefix("toggle ") {
        let (tx, rx) = std::sync::mpsc::channel();
        emit(Event::ToggleNamed(selector.trim().to_owned(), Some(tx)));
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "no answer from daemon".to_owned())??;
        return Ok(String::new());
    }
    let event = match line {
        "ping" => return Ok(String::new()),
        "toggle" => Event::Toggle(0),
        "settings" => Event::OpenSettings,
        "quit" => Event::Quit,
        "reload" => {
            let (tx, rx) = std::sync::mpsc::channel();
            emit(Event::Reload(Some(tx)));
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|_| "no answer from daemon".to_owned())??;
            return Ok(String::new());
        }
        other => return Err(format!("unknown command '{other}'")),
    };
    emit(event);
    Ok(String::new())
}

/// Starts listening for commands on a background thread. Every received
/// command is forwarded to `emit`.
pub fn serve(emit: impl Fn(Event) + Send + 'static) -> Result<()> {
    let listener = ListenerOptions::new()
        .name(socket_name()?)
        // Replaces a stale socket file left behind by a crashed daemon. The
        // caller checks beforehand that no daemon is running.
        .try_overwrite(true)
        .create_sync()
        .context("creating control socket")?;

    std::thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let conn = match conn {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::warn!("ipc: incoming connection failed: {e}");
                        continue;
                    }
                };
                let mut conn = BufReader::new(conn);
                let mut line = String::new();
                if conn.read_line(&mut line).is_err() {
                    continue;
                }
                let answer = match handle_command(&line, &emit) {
                    Ok(data) if data.is_empty() => "ok\n".to_owned(),
                    Ok(data) => format!("ok {data}\n"),
                    Err(e) => format!("error: {}\n", e.replace('\n', " ")),
                };
                let _ = conn.get_mut().write_all(answer.as_bytes());
            }
        })?;
    Ok(())
}

/// Forwards events to a channel (the common case for [`serve`]).
pub fn to_channel(tx: Sender<Event>) -> impl Fn(Event) + Send + 'static {
    move |event| {
        let _ = tx.send(event);
    }
}
