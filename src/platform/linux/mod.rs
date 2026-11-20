//! Linux: KWin (KDE Plasma, Wayland and X11), GNOME Shell (Wayland) or
//! plain X11 backend, StatusNotifierItem tray icon, channel based main loop.

mod gnome;
mod kwin;
mod x11;

use crate::config::App;
use crate::controller::Controller;
use crate::daemon::{Backend, Daemon, Event};
use crate::{icon, ipc};
use anyhow::{Context, Result, bail};
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;
use zbus::blocking::{Connection, connection};
use zbus::interface;

/// Name and object of the daemon on the session bus. The compositor side
/// (KWin script, GNOME Shell extension) calls into it.
const DBUS_NAME: &str = "de.anyquake.Daemon";
const DBUS_PATH: &str = "/de/anyquake/Daemon";

struct DaemonCallbacks {
    events: Sender<Event>,
}

#[interface(name = "de.anyquake.Daemon")]
impl DaemonCallbacks {
    /// No window was found for the application with index `app` (a decimal
    /// number: KWin scripts can't send D-Bus integers reliably); start it.
    fn launch_app(&self, app: String) {
        if let Ok(app) = app.parse() {
            let _ = self.events.send(Event::Launch(app));
        }
    }

    /// Reports the compositor's id of the window managed for the
    /// application with index `app`.
    fn set_managed_app(&self, app: String, id: String) {
        if let Ok(app) = app.parse() {
            let _ = self.events.send(Event::Managed(app, id));
        }
    }

    /// Reports what toggling application `app` did (for the log). Hotkeys
    /// are handled by the compositor, so this is the only way to learn
    /// about them.
    fn app_toggled(&self, app: String, action: String) {
        if let Ok(app) = app.parse() {
            let _ = self.events.send(Event::Toggled(app, action));
        }
    }

    /// Reports the connected screens as a JSON array of
    /// [`crate::daemon::ScreenInfo`] (KWin scripts can only pass simple
    /// values).
    fn set_screens(&self, screens: String) {
        match serde_json::from_str(&screens) {
            Ok(screens) => {
                let _ = self.events.send(Event::ScreensReported(screens));
            }
            Err(e) => log::warn!("unexpected screen list '{screens}': {e}"),
        }
    }

    /// [`Self::launch_app`] for the first application. Used by the GNOME
    /// Shell extension of AnyQuake 0.5 until the next login.
    fn launch(&self) {
        let _ = self.events.send(Event::Launch(0));
    }

    /// [`Self::set_managed_app`] for the first application (see
    /// [`Self::launch`]).
    fn set_managed(&self, id: String) {
        let _ = self.events.send(Event::Managed(0, id));
    }

    /// The GNOME Shell extension was (re-)enabled, e.g. after unlocking the
    /// screen, and needs the configuration again.
    fn extension_ready(&self) {
        let _ = self.events.send(Event::Reload(None));
    }
}

/// Connects to the session bus and serves [`DaemonCallbacks`].
fn session_connection(events: Sender<Event>) -> Result<Connection> {
    connection::Builder::session()?
        .name(DBUS_NAME)?
        .serve_at(DBUS_PATH, DaemonCallbacks { events })?
        .build()
        .context("connecting to the D-Bus session bus")
}

/// Configuration of one application as passed to the KWin script and the
/// GNOME Shell extension. They add their own keys (hotkey etc.).
fn app_json(backend: &str, index: usize, app: &App) -> serde_json::Value {
    let p = &app.placement;
    let matcher = app.target.effective_match();
    serde_json::json!({
        "index": index.to_string(),
        "match": matcher,
        "reuseExisting": app.target.reuse_existing,
        "managedId": crate::state::load_managed_window(backend, index, &matcher).unwrap_or_default(),
        "monitor": String::from(p.monitor.clone()),
        "anchor": p.anchor.id(),
        "width": p.width,
        "height": p.height,
        "respectPanels": p.respect_panels,
        "alwaysOnTop": p.always_on_top,
    })
}

/// Picks the backend for the current session. `ANYQUAKE_BACKEND=kwin|gnome|x11`
/// overrides the automatic detection.
fn create_backend(events: Sender<Event>) -> Result<Box<dyn Backend>> {
    let forced = std::env::var("ANYQUAKE_BACKEND")
        .unwrap_or_default()
        .to_lowercase();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let is_desktop = |name: &str| desktop.split(':').any(|d| d.eq_ignore_ascii_case(name));
    let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");

    match forced.as_str() {
        "kwin" => return Ok(Box::new(kwin::KWin::new(events)?)),
        "gnome" => return Ok(Box::new(gnome::Gnome::new(events)?)),
        "x11" => {
            return Ok(Box::new(Controller::new(
                x11::X11::new(events.clone())?,
                events,
            )));
        }
        "" => {}
        other => bail!("unknown ANYQUAKE_BACKEND '{other}' (expected 'kwin', 'gnome' or 'x11')"),
    }

    if is_desktop("kde") {
        // KWin works for Wayland and X11 sessions alike and is the more
        // robust choice on Plasma.
        return Ok(Box::new(kwin::KWin::new(events)?));
    }
    if is_wayland && is_desktop("gnome") {
        return Ok(Box::new(gnome::Gnome::new(events)?));
    }
    if is_wayland {
        bail!(
            "the Wayland compositor '{desktop}' is not supported: Wayland does not allow \
             applications to move other windows or grab global keys. Supported are KDE Plasma \
             (Wayland or X11), GNOME (Wayland) and X11 sessions with any EWMH window manager. \
             ANYQUAKE_BACKEND=x11 can be used to manage XWayland windows only."
        );
    }
    Ok(Box::new(Controller::new(
        x11::X11::new(events.clone())?,
        events,
    )))
}

struct Tray {
    events: Sender<Event>,
    /// Names of the configured applications.
    apps: Vec<String>,
}

impl Tray {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        crate::config::APP_NAME.into()
    }

    fn title(&self) -> String {
        "AnyQuake".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 64]
            .into_iter()
            .map(|size| {
                // SNI wants ARGB32 in network byte order.
                let mut data = icon::rgba(size, icon::Variant::Colored);
                for px in data.as_chunks_mut::<4>().0 {
                    px.rotate_right(1);
                }
                ksni::Icon {
                    width: size as i32,
                    height: size as i32,
                    data,
                }
            })
            .collect()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::OpenSettings);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let toggles = self.apps.iter().enumerate().map(|(i, name)| {
            StandardItem {
                label: format!("Toggle {name}"),
                activate: Box::new(move |tray: &mut Self| tray.send(Event::Toggle(i))),
                ..Default::default()
            }
            .into()
        });
        let mut items: Vec<_> = toggles.collect();
        if !items.is_empty() {
            items.push(ksni::MenuItem::Separator);
        }
        items.extend([
            StandardItem {
                label: "Settings…".into(),
                icon_name: "configure".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Event::OpenSettings)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }
}

pub fn run_daemon(open_settings: bool) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    ipc::serve(ipc::to_channel(tx.clone()))?;

    let backend = create_backend(tx.clone())?;
    let mut daemon = Daemon::new(backend);

    // Keep the handle alive for the lifetime of the daemon.
    let tray = Tray {
        events: tx.clone(),
        apps: Vec::new(),
    };
    let tray = match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::warn!("no system tray available ({e}); use `anyquake settings` instead");
            None
        }
    };

    let ctrlc_tx = tx.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        let _ = ctrlc_tx.send(Event::Quit);
    }) {
        log::warn!("installing signal handler: {e}");
    }

    daemon.start(open_settings);

    let mut labels = Vec::new();
    loop {
        // Keep the tray menu in sync with the configured applications.
        if let Some(tray) = &tray
            && labels != daemon.app_labels()
        {
            labels = daemon.app_labels();
            let apps = labels.clone();
            tray.update(move |tray| tray.apps = apps);
        }
        let timeout = daemon.poll_interval().unwrap_or(Duration::from_secs(3600));
        match rx.recv_timeout(timeout) {
            Ok(event) => {
                if !daemon.handle(event) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => daemon.tick(),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    log::info!("exiting");
    Ok(())
}
