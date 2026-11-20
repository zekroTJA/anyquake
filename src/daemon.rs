//! The long running background process: owns the window backend and reacts
//! to hotkeys, tray actions and IPC commands.

use crate::config::Config;
use crate::{autostart, launch};
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long to wait for the window of a freshly started application before
/// reporting the launch as failed. The KWin script and the GNOME Shell
/// extension use the same value.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Something the daemon has to react to.
/// Receives the outcome of a request (used to report errors back to the
/// settings window).
pub type Reply = Sender<Result<(), String>>;

#[derive(Debug, Clone)]
pub enum Event {
    /// Show or hide the window of the application with this index.
    Toggle(usize),
    /// Show or hide the window of the application with this (1-based)
    /// number or name (`anyquake toggle NAME`).
    ToggleNamed(String, Option<Reply>),
    /// Re-read the configuration file.
    Reload(Option<Reply>),
    /// Open the settings window.
    OpenSettings,
    /// Exit anyquake.
    Quit,
    /// The settings window asks for the connected screens.
    QueryScreens(ScreensReply),
    /// The compositor side (KWin script) reports the connected screens.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    ScreensReported(Vec<ScreenInfo>),
    /// A backend asks the daemon to start the application with this index.
    Launch(usize),
    /// A backend reports what toggling the application with this index did
    /// (shown, hidden, launching...), for the log.
    Toggled(usize, String),
    /// The screen layout changed (monitor added/removed, resolution, ...).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    ScreensChanged,
    /// A backend reports the identity of the window managed for the
    /// application with this index (persisted so it is found again after a
    /// restart).
    Managed(usize, String),
}

/// A connected screen, as listed in the settings window.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenInfo {
    /// The name used in the configuration (`MonitorChoice::Named`).
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

/// Receives the connected screens (for the settings window).
pub type ScreensReply = Sender<Result<Vec<ScreenInfo>, String>>;

/// Platform specific window management.
pub trait Backend {
    fn name(&self) -> &'static str;

    /// Applies a (new) configuration. Called on startup and on every reload.
    /// Applications are referred to by their index in `config.apps`.
    fn configure(&mut self, config: &Config) -> Result<()>;

    /// Shows or hides the window of application `app`.
    fn toggle(&mut self, app: usize) -> Result<()>;

    /// The connected screens, named like the configuration expects them.
    /// `None` if the backend can't query them directly (see
    /// [`Event::ScreensReported`]).
    fn screens(&mut self) -> Result<Option<Vec<ScreenInfo>>> {
        Ok(None)
    }

    /// Re-applies the placement after the screen layout changed.
    fn screens_changed(&mut self) -> Result<()> {
        Ok(())
    }

    /// If `Some`, [`Backend::tick`] should be called after this delay (used
    /// to wait for a freshly launched application's window).
    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    fn tick(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called before anyquake exits: give the windows back to the user
    /// (visible in the taskbar again etc.).
    fn release(&mut self) {}
}

pub struct Daemon {
    backend: Box<dyn Backend>,
    config: Config,
    /// The settings window process while it is open.
    settings: Arc<Mutex<Option<Child>>>,
    /// Applications started but without a window reported yet: when they
    /// were started and whether the process already failed (and that was
    /// logged).
    launches: HashMap<usize, (Instant, Arc<AtomicBool>)>,
    /// Screens last reported by the compositor side, for backends that
    /// can't query them (KWin).
    reported_screens: Option<Vec<ScreenInfo>>,
}

impl Daemon {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        log::info!("using backend: {}", backend.name());
        Self {
            backend,
            config: Config::default(),
            settings: Arc::default(),
            launches: HashMap::new(),
            reported_screens: None,
        }
    }

    /// Loads the configuration and initializes the backend.
    pub fn start(&mut self, open_settings: bool) {
        let loaded = match Config::load() {
            Ok(config) => config,
            Err(e) => {
                log::error!("{e:#}; using default configuration");
                Some(Config::default())
            }
        };
        let first_run = loaded.is_none();
        self.config = loaded.unwrap_or_default();
        crate::logging::configure(&self.config.debug);
        if let Err(e) = self.backend.configure(&self.config) {
            log::error!("configuring backend: {e:#}");
        }
        if let Err(e) = autostart::sync(self.config.general.autostart) {
            log::warn!("autostart: {e:#}");
        }
        if open_settings || first_run || !self.config.is_configured() {
            self.open_settings();
        }
    }

    /// Handles an event. Returns `false` if the daemon should exit.
    pub fn handle(&mut self, event: Event) -> bool {
        log::debug!("event: {event:?}");
        let result = match event {
            Event::Toggle(app) => self.toggle(app),
            Event::ToggleNamed(selector, reply) => {
                let result = match self.config.find_app(&selector) {
                    Some(app) => self.toggle(app),
                    None => Err(anyhow!("no application '{selector}' configured")),
                };
                if let Some(reply) = reply {
                    let _ = reply.send(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
                }
                result
            }
            Event::Reload(reply) => {
                let result = self.reload();
                if let Some(reply) = reply {
                    let _ = reply.send(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
                }
                result
            }
            Event::OpenSettings => {
                self.open_settings();
                Ok(())
            }
            Event::Launch(app) => self.launch(app),
            Event::Toggled(app, action) => {
                if let Some(config) = self.config.apps.get(app) {
                    log::debug!(
                        "toggled {} ({}): {action}",
                        config.label(app),
                        config.hotkey
                    );
                }
                Ok(())
            }
            Event::ScreensChanged => self.backend.screens_changed(),
            Event::QueryScreens(reply) => {
                let screens = match self.backend.screens() {
                    Ok(Some(screens)) => Ok(screens),
                    Ok(None) => self
                        .reported_screens
                        .clone()
                        .ok_or_else(|| "the screens are not known yet".to_owned()),
                    Err(e) => Err(format!("{e:#}")),
                };
                let _ = reply.send(screens);
                Ok(())
            }
            Event::ScreensReported(screens) => {
                log::debug!("screens: {screens:?}");
                self.reported_screens = Some(screens);
                Ok(())
            }
            Event::Managed(app, id) => {
                if self.launches.remove(&app).is_some()
                    && let Some(config) = self.config.apps.get(app)
                {
                    log::info!("window of {} appeared", config.label(app));
                }
                if let Some(config) = self.config.apps.get(app) {
                    let matcher = config.target.effective_match();
                    crate::state::save_managed_window(self.backend.name(), app, &matcher, &id);
                }
                Ok(())
            }
            Event::Quit => {
                self.backend.release();
                if let Some(mut child) = self.settings.lock().unwrap().take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return false;
            }
        };
        if let Err(e) = result {
            log::error!("{e:#}");
        }
        true
    }

    /// Names of the configured applications, for the tray menu.
    pub fn app_labels(&self) -> Vec<String> {
        let apps = self.config.apps.iter().enumerate();
        apps.map(|(i, app)| app.label(i)).collect()
    }

    fn toggle(&mut self, app: usize) -> Result<()> {
        if let Some(config) = self.config.apps.get(app)
            && config.target.is_configured()
        {
            log::debug!("toggling {} ({})", config.label(app), config.hotkey);
            self.backend.toggle(app)
        } else {
            log::warn!("application {} is not configured", app + 1);
            self.open_settings();
            Ok(())
        }
    }

    /// Starts application `app` and watches for its window to appear.
    fn launch(&mut self, app: usize) -> Result<()> {
        let Some(config) = self.config.apps.get(app) else {
            return Ok(());
        };
        let label = config.label(app);
        log::info!("starting {label}: {}", config.target.command);
        let failed = launch::spawn_detached(&config.target.command, &label)
            .with_context(|| format!("failed to launch {label}"))?;
        self.launches.insert(app, (Instant::now(), failed));
        Ok(())
    }

    /// Reports launches whose window did not appear in time.
    fn check_launches(&mut self) {
        let expired: Vec<usize> = self
            .launches
            .iter()
            .filter(|(_, (since, _))| since.elapsed() >= LAUNCH_TIMEOUT)
            .map(|(&app, _)| app)
            .collect();
        for app in expired {
            let Some((_, failed)) = self.launches.remove(&app) else {
                continue;
            };
            let Some(config) = self.config.apps.get(app) else {
                continue;
            };
            if failed.load(Ordering::Relaxed) {
                continue; // the failed process was reported already
            }
            log::error!(
                "failed to launch {}: no window matching '{}' appeared within {} s after \
                 running '{}'. Check the command and the 'window match' setting.",
                config.label(app),
                config.target.effective_match(),
                LAUNCH_TIMEOUT.as_secs(),
                config.target.command
            );
        }
    }

    pub fn poll_interval(&self) -> Option<Duration> {
        let launch = self
            .launches
            .values()
            .map(|(since, _)| LAUNCH_TIMEOUT.saturating_sub(since.elapsed()))
            .min();
        match (self.backend.poll_interval(), launch) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn tick(&mut self) {
        self.check_launches();
        if let Err(e) = self.backend.tick() {
            log::error!("{e:#}");
        }
    }

    fn reload(&mut self) -> Result<()> {
        let config = Config::load()?.unwrap_or_default();
        if config.general.autostart != self.config.general.autostart {
            autostart::sync(config.general.autostart)?;
        }
        crate::logging::configure(&config.debug);
        // Launches of applications that changed can't be matched anymore.
        let old = std::mem::replace(&mut self.config, config);
        self.launches.retain(|&app, _| {
            old.apps.get(app).map(|a| &a.target) == self.config.apps.get(app).map(|a| &a.target)
        });
        self.backend.configure(&self.config)?;
        log::info!("configuration reloaded");
        Ok(())
    }

    fn open_settings(&mut self) {
        let mut slot = self.settings.lock().unwrap();
        if slot.is_some() {
            log::info!("settings window is already open");
            return;
        }
        match launch::spawn_settings() {
            Ok(child) => *slot = Some(child),
            Err(e) => {
                log::error!("{e:#}");
                return;
            }
        }
        // Reap the process as soon as the window is closed, so it can be
        // opened again and does not linger as a zombie.
        let settings = self.settings.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let mut slot = settings.lock().unwrap();
                match slot.as_mut().map(Child::try_wait) {
                    Some(Ok(None)) => continue,
                    Some(_) => *slot = None,
                    None => {}
                }
                break;
            }
        });
    }
}
