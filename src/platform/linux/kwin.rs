//! KDE Plasma (KWin) backend for Wayland and X11 sessions.
//!
//! Wayland does not allow normal clients to grab global keys or move other
//! applications' windows, so the actual work happens in a KWin script that
//! anyquake generates and loads via D-Bus. The script registers the global
//! shortcut and calls back into this process over D-Bus to start the
//! application.

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

const SHORTCUT: &str = "anyquake-toggle";
const SHORTCUT_TEXT: &str = "AnyQuake: Toggle window";
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

    /// KDE remembers shortcuts registered by scripts and ignores the default
    /// passed by the script once a shortcut was saved. Force the configured
    /// key so changes in anyquake's settings take effect.
    fn assign_hotkey(&self, hotkey: &Hotkey) -> Result<()> {
        let action = vec!["kwin", SHORTCUT, "KWin", SHORTCUT_TEXT];
        let keys = vec![(vec![hotkey.qt_combination(), 0, 0, 0],)];
        self.kglobalaccel("setForeignShortcutKeys", &(&action, keys))?;

        let assigned: Vec<(Vec<i32>,)> = self
            .kglobalaccel("shortcutKeys", &(&action,))?
            .body()
            .deserialize()?;
        if !assigned
            .iter()
            .any(|(k,)| k.first() == Some(&hotkey.qt_combination()))
        {
            log::warn!(
                "KDE did not accept the hotkey {hotkey}; it is probably used by another \
                 shortcut. Free it in System Settings → Keyboard → Shortcuts."
            );
        }
        Ok(())
    }
}

impl Backend for KWin {
    fn name(&self) -> &'static str {
        NAME
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        let p = &config.placement;
        let script_config = json!({
            "match": config.target.effective_match(),
            "reuseExisting": config.target.reuse_existing,
            "managedId": crate::state::load_managed_window(NAME).unwrap_or_default(),
            "monitor": String::from(p.monitor.clone()),
            "anchor": p.anchor.id(),
            "width": p.width,
            "height": p.height,
            "respectPanels": p.respect_panels,
            "alwaysOnTop": p.always_on_top,
            "hotkey": config.general.hotkey.qt_string(),
            "shortcutName": SHORTCUT,
            "shortcutText": SHORTCUT_TEXT,
            "releaseShortcutName": RELEASE_SHORTCUT,
            "releaseShortcutText": RELEASE_SHORTCUT_TEXT,
        });
        let source = SCRIPT_SOURCE.replace("__ANYQUAKE_CONFIG__", &script_config.to_string());
        self.load_script(&source)?;
        self.assign_hotkey(&config.general.hotkey)?;
        log::info!("KWin script loaded, hotkey {}", config.general.hotkey);
        Ok(())
    }

    fn toggle(&mut self) -> Result<()> {
        self.invoke_shortcut(SHORTCUT)
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
