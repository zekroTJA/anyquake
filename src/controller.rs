//! Generic toggle logic on top of a small set of window system primitives.
//! Used by the X11, Windows and macOS backends (KWin runs the equivalent
//! logic inside a KWin script).

use crate::config::{Config, MonitorChoice, Placement};
use crate::daemon::{Backend, Event};
use crate::geometry::{self, Anchor, Rect};
use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// How long to wait for the window of a freshly started application.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
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

    /// Called whenever the configuration changes (e.g. to grab the hotkey).
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

pub struct Controller<O: WindowOps> {
    ops: O,
    events: Sender<Event>,
    matcher: String,
    reuse_existing: bool,
    placement: Placement,
    managed: Option<O::Window>,
    pending: Option<PendingLaunch>,
    last_monitors: Option<Vec<Monitor>>,
}

impl<O: WindowOps> Controller<O> {
    pub fn new(ops: O, events: Sender<Event>) -> Self {
        let managed =
            crate::state::load_managed_window(ops.name()).and_then(|id| ops.window_from_id(&id));
        Self {
            ops,
            events,
            matcher: String::new(),
            reuse_existing: true,
            placement: Placement::default(),
            managed,
            pending: None,
            last_monitors: None,
        }
    }

    fn set_managed(&mut self, window: O::Window) {
        if self.managed != Some(window) {
            self.managed = Some(window);
            let _ = self
                .events
                .send(Event::Managed(self.ops.window_to_id(window)));
        }
    }

    /// The managed window, adopting an existing one if allowed.
    fn current_window(&mut self) -> Result<Option<O::Window>> {
        if let Some(window) = self.managed {
            if self.ops.is_valid(window, &self.matcher) {
                return Ok(Some(window));
            }
            self.managed = None;
        }
        if !self.reuse_existing {
            return Ok(None);
        }
        let found = self.ops.find_all(&self.matcher)?.into_iter().next();
        if let Some(window) = found {
            self.set_managed(window);
        }
        Ok(found)
    }

    fn select_monitor(&mut self, monitors: &[Monitor], current: Option<&str>) -> Result<Monitor> {
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
        match &self.placement.monitor {
            MonitorChoice::Cursor => at_cursor(&mut self.ops),
            MonitorChoice::Primary => Ok(primary()),
            MonitorChoice::Named(name) => match by_name(name) {
                Some(monitor) => Ok(monitor),
                None => at_cursor(&mut self.ops),
            },
        }
    }

    fn params_for(&self, monitor: &Monitor, activate: bool) -> ShowParams {
        let p = &self.placement;
        let fullscreen = p.anchor == Anchor::Fullscreen;
        let area = if p.respect_panels && !fullscreen { monitor.work } else { monitor.full };
        ShowParams {
            rect: geometry::place(area, p.anchor, p.width, p.height),
            fullscreen,
            always_on_top: p.always_on_top,
            activate,
        }
    }

    /// Monitor the window is currently on (by its center).
    fn monitor_of(&mut self, window: O::Window, monitors: &[Monitor]) -> Option<Monitor> {
        let (x, y) = self.ops.window_center(window).ok()?;
        monitors.iter().find(|m| m.full.contains(x, y)).cloned()
    }

    fn show(&mut self, window: O::Window) -> Result<()> {
        let monitors = self.ops.monitors()?;
        let monitor = self.select_monitor(&monitors, None)?;
        let params = self.params_for(&monitor, true);
        log::debug!(
            "showing {window:?} on {} at {:?}",
            monitor.name,
            params.rect
        );
        self.ops.show(window, params)
    }

    fn start_launch(&mut self) -> Result<()> {
        let known = self
            .ops
            .find_all(&self.matcher)?
            .into_iter()
            .map(|w| self.ops.window_to_id(w))
            .collect();
        self.pending = Some(PendingLaunch {
            since: Instant::now(),
            known,
        });
        let _ = self.events.send(Event::Launch);
        Ok(())
    }
}

impl<O: WindowOps> Backend for Controller<O> {
    fn name(&self) -> &'static str {
        self.ops.name()
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        let matcher = config.target.effective_match();
        if matcher != self.matcher && !self.matcher.is_empty() {
            // A different application was configured.
            if let Some(window) = self.managed.take() {
                self.ops.release(window);
            }
        }
        self.matcher = matcher;
        self.reuse_existing = config.target.reuse_existing;
        self.placement = config.placement.clone();
        self.ops.configure(config)
    }

    fn toggle(&mut self) -> Result<()> {
        if self.pending.is_some() {
            log::debug!("still waiting for the application to start");
            return Ok(());
        }
        let Some(window) = self.current_window()? else {
            return self.start_launch();
        };

        if self.ops.is_shown(window)? && self.ops.is_active(window)? {
            // Visible and focused: hide it, unless it is on another screen
            // than the one it should appear on - then move it over.
            let monitors = self.ops.monitors()?;
            let target = self.select_monitor(&monitors, None)?;
            let current = self.monitor_of(window, &monitors);
            if current.is_none_or(|m| m.name == target.name) {
                return self.ops.hide(window);
            }
        }
        self.show(window)
    }

    fn screens_changed(&mut self) -> Result<()> {
        let Some(window) = self.managed else {
            return Ok(());
        };
        if !self.ops.is_valid(window, &self.matcher) || !self.ops.is_shown(window)? {
            return Ok(());
        }
        let monitors = self.ops.monitors()?;
        let current = self.monitor_of(window, &monitors).map(|m| m.name);
        let monitor = self.select_monitor(&monitors, current.as_deref())?;
        let params = self.params_for(&monitor, false);
        self.ops.show(window, params)
    }

    fn poll_interval(&self) -> Option<Duration> {
        if self.pending.is_some() {
            Some(LAUNCH_POLL)
        } else if O::POLL_SCREENS {
            Some(SCREEN_POLL)
        } else {
            None
        }
    }

    fn tick(&mut self) -> Result<()> {
        self.ops.poll();
        let Some(pending) = &self.pending else {
            if O::POLL_SCREENS {
                let monitors = Some(self.ops.monitors()?);
                if self.last_monitors.is_some() && self.last_monitors != monitors {
                    log::info!("screen layout changed");
                    self.screens_changed()?;
                }
                self.last_monitors = monitors;
            }
            return Ok(());
        };
        if pending.since.elapsed() > LAUNCH_TIMEOUT {
            self.pending = None;
            anyhow::bail!(
                "no window matching '{}' appeared after starting the application; check the \
                 'window match' setting",
                self.matcher
            );
        }
        let candidates = self.ops.find_all(&self.matcher)?;
        let new = candidates
            .into_iter()
            .find(|w| !pending.known.contains(&self.ops.window_to_id(*w)));
        if let Some(window) = new {
            self.pending = None;
            self.set_managed(window);
            self.show(window)?;
        }
        Ok(())
    }

    fn release(&mut self) {
        if let Some(window) = self.managed
            && self.ops.is_valid(window, &self.matcher)
        {
            self.ops.release(window);
        }
    }
}
