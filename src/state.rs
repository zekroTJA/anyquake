//! Small runtime state that survives restarts of the daemon: the identity of
//! the window anyquake manages, so a restarted daemon picks up the same
//! (possibly hidden) window again instead of starting a second instance.

use crate::config::APP_NAME;
use std::fs;
use std::path::PathBuf;

fn path(backend: &str) -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join(APP_NAME);
    Some(dir.join(format!(
        "managed-window-{}",
        backend.to_lowercase().replace(' ', "-")
    )))
}

pub fn load_managed_window(backend: &str) -> Option<String> {
    let text = fs::read_to_string(path(backend)?).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

pub fn save_managed_window(backend: &str, id: &str) {
    let Some(path) = path(backend) else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(&path, id) {
        log::warn!("saving {}: {e}", path.display());
    }
}
