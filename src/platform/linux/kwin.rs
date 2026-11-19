//! KDE Plasma (KWin) backend for Wayland and X11 sessions.
//!
//! Wayland does not allow normal clients to grab global keys or move other
//! applications' windows, so the actual work happens in a KWin script that
//! anyquake generates and loads via D-Bus. The script registers a global
//! shortcut per application and calls back into this process over D-Bus to
//! start the application.

use crate::config::{APP_NAME, Config};
use crate::daemon::{Backend, Event};
use crate::hotkey::Hotkey;
use anyhow::{Context, Result};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;
use zbus::blocking::Connection;

const NAME: &str = "KWin";
const SCRIPT_NAME: &str = "anyquake";
const SCRIPT_SOURCE: &str = include_str!("kwin.js");

/// Prefix of the toggle shortcuts' names, see [`shortcut_name`].
const SHORTCUT: &str = "anyquake-toggle";
const RELEASE_SHORTCUT: &str = "anyquake-release";
const RELEASE_SHORTCUT_TEXT: &str = "AnyQuake: Release window (internal)";

pub struct KWin {
    conn: Connection,
    script_path: PathBuf,
}

impl KWin {
    pub fn new(events: Sender<Event>) -> Result<Self> {
        let conn = super::session_connection(events)?;
        let script_path = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(APP_NAME)
            .join("kwin-script.js");
        let kwin = Self { conn, script_path };
        // Fail early with a clear message if KWin is not reachable.
        kwin.is_script_loaded()
            .context("KWin is not reachable over D-Bus")?;
        Ok(kwin)
    }

    fn call<B>(
        &self,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(dest), path, Some(iface), method, body)
            .with_context(|| format!("D-Bus call {iface}.{method}"))
    }

    fn scripting<B>(&self, method: &str, body: &B) -> Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.call(
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
            method,
            body,
        )
    }

    fn is_script_loaded(&self) -> Result<bool> {
        Ok(self
            .scripting("isScriptLoaded", &(SCRIPT_NAME,))?
            .body()
            .deserialize()?)
    }

    fn unload_script(&self) -> Result<()> {
        if self.is_script_loaded()? {
            self.scripting("unloadScript", &(SCRIPT_NAME,))?;
        }
        Ok(())
    }

    fn load_script(&self, source: &str) -> Result<()> {
        if let Some(dir) = self.script_path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&self.script_path, source)
            .with_context(|| format!("writing {}", self.script_path.display()))?;

        self.unload_script()?;
        let path = self.script_path.to_string_lossy();
        let id: i32 = self
            .scripting("loadScript", &(&*path, SCRIPT_NAME))?
            .body()
            .deserialize()?;
        anyhow::ensure!(id >= 0, "KWin refused to load the script ({path})");
        self.call(
            "org.kde.KWin",
            &format!("/Scripting/Script{id}"),
            "org.kde.kwin.Script",
            "run",
            &(),
        )?;
        Ok(())
    }

    fn kglobalaccel<B>(&self, method: &str, body: &B) -> Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.call(
            "org.kde.kglobalaccel",
            "/kglobalaccel",
            "org.kde.KGlobalAccel",
            method,
            body,
        )
    }

    fn invoke_shortcut(&self, name: &str) -> Result<()> {
        self.call(
            "org.kde.kglobalaccel",
            "/component/kwin",
            "org.kde.kglobalaccel.Component",
            "invokeShortcut",
            &(name,),
        )?;
        Ok(())
    }

    /// Removes toggle shortcuts of applications that are no longer
    /// configured. KDE keeps them (and their keys) otherwise.
    fn remove_stale_shortcuts(&self, current: &[String]) -> Result<()> {
        let actions: Vec<Vec<String>> = self
            .kglobalaccel("allActionsForComponent", &(vec!["kwin"],))?
            .body()
            .deserialize()?;
        for action in actions {
            let Some(name) = action.get(1) else { continue };
            if name.starts_with(SHORTCUT) && !current.contains(name) {
                log::debug!("removing stale shortcut {name}");
                self.kglobalaccel("unregister", &("kwin", name.as_str()))?;
            }
        }
        Ok(())
    }

    fn set_shortcut_keys(&self, name: &str, text: &str, keys: Option<i32>) -> Result<Vec<i32>> {
        let action = vec!["kwin", name, "KWin", text];
        let keys: Vec<(Vec<i32>,)> = keys.map(|k| (vec![k, 0, 0, 0],)).into_iter().collect();
        self.kglobalaccel("setForeignShortcutKeys", &(&action, keys))?;
        let assigned: Vec<(Vec<i32>,)> = self
            .kglobalaccel("shortcutKeys", &(&action,))?
            .body()
            .deserialize()?;
        Ok(assigned
            .into_iter()
            .filter_map(|(k,)| k.first().copied())
            .collect())
    }

    /// KDE remembers shortcuts registered by scripts and ignores the default
    /// passed by the script once a shortcut was saved. Force the configured
    /// keys so changes in anyquake's settings take effect.
    fn assign_hotkeys(&self, shortcuts: &[(String, String, &Hotkey)]) -> Result<()> {
        // Clear all keys first, so applications can swap hotkeys.
        for (name, text, _) in shortcuts {
            self.set_shortcut_keys(name, text, None)?;
        }
        for (name, text, hotkey) in shortcuts {
            let wanted = hotkey.qt_combination();
            if !self
                .set_shortcut_keys(name, text, Some(wanted))?
                .contains(&wanted)
            {
                log::warn!(
                    "KDE did not accept the hotkey {hotkey}; it is probably used by another \
                     shortcut. Free it in System Settings → Keyboard → Shortcuts."
                );
            }
        }
        Ok(())
    }
}

/// Name of the toggle shortcut of application `index`. The first one keeps
/// the name used by AnyQuake 0.5 and older.
fn shortcut_name(index: usize) -> String {
    match index {
        0 => SHORTCUT.to_owned(),
        n => format!("{SHORTCUT}-{}", n + 1),
    }
}

impl Backend for KWin {
    fn name(&self) -> &'static str {
        NAME
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        let shortcuts: Vec<_> = config
            .apps
            .iter()
            .enumerate()
            .map(|(i, app)| {
                let text = format!("AnyQuake: Toggle {}", app.label(i));
                (shortcut_name(i), text, &app.hotkey)
            })
            .collect();
        let apps: Vec<_> = config
            .apps
            .iter()
            .zip(&shortcuts)
            .enumerate()
            .map(|(i, (app, (name, text, _)))| {
                let mut json = super::app_json(NAME, i, app);
                json["hotkey"] = app.hotkey.qt_string().into();
                json["shortcutName"] = name.as_str().into();
                json["shortcutText"] = text.as_str().into();
                json
            })
            .collect();
        let script_config = json!({
            "apps": apps,
            "releaseShortcutName": RELEASE_SHORTCUT,
            "releaseShortcutText": RELEASE_SHORTCUT_TEXT,
        });
        let source = SCRIPT_SOURCE.replace("__ANYQUAKE_CONFIG__", &script_config.to_string());
        self.load_script(&source)?;
        let names: Vec<_> = shortcuts.iter().map(|(name, ..)| name.clone()).collect();
        if let Err(e) = self.remove_stale_shortcuts(&names) {
            log::warn!("removing old shortcuts: {e:#}");
        }
        self.assign_hotkeys(&shortcuts)?;
        let hotkeys: Vec<_> = config
            .apps
            .iter()
            .map(|app| app.hotkey.to_string())
            .collect();
        log::info!("KWin script loaded, hotkeys {}", hotkeys.join(", "));
        Ok(())
    }

    fn toggle(&mut self, app: usize) -> Result<()> {
        self.invoke_shortcut(&shortcut_name(app))
    }
    fn release(&mut self) {
        if let Err(e) = self.invoke_shortcut(RELEASE_SHORTCUT) {
            log::warn!("releasing window: {e:#}");
        }
        // invokeShortcut is delivered to KWin via kglobalaccel; give it a
        // moment before the script disappears.
        std::thread::sleep(Duration::from_millis(200));
        if let Err(e) = self.unload_script() {
            log::warn!("unloading KWin script: {e:#}");
        }
    }
}
