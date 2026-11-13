//! Persistent configuration, stored as TOML in the platform's config
//! directory (`~/.config/anyquake/config.toml` on Linux).

use crate::geometry::Anchor;
use crate::hotkey::Hotkey;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const APP_NAME: &str = "anyquake";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub target: Target,
    pub placement: Placement,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Key combination toggling the target window.
    #[serde(with = "hotkey_serde")]
    pub hotkey: Hotkey,
    /// Start anyquake when the user logs in.
    pub autostart: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Target {
    /// Command line used to start the application when no matching window
    /// exists, e.g. `konsole` or `kitty --single-instance`.
    pub command: String,
    /// Identifies the application's window. Compared case-insensitively
    /// against the window class / app id (Linux), the executable name
    /// (Windows) or the bundle identifier / app name (macOS).
    ///
    /// Defaults to the file name of the command's program if empty.
    pub window_match: String,
    /// Adopt an already open matching window. If disabled, anyquake only
    /// manages a window of an instance it started itself (useful for
    /// terminals where other windows of the same application exist).
    pub reuse_existing: bool,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            command: String::new(),
            window_match: String::new(),
            reuse_existing: true,
        }
    }
}

impl Target {
    pub fn is_configured(&self) -> bool {
        !self.command.trim().is_empty() || !self.window_match.trim().is_empty()
    }

    /// The effective window match string (lower case).
    pub fn effective_match(&self) -> String {
        let explicit = self.window_match.trim();
        if !explicit.is_empty() {
            return explicit.to_lowercase();
        }
        crate::launch::program_stem(&self.command)
    }
}

/// Which screen the window is shown on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum MonitorChoice {
    /// The screen the mouse cursor is currently on.
    #[default]
    Cursor,
    /// The primary screen.
    Primary,
    /// A screen identified by its connector / display name (e.g. `DP-2`).
    /// Falls back to the cursor screen if it is not connected.
    Named(String),
}

impl From<String> for MonitorChoice {
    fn from(value: String) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "cursor" => MonitorChoice::Cursor,
            "primary" => MonitorChoice::Primary,
            _ => MonitorChoice::Named(value.trim().to_owned()),
        }
    }
}

impl From<MonitorChoice> for String {
    fn from(value: MonitorChoice) -> Self {
        match value {
            MonitorChoice::Cursor => "cursor".into(),
            MonitorChoice::Primary => "primary".into(),
            MonitorChoice::Named(name) => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    pub monitor: MonitorChoice,
    pub anchor: Anchor,
    /// Width in percent of the screen.
    pub width: f32,
    /// Height in percent of the screen.
    pub height: f32,
    /// Exclude panels / taskbars / the menu bar from the usable area.
    pub respect_panels: bool,
    /// Keep the window above all other windows while it is shown.
    pub always_on_top: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            monitor: MonitorChoice::Cursor,
            anchor: Anchor::Top,
            width: 100.0,
            height: 50.0,
            respect_panels: true,
            always_on_top: true,
        }
    }
}

mod hotkey_serde {
    use crate::hotkey::Hotkey;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(hotkey: &Hotkey, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hotkey.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Hotkey, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

pub fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("could not determine the config directory")?;
    Ok(dir.join(APP_NAME).join("config.toml"))
}

impl Config {
    /// Loads the configuration. Returns `None` if no config file exists yet.
    pub fn load() -> Result<Option<Config>> {
        let path = config_path()?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(config))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        // Write atomically so the daemon never reads a half written file.
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut config = Config::default();
        config.target.command = "kitty --class quake".into();
        config.placement.monitor = MonitorChoice::Named("DP-2".into());
        config.placement.anchor = Anchor::BottomRight;
        config.general.hotkey = "Ctrl+Backtick".parse().unwrap();
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let config: Config = toml::from_str("[target]\ncommand = \"konsole\"\n").unwrap();
        assert_eq!(config.target.command, "konsole");
        assert_eq!(config.placement, Placement::default());
    }

    #[test]
    fn effective_match() {
        let target = |command: &str, window_match: &str| Target {
            command: command.into(),
            window_match: window_match.into(),
            reuse_existing: true,
        };
        assert_eq!(target("/usr/bin/Kitty --x", "").effective_match(), "kitty");
        assert_eq!(
            target("konsole", "org.kde.Konsole").effective_match(),
            "org.kde.konsole"
        );
    }
}
