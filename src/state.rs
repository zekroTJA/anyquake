//! Small runtime state that survives restarts of the daemon: the identity of
//! the windows anyquake manages, so a restarted daemon picks up the same
//! (possibly hidden) windows again instead of starting second instances.

use crate::config::APP_NAME;
use std::fs;
use std::path::PathBuf;

/// One file per backend and application. The first application uses the
/// file name of AnyQuake 0.5 and older, which only had one.
fn path(backend: &str, app: usize) -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join(APP_NAME);
    let backend = backend.to_lowercase().replace(' ', "-");
    let name = match app {
        0 => format!("managed-window-{backend}"),
        n => format!("managed-window-{backend}-{}", n + 1),
    };
    Some(dir.join(name))
}

/// The saved window id of application `app`, if it was saved for the same
/// window match (the list of applications may have been changed since).
pub fn load_managed_window(backend: &str, app: usize, matcher: &str) -> Option<String> {
    let text = fs::read_to_string(path(backend, app)?).ok()?;
    let mut lines = text.lines().map(str::trim);
    let id = lines.next().filter(|id| !id.is_empty())?;
    // Files written by older versions have no match line.
    match lines.next() {
        Some(saved) if saved != matcher => None,
        _ => Some(id.to_owned()),
    }
}

pub fn save_managed_window(backend: &str, app: usize, matcher: &str, id: &str) {
    let Some(path) = path(backend, app) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(&path, format!("{id}\n{matcher}\n")) {
        log::warn!("saving {}: {e}", path.display());
    }
}
