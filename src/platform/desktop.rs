//! Daemon main loop for Windows and macOS: a winit event loop (needed for
//! the tray icon and global hotkeys) that dispatches into [`Daemon`].

use crate::controller::Controller;
use crate::daemon::{Daemon, Event};
use crate::hotkey::Hotkey;
use crate::{icon, ipc};
use anyhow::{Context, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

enum UserEvent {
    Daemon(Event),
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Hotkey(GlobalHotKeyEvent),
}

/// Maps the ids of registered hotkeys to the index of their application.
/// Shared between the hotkey registration and the event loop.
pub type HotkeyApps = Arc<Mutex<HashMap<u32, usize>>>;

/// One application's hotkey.
struct Entry {
    hotkey: Hotkey,
    /// The native hotkey last tried to register and whether that worked.
    attempted: Option<(HotKey, bool)>,
}

/// Registers the configured hotkeys with the OS. Owned by the window backend
/// so registration errors are reported back to the settings window.
pub struct HotkeyRegistration {
    manager: GlobalHotKeyManager,
    /// One entry per application, in the order of the configuration.
    entries: Vec<Entry>,
    apps: HotkeyApps,
}

impl HotkeyRegistration {
    pub fn new(apps: HotkeyApps) -> Result<Self> {
        Ok(Self {
            manager: GlobalHotKeyManager::new().context("initializing global hotkeys")?,
            entries: Vec::new(),
            apps,
        })
    }

    /// Registers the hotkeys of all applications, replacing the previous
    /// ones. Hotkeys that stay the same are kept registered.
    pub fn set(&mut self, hotkeys: &[&Hotkey]) -> Result<()> {
        let old = std::mem::take(&mut self.entries);
        let wanted: Vec<_> = hotkeys.iter().map(|h| native(h, true)).collect();
        for (old, _) in old.iter().filter_map(|e| e.attempted.filter(|(_, ok)| *ok)) {
            if !wanted.iter().any(|w| w.as_ref().is_ok_and(|w| *w == old)) {
                let _ = self.manager.unregister(old);
            }
        }
        let mut errors = Vec::new();
        for (hotkey, native) in hotkeys.iter().zip(wanted) {
            let attempted = match native {
                Err(e) => {
                    errors.push(format!("{e:#}"));
                    None
                }
                Ok(new)
                    if self
                        .entries
                        .iter()
                        .any(|e| e.attempted == Some((new, true))) =>
                {
                    errors.push(format!("{hotkey} is used by more than one application"));
                    Some((new, false))
                }
                Ok(new) if old.iter().any(|e| e.attempted == Some((new, true))) => {
                    Some((new, true))
                }
                Ok(new) => {
                    let result = self.register(hotkey, new);
                    if let Err(e) = &result {
                        errors.push(format!("{e:#}"));
                    }
                    Some((new, result.is_ok()))
                }
            };
            self.entries.push(Entry {
                hotkey: (*hotkey).clone(),
                attempted,
            });
        }
        self.update_apps();
        if !errors.is_empty() {
            anyhow::bail!("{}", errors.join("; "));
        }
        Ok(())
    }

    /// Registers the hotkeys again whose physical key changed (keyboard
    /// layout switched). Called periodically.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn refresh(&mut self) {
        let mut changed = false;
        for i in 0..self.entries.len() {
            let hotkey = self.entries[i].hotkey.clone();
            let Ok(new) = native(&hotkey, false) else {
                continue;
            };
            // Don't retry (and warn) every time if the same key failed before.
            if self.entries[i].attempted.is_some_and(|(old, _)| old == new) {
                continue;
            }
            log::info!("keyboard layout changed, registering {hotkey} again");
            if let Some((old, true)) = self.entries[i].attempted.take() {
                let _ = self.manager.unregister(old);
            }
            let result = self.register(&hotkey, new);
            self.entries[i].attempted = Some((new, result.is_ok()));
            if let Err(e) = result {
                log::warn!("{e:#}");
            }
            changed = true;
        }
        if changed {
            self.update_apps();
        }
    }

    fn register(&mut self, hotkey: &Hotkey, new: HotKey) -> Result<()> {
        self.manager.register(new).with_context(|| {
            format!("could not register {hotkey}, it is probably used by another application")
        })?;
        log::info!("registered hotkey {hotkey} ({})", new.key);
        Ok(())
    }

    fn update_apps(&self) {
        let mut apps = self.apps.lock().unwrap();
        apps.clear();
        for (app, entry) in self.entries.iter().enumerate() {
            if let Some((hotkey, true)) = entry.attempted {
                apps.insert(hotkey.id(), app);
            }
        }
    }
}

/// The native hotkey for `hotkey`. On macOS the physical key is chosen by
/// the current keyboard layout (see [`super::macos_layout`]).
fn native(hotkey: &Hotkey, warn: bool) -> Result<HotKey> {
    let mut mods = Modifiers::empty();
    mods.set(Modifiers::CONTROL, hotkey.ctrl);
    mods.set(Modifiers::ALT, hotkey.alt);
    mods.set(Modifiers::SHIFT, hotkey.shift);
    mods.set(Modifiers::META, hotkey.super_key);
    let code = Code::from_str(hotkey.key.code)
        .map_err(|_| anyhow::anyhow!("unsupported key {}", hotkey.key.name))?;
    #[cfg(target_os = "macos")]
    let code = super::macos_layout::resolve(hotkey.key, code).unwrap_or_else(|| {
        if warn {
            log::warn!(
                "no key types '{}' without modifiers in the current keyboard layout, using the \
                 key at its position on a US keyboard",
                hotkey.key.name
            );
        }
        code
    });
    #[cfg(not(target_os = "macos"))]
    let _ = warn;
    Ok(HotKey::new(Some(mods), code))
}

struct MenuIds {
    /// Toggle items, one per application.
    toggles: Vec<MenuId>,
    settings: MenuId,
    quit: MenuId,
}

struct App {
    daemon: Daemon,
    open_settings: bool,
    tray: Option<TrayIcon>,
    menu_ids: Option<MenuIds>,
    /// Application names the tray menu was built for.
    menu_labels: Vec<String>,
    hotkey_apps: HotkeyApps,
}

/// The tray menu with a toggle item per application.
fn create_menu(labels: &[String]) -> Result<(Menu, MenuIds)> {
    let menu = Menu::new();
    let mut toggles = Vec::new();
    for label in labels {
        let item = MenuItem::new(format!("Toggle {label}"), true, None);
        menu.append(&item)?;
        toggles.push(item.id().clone());
    }
    if !labels.is_empty() {
        menu.append(&PredefinedMenuItem::separator())?;
    }
    let settings = MenuItem::new("Settings…", true, None);
    let quit = MenuItem::new("Quit", true, None);
    menu.append_items(&[&settings, &PredefinedMenuItem::separator(), &quit])?;
    let ids = MenuIds {
        toggles,
        settings: settings.id().clone(),
        quit: quit.id().clone(),
    };
    Ok((menu, ids))
}

impl App {
    /// Rebuilds the tray menu if the configured applications changed.
    fn update_menu(&mut self) {
        let labels = self.daemon.app_labels();
        let Some(tray) = &self.tray else { return };
        if labels == self.menu_labels {
            return;
        }
        match create_menu(&labels) {
            Ok((menu, ids)) => {
                tray.set_menu(Some(Box::new(menu)));
                self.menu_ids = Some(ids);
                self.menu_labels = labels;
            }
            Err(e) => log::warn!("updating tray menu: {e:#}"),
        }
    }

    fn create_tray(&mut self) -> Result<()> {
        let (menu, ids) = create_menu(&self.menu_labels)?;
        // On macOS the menu bar icon is a white template image, which the
        // system tints to match the other menu bar icons.
        #[cfg(target_os = "macos")]
        let builder = {
            let size = 44;
            let icon =
                tray_icon::Icon::from_rgba(icon::rgba(size, icon::Variant::MenuBar), size, size)?;
            TrayIconBuilder::new().with_icon_templated(icon)
        };
        #[cfg(not(target_os = "macos"))]
        let builder = {
            let size = 64;
            let icon =
                tray_icon::Icon::from_rgba(icon::rgba(size, icon::Variant::Colored), size, size)?;
            TrayIconBuilder::new().with_icon(icon)
        };
        let tray = builder
            .with_tooltip("AnyQuake")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()?;
        self.menu_ids = Some(ids);
        self.tray = Some(tray);
        Ok(())
    }

    fn dispatch(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        if !self.daemon.handle(event) {
            self.tray = None;
            event_loop.exit();
            return;
        }
        self.update_menu();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            // The tray icon must be created once the event loop runs (macOS).
            StartCause::Init => {
                if let Err(e) = self.create_tray() {
                    log::warn!("could not create tray icon: {e:#}");
                }
                self.daemon.start(self.open_settings);
                self.update_menu();
            }
            StartCause::ResumeTimeReached { .. } => self.daemon.tick(),
            _ => {}
        }
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let event = match event {
            UserEvent::Daemon(event) => Some(event),
            UserEvent::Hotkey(e) if e.state == HotKeyState::Pressed => {
                let app = self.hotkey_apps.lock().unwrap().get(&e.id).copied();
                app.map(Event::Toggle)
            }
            UserEvent::Hotkey(_) => None,
            UserEvent::Menu(e) => self.menu_ids.as_ref().and_then(|ids| {
                if let Some(app) = ids.toggles.iter().position(|id| *id == e.id) {
                    Some(Event::Toggle(app))
                } else if e.id == ids.settings {
                    Some(Event::OpenSettings)
                } else if e.id == ids.quit {
                    Some(Event::Quit)
                } else {
                    None
                }
            }),
            UserEvent::Tray(TrayIconEvent::DoubleClick { .. }) => Some(Event::OpenSettings),
            UserEvent::Tray(_) => None,
        };
        if let Some(event) = event {
            self.dispatch(event_loop, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(match self.daemon.poll_interval() {
            Some(delay) => ControlFlow::WaitUntil(Instant::now() + delay),
            None => ControlFlow::Wait,
        });
    }
}

pub fn run_daemon(open_settings: bool) -> Result<()> {
    #[allow(unused_mut)]
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // No Dock icon, only the menu bar item.
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let event_loop = builder.build().context("creating event loop")?;
    let proxy = event_loop.create_proxy();

    // Events from IPC, signal handler and backend go through a channel and
    // are forwarded into the event loop.
    let (tx, rx) = mpsc::channel::<Event>();
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            for event in rx {
                if proxy.send_event(UserEvent::Daemon(event)).is_err() {
                    break;
                }
            }
        });
    }
    ipc::serve(ipc::to_channel(tx.clone()))?;
    let ctrlc_tx = tx.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        let _ = ctrlc_tx.send(Event::Quit);
    }) {
        log::warn!("installing signal handler: {e}");
    }

    MenuEvent::set_event_handler(Some(forward(&proxy, UserEvent::Menu)));
    TrayIconEvent::set_event_handler(Some(forward(&proxy, UserEvent::Tray)));
    GlobalHotKeyEvent::set_event_handler(Some(forward(&proxy, UserEvent::Hotkey)));

    let hotkey_apps = HotkeyApps::default();
    #[cfg(target_os = "windows")]
    let ops = super::windows::Win32::new(hotkey_apps.clone())?;
    #[cfg(target_os = "macos")]
    let ops = super::macos::MacOS::new(hotkey_apps.clone())?;

    let backend = Box::new(Controller::new(ops, tx));
    let mut app = App {
        daemon: Daemon::new(backend),
        open_settings,
        tray: None,
        menu_ids: None,
        menu_labels: Vec::new(),
        hotkey_apps,
    };
    event_loop.run_app(&mut app).context("running event loop")?;
    Ok(())
}

/// Event handler forwarding a library event into the winit event loop.
fn forward<E: Send + 'static>(
    proxy: &EventLoopProxy<UserEvent>,
    wrap: fn(E) -> UserEvent,
) -> impl Fn(E) + Send + Sync + 'static {
    let proxy = Mutex::new(proxy.clone());
    move |event| {
        let _ = proxy.lock().unwrap().send_event(wrap(event));
    }
}
