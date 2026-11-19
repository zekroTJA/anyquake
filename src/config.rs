//! Persistent configuration, stored as TOML in the platform's config
//! directory (`~/.config/anyquake/config.toml` on Linux).

use crate::geometry::Anchor;
use crate::hotkey::Hotkey;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const APP_NAME: &str = "anyquake";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawConfig")]
pub struct Config {
    pub general: General,
    /// The applications toggled by AnyQuake, each with its own hotkey.
    #[serde(rename = "app")]
    pub apps: Vec<App>,
    pub debug: Debug,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            apps: vec![App::default()],
            debug: Debug::default(),
        }
    }
}

/// What is read from the file. Besides the current format it accepts the
/// one of AnyQuake 0.5 and older, which had a single application configured
/// in `[target]`, `[placement]` and `general.hotkey`.
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    general: RawGeneral,
    app: Vec<App>,
    debug: Debug,
    target: Option<Target>,
    placement: Option<Placement>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RawGeneral {
    autostart: bool,
    hotkey: Option<Hotkey>,
}

impl From<RawConfig> for Config {
    fn from(raw: RawConfig) -> Self {
        let mut apps = raw.app;
        let legacy =
            raw.target.is_some() || raw.placement.is_some() || raw.general.hotkey.is_some();
        if apps.is_empty() && legacy {
            apps.push(App {
                name: String::new(),
                hotkey: raw.general.hotkey.unwrap_or_default(),
                target: raw.target.unwrap_or_default(),
                placement: raw.placement.unwrap_or_default(),
            });
        }
        Self {
            general: General {
                autostart: raw.general.autostart,
            },
            apps,
            debug: raw.debug,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Debug {
    /// Also write the log output to a file (see `logging::path`).
    pub log_file: bool,
    /// Most detailed messages written to the log file.
    pub log_level: LogLevel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
        LogLevel::Trace,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Error => "Errors",
            LogLevel::Warn => "Warnings",
            LogLevel::Info => "Info",
            LogLevel::Debug => "Debug",
            LogLevel::Trace => "Trace (very verbose)",
        }
    }

    pub fn filter(self) -> log::LevelFilter {
        match self {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Start anyquake when the user logs in.
    pub autostart: bool,
}

/// One application toggled with its own hotkey.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct App {
    /// Optional display name (tray menu, settings, `anyquake toggle NAME`).
    pub name: String,
    /// Key combination toggling the application's window.
    pub hotkey: Hotkey,
    pub target: Target,
    pub placement: Placement,
}

impl App {
    /// Name shown to the user: the configured name, else the window match,
    /// else the position in the list.
    pub fn label(&self, index: usize) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_owned();
        }
        let matcher = self.target.effective_match();
        if !matcher.is_empty() {
            return matcher;
        }
        format!("Application {}", index + 1)
    }
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

pub fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("could not determine the config directory")?;
    Ok(dir.join(APP_NAME).join("config.toml"))
}

impl Config {
    /// Whether at least one application is set up.
    pub fn is_configured(&self) -> bool {
        self.apps.iter().any(|app| app.target.is_configured())
    }

    /// Finds an application by its 1-based position or (case-insensitively)
    /// by its name as shown to the user.
    pub fn find_app(&self, selector: &str) -> Option<usize> {
        let selector = selector.trim();
        if let Ok(n) = selector.parse::<usize>() {
            return (1..=self.apps.len()).contains(&n).then(|| n - 1);
        }
        (0..self.apps.len()).find(|&i| self.apps[i].label(i).eq_ignore_ascii_case(selector))
    }

    /// The first two applications sharing a hotkey, if any.
    pub fn duplicate_hotkey(&self) -> Option<(usize, usize)> {
        (0..self.apps.len()).find_map(|i| {
            (i + 1..self.apps.len())
                .find(|&j| self.apps[i].hotkey == self.apps[j].hotkey)
                .map(|j| (i, j))
        })
    }

    /// A hotkey not used by any application yet (for a new one).
    pub fn unused_hotkey(&self) -> Hotkey {
        crate::hotkey::function_keys()
            .find(|hotkey| self.apps.iter().all(|app| &app.hotkey != hotkey))
            .unwrap_or_default()
    }

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
        config.general.autostart = true;
        config.apps[0].name = "Terminal".into();
        config.apps[0].target.command = "kitty --class quake".into();
        config.apps[0].placement.monitor = MonitorChoice::Named("DP-2".into());
        config.apps[0].placement.anchor = Anchor::BottomRight;
        config.apps[0].hotkey = "Ctrl+Backtick".parse().unwrap();
        config.apps.push(App {
            hotkey: config.unused_hotkey(),
            ..App::default()
        });
        config.apps[1].target.command = "firefox".into();
        config.debug.log_file = true;
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn empty_file() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.apps.is_empty());
        assert!(!config.is_configured());
    }

    #[test]
    fn legacy_single_app() {
        let config: Config = toml::from_str(
            "[general]\nhotkey = \"Ctrl+Backtick\"\nautostart = true\n\
             [target]\ncommand = \"konsole\"\n[placement]\nheight = 30.0\n",
        )
        .unwrap();
        assert!(config.general.autostart);
        assert_eq!(config.apps.len(), 1);
        let app = &config.apps[0];
        assert_eq!(app.hotkey.to_string(), "Ctrl+Backtick");
        assert_eq!(app.target.command, "konsole");
        assert_eq!(app.placement.height, 30.0);
        assert_eq!(app.placement.width, Placement::default().width);
        // Written back in the new format.
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(
            text.contains("[[app]]") && !text.contains("[target]"),
            "{text}"
        );
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn invalid_hotkey_is_an_error() {
        assert!(toml::from_str::<Config>("[[app]]\nhotkey = \"Ctrl+Foo\"\n").is_err());
        assert!(toml::from_str::<Config>("[general]\nhotkey = \"Ctrl+Foo\"\n").is_err());
    }

    #[test]
    fn find_and_duplicates() {
        let mut config = Config::default();
        config.apps[0].target.command = "konsole".into();
        config.apps.push(App {
            name: "Browser".into(),
            hotkey: config.unused_hotkey(),
            ..App::default()
        });
        assert_eq!(config.apps[1].hotkey.to_string(), "F11");
        assert_eq!(config.find_app("konsole"), Some(0));
        assert_eq!(config.find_app("browser"), Some(1));
        assert_eq!(config.find_app("2"), Some(1));
        assert_eq!(config.find_app("3"), None);
        assert_eq!(config.find_app("0"), None);
        assert_eq!(config.duplicate_hotkey(), None);
        config.apps[1].hotkey = config.apps[0].hotkey.clone();
        assert_eq!(config.duplicate_hotkey(), Some((0, 1)));
    }

    #[test]
    fn partial_file_uses_defaults() {
        let config: Config =
            toml::from_str("[[app]]\n[app.target]\ncommand = \"konsole\"\n").unwrap();
        assert_eq!(config.apps[0].target.command, "konsole");
        assert_eq!(config.apps[0].placement, Placement::default());
        assert!(!config.debug.log_file, "file logging is off by default");
        assert_eq!(config.debug.log_level, LogLevel::Info);
        let debug: Debug = toml::from_str("log_file = true\nlog_level = \"trace\"\n").unwrap();
        assert_eq!(debug.log_level, LogLevel::Trace);
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
