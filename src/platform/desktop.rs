//! Daemon main loop for Windows and macOS: a winit event loop (needed for
//! the tray icon and global hotkeys) that dispatches into [`Daemon`].

use crate::controller::Controller;
use crate::daemon::{Daemon, Event};
use crate::hotkey::Hotkey;
use crate::{icon, ipc};
use anyhow::{Context, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::str::FromStr;
use std::sync::{Mutex, mpsc};
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

/// Registers the configured hotkey with the OS. Owned by the window backend
/// so registration errors are reported back to the settings window.
pub struct HotkeyRegistration {
    manager: GlobalHotKeyManager,
    current: Option<HotKey>,
}

impl HotkeyRegistration {
    pub fn new() -> Result<Self> {
        Ok(Self {
            manager: GlobalHotKeyManager::new().context("initializing global hotkeys")?,
            current: None,
        })
    }

    pub fn set(&mut self, hotkey: &Hotkey) -> Result<()> {
        let mut mods = Modifiers::empty();
        mods.set(Modifiers::CONTROL, hotkey.ctrl);
        mods.set(Modifiers::ALT, hotkey.alt);
        mods.set(Modifiers::SHIFT, hotkey.shift);
        mods.set(Modifiers::META, hotkey.super_key);
        let code = Code::from_str(hotkey.key.code)
            .map_err(|_| anyhow::anyhow!("unsupported key {}", hotkey.key.name))?;
        let new = HotKey::new(Some(mods), code);
        if self.current == Some(new) {
            return Ok(());
        }
        if let Some(old) = self.current.take() {
            let _ = self.manager.unregister(old);
        }
        self.manager.register(new).with_context(|| {
            format!("could not register {hotkey}, it is probably used by another application")
        })?;
        self.current = Some(new);
        log::info!("registered hotkey {hotkey}");
        Ok(())
    }
}

struct MenuIds {
    toggle: MenuId,
    settings: MenuId,
    quit: MenuId,
}

struct App {
    daemon: Daemon,
    open_settings: bool,
    tray: Option<TrayIcon>,
    menu_ids: Option<MenuIds>,
}

impl App {
    fn create_tray(&mut self) -> Result<()> {
        let toggle = MenuItem::new("Toggle window", true, None);
        let settings = MenuItem::new("Settings…", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&toggle, &settings, &PredefinedMenuItem::separator(), &quit])?;
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
        self.menu_ids = Some(MenuIds {
            toggle: toggle.id().clone(),
            settings: settings.id().clone(),
            quit: quit.id().clone(),
        });
        self.tray = Some(tray);
        Ok(())
    }

    fn dispatch(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        if !self.daemon.handle(event) {
            self.tray = None;
            event_loop.exit();
        }
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
            UserEvent::Hotkey(e) => (e.state == HotKeyState::Pressed).then_some(Event::Toggle),
            UserEvent::Menu(e) => self.menu_ids.as_ref().and_then(|ids| {
                if e.id == ids.toggle {
                    Some(Event::Toggle)
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

    #[cfg(target_os = "windows")]
    let ops = super::windows::Win32::new()?;
    #[cfg(target_os = "macos")]
    let ops = super::macos::MacOS::new()?;

    let backend = Box::new(Controller::new(ops, tx));
    let mut app = App {
        daemon: Daemon::new(backend),
        open_settings,
        tray: None,
        menu_ids: None,
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
