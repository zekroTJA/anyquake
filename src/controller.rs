//! Generic toggle logic on top of a small set of window system primitives.
//! Used by the X11, Windows and macOS backends (KWin runs the equivalent
//! logic inside a KWin script).

use crate::config::{Config, MonitorChoice, Placement};
use crate::daemon::{Backend, Event, LAUNCH_TIMEOUT};
use crate::geometry::{self, Anchor, Rect};
use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const LAUNCH_POLL: Duration = Duration::from_millis(50);
/// How often the screen layout is compared on platforms without change
/// notifications (see [`WindowOps::POLL_SCREENS`]).
const SCREEN_POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub primary: bool,
    /// Full screen geometry.
    pub full: Rect,
    /// Geometry without panels / taskbars / menu bar.
    pub work: Rect,
}

#[derive(Clone, Copy, Debug)]
pub struct ShowParams {
    pub rect: Rect,
    pub fullscreen: bool,
    pub always_on_top: bool,
    /// Whether the window should also be activated (focused).
    pub activate: bool,
}

/// Window system primitives needed by [`Controller`].
pub trait WindowOps {
    type Window: Copy + Eq + Hash + Debug;

    /// The platform does not report screen layout changes, so the
    /// controller has to poll [`WindowOps::monitors`].
    const POLL_SCREENS: bool = false;

    fn name(&self) -> &'static str;

    /// Called whenever the configuration changes (e.g. to grab the hotkeys,
    /// which then send [`Event::Toggle`] with the application's index).
    fn configure(&mut self, _config: &Config) -> Result<()> {
        Ok(())
    }

    /// Called on every controller tick, i.e. every [`SCREEN_POLL`] if
    /// [`WindowOps::POLL_SCREENS`] is set.
    fn poll(&mut self) {}

    /// All windows matching `matcher` (lower case class / exe / bundle id).
    fn find_all(&mut self, matcher: &str) -> Result<Vec<Self::Window>>;

    /// Whether the window still exists and still belongs to the application
    /// identified by `matcher` (window ids may be reused by the system).
    fn is_valid(&mut self, window: Self::Window, matcher: &str) -> bool;

    /// Whether the window is currently visible (not hidden / minimized).
    fn is_shown(&mut self, window: Self::Window) -> Result<bool>;

    /// Whether the window currently has keyboard focus.
    fn is_active(&mut self, window: Self::Window) -> Result<bool>;

    fn monitors(&mut self) -> Result<Vec<Monitor>>;

    fn cursor_position(&mut self) -> Result<(i32, i32)>;

    /// Center of the window, used to find the screen it is on.
    fn window_center(&mut self, window: Self::Window) -> Result<(i32, i32)>;

    fn show(&mut self, window: Self::Window, params: ShowParams) -> Result<()>;

    fn hide(&mut self, window: Self::Window) -> Result<()>;

    /// Undo everything anyquake changed (taskbar visibility, always on
    /// top...) so the window behaves normally again.
    fn release(&mut self, window: Self::Window);

    fn window_to_id(&self, window: Self::Window) -> String;

    fn window_from_id(&self, id: &str) -> Option<Self::Window>;
}

struct PendingLaunch {
    since: Instant,
    known: HashSet<String>,
}

/// Runtime state of one configured application.
struct AppState<W> {
    matcher: String,
    reuse_existing: bool,
    placement: Placement,
    managed: Option<W>,
    pending: Option<PendingLaunch>,
}

pub struct Controller<O: WindowOps> {
    ops: O,
    events: Sender<Event>,
    apps: Vec<AppState<O::Window>>,
    last_monitors: Option<Vec<Monitor>>,
}

impl<O: WindowOps> Controller<O> {
    pub fn new(ops: O, events: Sender<Event>) -> Self {
        Self {
            ops,
            events,
            apps: Vec::new(),
            last_monitors: None,
        }
    }

    fn set_managed(&mut self, app: usize, window: O::Window) {
        if self.apps[app].managed != Some(window) {
            self.apps[app].managed = Some(window);
            let id = self.ops.window_to_id(window);
            let _ = self.events.send(Event::Managed(app, id));
        }
    }

    /// Tells the daemon what toggling `app` did, for the log.
    fn report(&self, app: usize, action: &str) {
        let _ = self.events.send(Event::Toggled(app, action.to_owned()));
    }

    /// Whether another application than `app` manages `window`.
    fn managed_by_other(&self, app: usize, window: O::Window) -> bool {
        self.apps
            .iter()
            .enumerate()
            .any(|(i, other)| i != app && other.managed == Some(window))
    }

    /// The managed window of `app`, adopting an existing one if allowed.
    fn current_window(&mut self, app: usize) -> Result<Option<O::Window>> {
        if let Some(window) = self.apps[app].managed {
            if self.ops.is_valid(window, &self.apps[app].matcher) {
                return Ok(Some(window));
            }
            self.apps[app].managed = None;
        }
        if !self.apps[app].reuse_existing {
            return Ok(None);
        }
        let found = self
            .ops
            .find_all(&self.apps[app].matcher)?
            .into_iter()
            .find(|&w| !self.managed_by_other(app, w));
        if let Some(window) = found {
            self.set_managed(app, window);
        }
        Ok(found)
    }

    /// Monitor the window is currently on (by its center).
    fn monitor_of(&mut self, window: O::Window, monitors: &[Monitor]) -> Option<Monitor> {
        let (x, y) = self.ops.window_center(window).ok()?;
        monitors.iter().find(|m| m.full.contains(x, y)).cloned()
    }

    fn show(&mut self, app: usize, window: O::Window) -> Result<()> {
        let monitors = self.ops.monitors()?;
        let placement = &self.apps[app].placement;
        let monitor = select_monitor(&mut self.ops, placement, &monitors, None)?;
        let params = params_for(placement, &monitor, true);
        log::debug!(
            "showing {window:?} on {} at {:?}",
            monitor.name,
            params.rect
        );
        self.ops.show(window, params)
    }

    fn start_launch(&mut self, app: usize) -> Result<()> {
        let known = self
            .ops
            .find_all(&self.apps[app].matcher)?
            .into_iter()
            .map(|w| self.ops.window_to_id(w))
            .collect();
        self.apps[app].pending = Some(PendingLaunch {
            since: Instant::now(),
            known,
        });
        let _ = self.events.send(Event::Launch(app));
        Ok(())
    }

    /// Looks for the window of an application started by
    /// [`Self::start_launch`].
    fn check_launch(&mut self, app: usize) -> Result<()> {
        let Some(pending) = &self.apps[app].pending else {
            return Ok(());
        };
        if pending.since.elapsed() > LAUNCH_TIMEOUT {
            // Reported by the daemon, which watches launches of all backends.
            self.apps[app].pending = None;
            return Ok(());
        }
        let candidates = self.ops.find_all(&self.apps[app].matcher)?;
        let new = candidates.into_iter().find(|&w| {
            !pending.known.contains(&self.ops.window_to_id(w)) && !self.managed_by_other(app, w)
        });
        if let Some(window) = new {
            self.apps[app].pending = None;
            self.set_managed(app, window);
            self.show(app, window)?;
        }
        Ok(())
    }

    /// Re-applies the placement of `app`'s window if it is shown.
    fn replace_window(&mut self, app: usize) -> Result<()> {
        let Some(window) = self.apps[app].managed else {
            return Ok(());
        };
        if !self.ops.is_valid(window, &self.apps[app].matcher) || !self.ops.is_shown(window)? {
            return Ok(());
        }
        let monitors = self.ops.monitors()?;
        let current = self.monitor_of(window, &monitors).map(|m| m.name);
        let placement = &self.apps[app].placement;
        let monitor = select_monitor(&mut self.ops, placement, &monitors, current.as_deref())?;
        let params = params_for(placement, &monitor, false);
        self.ops.show(window, params)
    }
}

fn select_monitor<O: WindowOps>(
    ops: &mut O,
    placement: &Placement,
    monitors: &[Monitor],
    current: Option<&str>,
) -> Result<Monitor> {
    anyhow::ensure!(!monitors.is_empty(), "no monitors found");
    let by_name = |name: &str| {
        monitors
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .cloned()
    };
    if let Some(monitor) = current.and_then(by_name) {
        return Ok(monitor);
    }
    let primary = || {
        monitors
            .iter()
            .find(|m| m.primary)
            .unwrap_or(&monitors[0])
            .clone()
    };
    let at_cursor = |ops: &mut O| -> Result<Monitor> {
        let (x, y) = ops.cursor_position()?;
        Ok(monitors
            .iter()
            .find(|m| m.full.contains(x, y))
            .cloned()
            .unwrap_or_else(primary))
    };
    match &placement.monitor {
        MonitorChoice::Cursor => at_cursor(ops),
        MonitorChoice::Primary => Ok(primary()),
        MonitorChoice::Named(name) => match by_name(name) {
            Some(monitor) => Ok(monitor),
            None => at_cursor(ops),
        },
    }
}

fn params_for(p: &Placement, monitor: &Monitor, activate: bool) -> ShowParams {
    let fullscreen = p.anchor == Anchor::Fullscreen;
    let area = if p.respect_panels && !fullscreen { monitor.work } else { monitor.full };
    ShowParams {
        rect: geometry::place(area, p.anchor, p.width, p.height),
        fullscreen,
        always_on_top: p.always_on_top,
        activate,
    }
}

/// Combines several errors into one.
fn combine(errors: Vec<anyhow::Error>) -> Result<()> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().unwrap()),
        _ => {
            let messages: Vec<_> = errors.iter().map(|e| format!("{e:#}")).collect();
            anyhow::bail!("{}", messages.join("; "))
        }
    }
}

impl<O: WindowOps> Backend for Controller<O> {
    fn name(&self) -> &'static str {
        self.ops.name()
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        let mut old = std::mem::take(&mut self.apps).into_iter();
        for (i, app) in config.apps.iter().enumerate() {
            let matcher = app.target.effective_match();
            let (managed, pending) = match old.next() {
                Some(prev) if prev.matcher == matcher => (prev.managed, prev.pending),
                prev => {
                    // A different application was configured at this position
                    // (or it is new): give the old window back.
                    if let Some(window) = prev.and_then(|p| p.managed) {
                        self.ops.release(window);
                    }
                    let saved = crate::state::load_managed_window(self.ops.name(), i, &matcher);
                    (saved.and_then(|id| self.ops.window_from_id(&id)), None)
                }
            };
            self.apps.push(AppState {
                matcher,
                reuse_existing: app.target.reuse_existing,
                placement: app.placement.clone(),
                managed,
                pending,
            });
        }
        // Applications that were removed.
        for prev in old {
            if let Some(window) = prev.managed
                && self.ops.is_valid(window, &prev.matcher)
            {
                self.ops.release(window);
            }
        }
        self.ops.configure(config)
    }

    fn toggle(&mut self, app: usize) -> Result<()> {
        anyhow::ensure!(app < self.apps.len(), "unknown application {}", app + 1);
        if self.apps[app].pending.is_some() {
            self.report(app, "still waiting for the application to start");
            return Ok(());
        }
        let Some(window) = self.current_window(app)? else {
            self.report(app, "no window, launching");
            return self.start_launch(app);
        };

        if self.ops.is_shown(window)? && self.ops.is_active(window)? {
            // Visible and focused: hide it, unless it is on another screen
            // than the one it should appear on - then move it over.
            let monitors = self.ops.monitors()?;
            let placement = &self.apps[app].placement;
            let target = select_monitor(&mut self.ops, placement, &monitors, None)?;
            let current = self.monitor_of(window, &monitors);
            if current.is_none_or(|m| m.name == target.name) {
                self.report(app, "hiding");
                return self.ops.hide(window);
            }
        }
        self.report(app, "showing");
        self.show(app, window)
    }

    fn screens_changed(&mut self) -> Result<()> {
        let errors = (0..self.apps.len())
            .filter_map(|app| self.replace_window(app).err())
            .collect();
        combine(errors)
    }

    fn poll_interval(&self) -> Option<Duration> {
        if self.apps.iter().any(|app| app.pending.is_some()) {
            Some(LAUNCH_POLL)
        } else if O::POLL_SCREENS {
            Some(SCREEN_POLL)
        } else {
            None
        }
    }

    fn tick(&mut self) -> Result<()> {
        self.ops.poll();
        if self.apps.iter().any(|app| app.pending.is_some()) {
            let errors = (0..self.apps.len())
                .filter_map(|app| self.check_launch(app).err())
                .collect();
            return combine(errors);
        }
        if O::POLL_SCREENS {
            let monitors = Some(self.ops.monitors()?);
            if self.last_monitors.is_some() && self.last_monitors != monitors {
                log::info!("screen layout changed");
                self.screens_changed()?;
            }
            self.last_monitors = monitors;
        }
        Ok(())
    }

    fn release(&mut self) {
        for app in &self.apps {
            if let Some(window) = app.managed
                && self.ops.is_valid(window, &app.matcher)
            {
                self.ops.release(window);
            }
        }
    }
}
