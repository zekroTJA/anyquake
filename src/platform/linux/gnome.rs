//! GNOME Shell backend for Wayland sessions.
//!
//! Like on KDE, Wayland does not let normal clients grab global keys or
//! move other applications' windows, so the work happens inside the
//! compositor: anyquake installs a small GNOME Shell extension that owns the
//! shortcuts and manages the windows. It is configured over D-Bus and calls
//! back into this process to start the applications.
//!
//! GNOME Shell only discovers newly installed extensions at login, so after
//! the first installation (and after updates of the extension) the user has
//! to log out and back in once.

use crate::config::Config;
use crate::daemon::{Backend, Event, ScreenInfo};
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "GNOME Shell";

const EXTENSION_UUID: &str = "anyquake@zekrotja.github.io";
const EXTENSION_SOURCE: &str = include_str!("gnome-extension.js");
/// Bump together with `VERSION` in the extension on every change of it.
const EXTENSION_VERSION: u32 = 2;
/// The first GNOME version with ES module extensions.
const MIN_SHELL_VERSION: u32 = 45;

const EXTENSION_BUS_NAME: &str = "de.anyquake.GnomeShell";
const EXTENSION_PATH: &str = "/de/anyquake/GnomeShell";
const EXTENSION_IFACE: &str = "de.anyquake.GnomeShell";

pub struct Gnome {
    conn: Connection,
}

impl Gnome {
    pub fn new(events: Sender<Event>) -> Result<Self> {
        let conn = super::session_connection(events)?;
        let gnome = Self { conn };
        let shell_major = gnome
            .shell_major_version()
            .context("GNOME Shell is not reachable over D-Bus")?;
        // Problems here are reported again (with more context) by
        // `configure` as long as the extension is not running.
        if let Err(e) = gnome.install_extension(shell_major) {
            log::error!("installing the GNOME Shell extension: {e:#}");
        }
        if let Err(e) = enable_extension() {
            log::error!("enabling the GNOME Shell extension: {e:#}");
        }
        gnome.check_extension();
        Ok(gnome)
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

    fn property(&self, dest: &str, path: &str, iface: &str, name: &str) -> Result<OwnedValue> {
        Ok(self
            .call(
                dest,
                path,
                "org.freedesktop.DBus.Properties",
                "Get",
                &(iface, name),
            )?
            .body()
            .deserialize()?)
    }

    fn shell_major_version(&self) -> Result<u32> {
        let version: String = self
            .property(
                "org.gnome.Shell",
                "/org/gnome/Shell",
                "org.gnome.Shell",
                "ShellVersion",
            )?
            .try_into()?;
        version
            .split('.')
            .next()
            .and_then(|major| major.parse().ok())
            .with_context(|| format!("unexpected GNOME Shell version '{version}'"))
    }

    /// Writes the extension to the user's extension directory.
    fn install_extension(&self, shell_major: u32) -> Result<()> {
        if shell_major < MIN_SHELL_VERSION {
            bail!("GNOME {shell_major} is too old, at least GNOME {MIN_SHELL_VERSION} is required");
        }
        // Also declare the next version, so the extension keeps loading after
        // a GNOME upgrade; the metadata is then updated for the new version
        // before the following upgrade.
        let shell_versions: Vec<String> = (MIN_SHELL_VERSION..=shell_major + 1)
            .map(|v| v.to_string())
            .collect();
        let metadata = json!({
            "uuid": EXTENSION_UUID,
            "name": "AnyQuake",
            "description": "Integration for AnyQuake: global hotkey and window placement. \
                            Installed and configured by the AnyQuake application.",
            "url": env!("CARGO_PKG_REPOSITORY"),
            "version": EXTENSION_VERSION,
            "shell-version": shell_versions,
        });
        let metadata = serde_json::to_string_pretty(&metadata)? + "\n";

        let dir = extension_dir()?;
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut changed = false;
        for (name, content) in [
            ("metadata.json", metadata.as_str()),
            ("extension.js", EXTENSION_SOURCE),
        ] {
            let path = dir.join(name);
            if fs::read_to_string(&path).is_ok_and(|existing| existing == content) {
                continue;
            }
            fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
            changed = true;
        }
        if changed {
            log::info!("installed the GNOME Shell extension to {}", dir.display());
        }
        Ok(())
    }

    fn extension_running(&self) -> Result<bool> {
        Ok(self
            .call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                &(EXTENSION_BUS_NAME,),
            )?
            .body()
            .deserialize()?)
    }

    fn running_extension_version(&self) -> Result<u32> {
        Ok(self
            .property(
                EXTENSION_BUS_NAME,
                EXTENSION_PATH,
                EXTENSION_IFACE,
                "Version",
            )?
            .try_into()?)
    }

    /// Tells the user if the extension is not running (yet) or an outdated
    /// version of it is.
    fn check_extension(&self) {
        let problem = match self.extension_running() {
            Ok(true) => match self.running_extension_version() {
                Ok(EXTENSION_VERSION) => None,
                Ok(_) => Some(
                    "The AnyQuake GNOME Shell extension was updated. Log out and back in once \
                     to load the new version."
                        .into(),
                ),
                Err(e) => Some(format!("{e:#}")),
            },
            Ok(false) => Some(self.not_running_reason()),
            Err(e) => Some(format!("{e:#}")),
        };
        if let Some(problem) = problem {
            log::warn!("{problem}");
            self.notify(&problem);
        }
    }

    /// Explains why the extension is not running, based on what GNOME Shell
    /// reports about it.
    fn not_running_reason(&self) -> String {
        let user_extensions_enabled: Option<bool> = self
            .property(
                "org.gnome.Shell",
                "/org/gnome/Shell",
                "org.gnome.Shell.Extensions",
                "UserExtensionsEnabled",
            )
            .ok()
            .and_then(|v| v.try_into().ok());
        if user_extensions_enabled == Some(false) {
            return "AnyQuake needs its GNOME Shell extension, but extensions are turned off. \
                    Turn them on in the Extensions app."
                .into();
        }
        let info: HashMap<String, OwnedValue> = self
            .call(
                "org.gnome.Shell",
                "/org/gnome/Shell",
                "org.gnome.Shell.Extensions",
                "GetExtensionInfo",
                &(EXTENSION_UUID,),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize().ok())
            .unwrap_or_default();
        let get = |key: &str| info.get(key).map(|v| Value::from(v.clone()));
        // ExtensionState in GNOME Shell's extensionUtils.js.
        let state = match get("state") {
            Some(Value::F64(state)) => state as u32,
            Some(Value::U32(state)) => state,
            _ => 0,
        };
        let error = match get("error") {
            Some(Value::Str(error)) => error.to_string(),
            _ => String::new(),
        };
        match state {
            // Not known to GNOME Shell yet: installed during this session.
            0 => "The AnyQuake GNOME Shell extension was installed. Log out and back \
                  in once to activate it."
                .into(),
            3 => format!("The AnyQuake GNOME Shell extension failed: {error}"),
            4 => "The AnyQuake GNOME Shell extension does not support this GNOME version. \
                  Log out and back in once to load an updated version."
                .into(),
            _ => "The AnyQuake GNOME Shell extension is not active. Make sure it is turned on \
                  in the Extensions app."
                .into(),
        }
    }

    /// Shows a desktop notification.
    fn notify(&self, body: &str) {
        let hints: HashMap<&str, Value> = HashMap::new();
        let result = self.call(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            &(
                "AnyQuake",
                0u32,
                crate::config::APP_NAME,
                "AnyQuake",
                body,
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        );
        if let Err(e) = result {
            log::warn!("showing notification: {e:#}");
        }
    }

    fn extension<B>(&self, method: &str, body: &B) -> Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        if !self.extension_running()? {
            bail!("{}", self.not_running_reason());
        }
        self.call(
            EXTENSION_BUS_NAME,
            EXTENSION_PATH,
            EXTENSION_IFACE,
            method,
            body,
        )
    }
}

impl Backend for Gnome {
    fn name(&self) -> &'static str {
        NAME
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        let apps: Vec<_> = config
            .apps
            .iter()
            .enumerate()
            .map(|(i, app)| {
                let mut json = super::app_json(NAME, i, app);
                json["accelerator"] = app.hotkey.gtk_accelerator().into();
                json
            })
            .collect();
        // Version 1 of the extension (AnyQuake 0.5) is still running until
        // the next login. It only knows a single application, configured by
        // the top level keys.
        let mut extension_config = apps.first().cloned().unwrap_or_else(|| json!({}));
        extension_config["apps"] = apps.into();
        let warning: String = self
            .extension("Configure", &(extension_config.to_string(),))?
            .body()
            .deserialize()?;
        if !warning.is_empty() {
            bail!("{warning}");
        }
        let hotkeys: Vec<_> = config
            .apps
            .iter()
            .map(|app| app.hotkey.to_string())
            .collect();
        log::info!(
            "GNOME Shell extension configured, hotkeys {}",
            hotkeys.join(", ")
        );
        Ok(())
    }

    /// Active monitors from mutter's DisplayConfig interface, named by
    /// connector like the extension expects them.
    fn screens(&mut self) -> Result<Option<Vec<ScreenInfo>>> {
        type Props = HashMap<String, OwnedValue>;
        type MonitorId = (String, String, String, String);
        type Mode = (String, i32, i32, f64, f64, Vec<f64>, Props);
        type Monitor = (MonitorId, Vec<Mode>, Props);
        type Logical = (i32, i32, f64, u32, bool, Vec<MonitorId>, Props);
        let (_serial, monitors, logical, _props): (u32, Vec<Monitor>, Vec<Logical>, Props) = self
            .call(
                "org.gnome.Mutter.DisplayConfig",
                "/org/gnome/Mutter/DisplayConfig",
                "org.gnome.Mutter.DisplayConfig",
                "GetCurrentState",
                &(),
            )?
            .body()
            .deserialize()?;
        let mut screens = Vec::new();
        for (id, modes, _) in monitors {
            // Monitors without a logical monitor are turned off.
            let Some(&(.., primary, _, _)) = logical.iter().find(|l| l.5.contains(&id)) else {
                continue;
            };
            let current = modes.iter().find(|mode| {
                mode.6
                    .get("is-current")
                    .and_then(|v| bool::try_from(v).ok())
                    .unwrap_or(false)
            });
            let (width, height) = current.map(|m| (m.1, m.2)).unwrap_or_default();
            screens.push(ScreenInfo {
                name: id.0,
                width,
                height,
                primary,
            });
        }
        Ok(Some(screens))
    }

    fn toggle(&mut self, app: usize) -> Result<()> {
        let version = self
            .running_extension_version()
            .unwrap_or(EXTENSION_VERSION);
        if version < 2 {
            anyhow::ensure!(
                app == 0,
                "log out and back in once to load the updated GNOME Shell extension, which \
                 supports more than one application"
            );
            self.extension("Toggle", &())?;
        } else {
            self.extension("ToggleApp", &(app as u32,))?;
        }
        Ok(())
    }

    fn release(&mut self) {
        if !self.extension_running().unwrap_or(false) {
            return; // nothing to give back
        }
        if let Err(e) = self.extension("Release", &()) {
            log::warn!("releasing window: {e:#}");
        }
    }
}

fn extension_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("no data directory")?
        .join("gnome-shell/extensions")
        .join(EXTENSION_UUID))
}

/// Adds the extension to GNOME Shell's enabled extensions. GNOME Shell's
/// own `EnableExtension` D-Bus method ignores extensions it has not loaded
/// yet, so the setting is changed directly.
fn enable_extension() -> Result<()> {
    let enabled = gsettings_get_strv("enabled-extensions")?;
    if !enabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
        let mut enabled = enabled;
        enabled.push(EXTENSION_UUID.to_owned());
        gsettings_set_strv("enabled-extensions", &enabled)?;
    }
    // Takes precedence over enabled-extensions, e.g. after turning the
    // extension off in the Extensions app.
    let disabled = gsettings_get_strv("disabled-extensions")?;
    if disabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
        let disabled: Vec<_> = disabled
            .into_iter()
            .filter(|uuid| uuid != EXTENSION_UUID)
            .collect();
        gsettings_set_strv("disabled-extensions", &disabled)?;
    }
    Ok(())
}

fn gsettings(args: &[&str]) -> Result<String> {
    // A gsettings from Homebrew, Nix etc. earlier in PATH may not use the
    // dconf backend GNOME Shell reads its settings from.
    let program = ["/usr/bin/gsettings", "/bin/gsettings"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
        .unwrap_or("gsettings");
    let output = Command::new(program)
        .args(args)
        .output()
        .context("running gsettings")?;
    if !output.status.success() {
        bail!(
            "gsettings {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn gsettings_get_strv(key: &str) -> Result<Vec<String>> {
    parse_strv(&gsettings(&["get", "org.gnome.shell", key])?)
        .with_context(|| format!("unexpected value of org.gnome.shell {key}"))
}

fn gsettings_set_strv(key: &str, values: &[String]) -> Result<()> {
    gsettings(&["set", "org.gnome.shell", key, &format_strv(values)])?;
    Ok(())
}

/// Parses a GVariant string array as printed by gsettings, e.g.
/// `['a@b', 'c']` or `@as []`. Extension UUIDs contain no quotes or commas.
fn parse_strv(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    let text = text.strip_prefix("@as").unwrap_or(text).trim();
    let inner = text.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|item| {
            let item = item.trim();
            item.strip_prefix('\'')
                .and_then(|i| i.strip_suffix('\''))
                .or_else(|| item.strip_prefix('"').and_then(|i| i.strip_suffix('"')))
                .map(str::to_owned)
        })
        .collect()
}

fn format_strv(values: &[String]) -> String {
    let items: Vec<_> = values.iter().map(|v| format!("'{v}'")).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strv_roundtrip() {
        assert_eq!(parse_strv("@as []\n"), Some(vec![]));
        let values = parse_strv("['a@b.c', \"d@e\"]\n").unwrap();
        assert_eq!(values, ["a@b.c", "d@e"]);
        assert_eq!(format_strv(&values), "['a@b.c', 'd@e']");
        assert_eq!(parse_strv("garbage"), None);
    }
}
