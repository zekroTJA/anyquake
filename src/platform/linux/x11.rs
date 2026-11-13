//! X11 backend for any EWMH compliant window manager.

use crate::config::Config;
use crate::controller::{Monitor, ShowParams, WindowOps};
use crate::daemon::Event;
use crate::geometry::Rect;
use crate::hotkey::Hotkey;
use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::properties::WmClass;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, GrabMode, Keycode, MapState,
    ModMask, Window,
};
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST,
        _NET_CURRENT_DESKTOP,
        _NET_FRAME_EXTENTS,
        _NET_MOVERESIZE_WINDOW,
        _NET_WM_DESKTOP,
        _NET_WM_STATE,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_HIDDEN,
        _NET_WM_STATE_MAXIMIZED_HORZ,
        _NET_WM_STATE_MAXIMIZED_VERT,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _NET_WORKAREA,
        WM_CHANGE_STATE,
    }
}

const STATE_REMOVE: u32 = 0;
const STATE_ADD: u32 = 1;
/// "Source indication" for EWMH requests: 2 = pager / direct user action,
/// which makes window managers honor the request without focus stealing
/// prevention.
const SOURCE_PAGER: u32 = 2;
const ICONIC_STATE: u32 = 3;
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

pub struct X11 {
    conn: Arc<RustConnection>,
    root: Window,
    atoms: Atoms,
    grabbed: Option<Keycode>,
}

impl X11 {
    pub fn new(events: Sender<Event>) -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None).context("connecting to the X server")?;
        let conn = Arc::new(conn);
        let root = conn.setup().roots[screen_num].root;
        let atoms = Atoms::new(&*conn)?.reply()?;

        if conn
            .extension_information(randr::X11_EXTENSION_NAME)?
            .is_some()
        {
            conn.randr_select_input(
                root,
                randr::NotifyMask::SCREEN_CHANGE
                    | randr::NotifyMask::OUTPUT_CHANGE
                    | randr::NotifyMask::CRTC_CHANGE,
            )?;
        }
        conn.flush()?;

        let event_conn = conn.clone();
        std::thread::Builder::new()
            .name("x11-events".into())
            .spawn(move || event_loop(&event_conn, &events))?;

        Ok(Self {
            conn,
            root,
            atoms,
            grabbed: None,
        })
    }

    fn property32(&self, window: Window, property: u32, ty: impl Into<u32>) -> Result<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, window, property, ty, 0, u32::MAX)?
            .reply()?;
        Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
    }

    fn client_message(&self, window: Window, ty: u32, data: [u32; 5]) -> Result<()> {
        let event = ClientMessageEvent::new(32, window, ty, data);
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?;
        Ok(())
    }

    fn set_state(&self, window: Window, action: u32, first: u32, second: u32) -> Result<()> {
        self.client_message(
            window,
            self.atoms._NET_WM_STATE,
            [action, first, second, SOURCE_PAGER, 0],
        )
    }

    fn states(&self, window: Window) -> Result<Vec<u32>> {
        self.property32(window, self.atoms._NET_WM_STATE, AtomEnum::ATOM)
    }

    fn class_matches(&self, window: Window, matcher: &str) -> bool {
        let Ok(Ok(Some(class))) = WmClass::get(&*self.conn, window).map(|c| c.reply()) else {
            return false;
        };
        [class.instance(), class.class()]
            .iter()
            .any(|v| String::from_utf8_lossy(v).to_lowercase() == matcher)
    }

    fn is_normal_window(&self, window: Window) -> bool {
        let types = self
            .property32(window, self.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM)
            .unwrap_or_default();
        types.is_empty()
            || types.contains(&self.atoms._NET_WM_WINDOW_TYPE_NORMAL)
            || types.contains(&self.atoms._NET_WM_WINDOW_TYPE_DIALOG)
    }

    fn current_desktop(&self) -> u32 {
        self.property32(
            self.root,
            self.atoms._NET_CURRENT_DESKTOP,
            AtomEnum::CARDINAL,
        )
        .ok()
        .and_then(|v| v.first().copied())
        .unwrap_or(0)
    }

    fn workarea(&self) -> Option<Rect> {
        let values = self
            .property32(self.root, self.atoms._NET_WORKAREA, AtomEnum::CARDINAL)
            .ok()?;
        let i = self.current_desktop() as usize * 4;
        let v = values.get(i..i + 4).or_else(|| values.get(0..4))?;
        Some(Rect::new(
            v[0] as i32,
            v[1] as i32,
            v[2] as i32,
            v[3] as i32,
        ))
    }

    fn keycode_for(&self, keysym: u32) -> Result<Keycode> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let mapping = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)?
            .reply()?;
        let per = mapping.keysyms_per_keycode as usize;
        mapping
            .keysyms
            .chunks(per)
            .position(|syms| syms.contains(&keysym))
            .map(|i| min + i as u8)
            .with_context(|| format!("no key on the keyboard produces keysym {keysym:#x}"))
    }

    fn grab(&mut self, hotkey: &Hotkey) -> Result<()> {
        if let Some(keycode) = self.grabbed.take() {
            self.conn.ungrab_key(keycode, self.root, ModMask::ANY)?;
        }
        let keycode = self.keycode_for(hotkey.key.keysym)?;
        let mut mods = ModMask::from(0u16);
        if hotkey.shift {
            mods |= ModMask::SHIFT;
        }
        if hotkey.ctrl {
            mods |= ModMask::CONTROL;
        }
        if hotkey.alt {
            mods |= ModMask::M1;
        }
        if hotkey.super_key {
            mods |= ModMask::M4;
        }
        // Grab with every combination of NumLock (M2) and CapsLock (LOCK),
        // otherwise the hotkey does not work while one of them is active.
        for extra in [
            0u16,
            ModMask::M2.into(),
            ModMask::LOCK.into(),
            (ModMask::M2 | ModMask::LOCK).into(),
        ] {
            let cookie = self.conn.grab_key(
                false,
                self.root,
                mods | ModMask::from(extra),
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?;
            if let Err(e) = cookie.check() {
                let _ = self.conn.ungrab_key(keycode, self.root, ModMask::ANY);
                bail!("could not grab {hotkey}, it is probably used by another application ({e})");
            }
        }
        self.conn.flush()?;
        self.grabbed = Some(keycode);
        log::info!("grabbed hotkey {hotkey}");
        Ok(())
    }

    fn absolute_geometry(&self, window: Window) -> Result<Rect> {
        let geo = self.conn.get_geometry(window)?.reply()?;
        let pos = self
            .conn
            .translate_coordinates(window, self.root, 0, 0)?
            .reply()?;
        Ok(Rect::new(
            pos.dst_x.into(),
            pos.dst_y.into(),
            geo.width.into(),
            geo.height.into(),
        ))
    }
}

fn event_loop(conn: &RustConnection, events: &Sender<Event>) {
    // Holding a key down makes X send Release+Press pairs with the same
    // timestamp. Those are filtered out so the window does not flicker.
    let mut last_release = None;
    loop {
        let event = match conn.wait_for_event() {
            Ok(event) => event,
            Err(e) => {
                log::error!("X11 connection lost: {e}");
                let _ = events.send(Event::Quit);
                return;
            }
        };
        match event {
            XEvent::KeyPress(e) => {
                if last_release != Some(e.time) {
                    let _ = events.send(Event::Toggle);
                }
            }
            XEvent::KeyRelease(e) => last_release = Some(e.time),
            XEvent::RandrScreenChangeNotify(_) | XEvent::RandrNotify(_) => {
                let _ = events.send(Event::ScreensChanged);
            }
            _ => {}
        }
    }
}

impl WindowOps for X11 {
    type Window = Window;

    fn name(&self) -> &'static str {
        "X11"
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        self.grab(&config.general.hotkey)
    }

    fn find_all(&mut self, matcher: &str) -> Result<Vec<Window>> {
        let clients = self.property32(self.root, self.atoms._NET_CLIENT_LIST, AtomEnum::WINDOW)?;
        Ok(clients
            .into_iter()
            .filter(|&w| self.class_matches(w, matcher) && self.is_normal_window(w))
            .collect())
    }

    fn is_valid(&mut self, window: Window, matcher: &str) -> bool {
        self.property32(self.root, self.atoms._NET_CLIENT_LIST, AtomEnum::WINDOW)
            .is_ok_and(|clients| clients.contains(&window))
            && self.class_matches(window, matcher)
    }

    fn is_shown(&mut self, window: Window) -> Result<bool> {
        let attrs = self.conn.get_window_attributes(window)?.reply()?;
        Ok(attrs.map_state == MapState::VIEWABLE
            && !self
                .states(window)?
                .contains(&self.atoms._NET_WM_STATE_HIDDEN))
    }

    fn is_active(&mut self, window: Window) -> Result<bool> {
        let active = self.property32(self.root, self.atoms._NET_ACTIVE_WINDOW, AtomEnum::WINDOW)?;
        Ok(active.first() == Some(&window))
    }

    fn monitors(&mut self) -> Result<Vec<Monitor>> {
        let workarea = self.workarea();
        let reply = self.conn.randr_get_monitors(self.root, true)?.reply()?;
        let mut monitors = Vec::new();
        for m in reply.monitors {
            let name = self.conn.get_atom_name(m.name)?.reply()?;
            let full = Rect::new(m.x.into(), m.y.into(), m.width.into(), m.height.into());
            monitors.push(Monitor {
                name: String::from_utf8_lossy(&name.name).into_owned(),
                primary: m.primary,
                full,
                work: workarea.and_then(|wa| intersect(full, wa)).unwrap_or(full),
            });
        }
        if monitors.is_empty() {
            let screen = &self.conn.setup().roots[0];
            let full = Rect::new(
                0,
                0,
                screen.width_in_pixels.into(),
                screen.height_in_pixels.into(),
            );
            monitors.push(Monitor {
                name: "default".into(),
                primary: true,
                full,
                work: workarea.unwrap_or(full),
            });
        }
        Ok(monitors)
    }

    fn cursor_position(&mut self) -> Result<(i32, i32)> {
        let reply = self.conn.query_pointer(self.root)?.reply()?;
        Ok((reply.root_x.into(), reply.root_y.into()))
    }

    fn window_center(&mut self, window: Window) -> Result<(i32, i32)> {
        let r = self.absolute_geometry(window)?;
        Ok((r.x + r.w / 2, r.y + r.h / 2))
    }

    fn show(&mut self, window: Window, params: ShowParams) -> Result<()> {
        let a = &self.atoms;
        self.conn.map_window(window)?;
        self.client_message(
            window,
            a._NET_WM_DESKTOP,
            [ALL_DESKTOPS, SOURCE_PAGER, 0, 0, 0],
        )?;
        self.set_state(
            window,
            STATE_ADD,
            a._NET_WM_STATE_SKIP_TASKBAR,
            a._NET_WM_STATE_SKIP_PAGER,
        )?;
        self.set_state(
            window,
            STATE_REMOVE,
            a._NET_WM_STATE_MAXIMIZED_VERT,
            a._NET_WM_STATE_MAXIMIZED_HORZ,
        )?;
        let above = if params.always_on_top { STATE_ADD } else { STATE_REMOVE };
        self.set_state(window, above, a._NET_WM_STATE_ABOVE, 0)?;
        if !params.fullscreen {
            self.set_state(window, STATE_REMOVE, a._NET_WM_STATE_FULLSCREEN, 0)?;
        }

        // _NET_MOVERESIZE_WINDOW positions the frame but sizes the client,
        // so the decoration size has to be subtracted.
        let extents = self.property32(window, a._NET_FRAME_EXTENTS, AtomEnum::CARDINAL)?;
        let (left, right, top, bottom) = match extents[..] {
            [l, r, t, b] => (l as i32, r as i32, t as i32, b as i32),
            _ => (0, 0, 0, 0),
        };
        let r = params.rect;
        let gravity_north_west = 1;
        let flags = gravity_north_west | (0b1111 << 8) | (SOURCE_PAGER << 12);
        self.client_message(
            window,
            a._NET_MOVERESIZE_WINDOW,
            [
                flags,
                r.x as u32,
                r.y as u32,
                (r.w - left - right).max(1) as u32,
                (r.h - top - bottom).max(1) as u32,
            ],
        )?;
        if params.fullscreen {
            // Fullscreen applies to the monitor the window is on, which is
            // why it is moved there first.
            self.set_state(window, STATE_ADD, a._NET_WM_STATE_FULLSCREEN, 0)?;
        }
        if params.activate {
            // Also de-iconifies the window.
            self.client_message(window, a._NET_ACTIVE_WINDOW, [SOURCE_PAGER, 0, 0, 0, 0])?;
        }
        self.conn.flush()?;
        Ok(())
    }

    fn hide(&mut self, window: Window) -> Result<()> {
        self.client_message(
            window,
            self.atoms.WM_CHANGE_STATE,
            [ICONIC_STATE, 0, 0, 0, 0],
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn release(&mut self, window: Window) {
        let a = &self.atoms;
        let desktop = self.current_desktop();
        let result = (|| -> Result<()> {
            self.set_state(
                window,
                STATE_REMOVE,
                a._NET_WM_STATE_SKIP_TASKBAR,
                a._NET_WM_STATE_SKIP_PAGER,
            )?;
            self.set_state(window, STATE_REMOVE, a._NET_WM_STATE_ABOVE, 0)?;
            self.client_message(window, a._NET_WM_DESKTOP, [desktop, SOURCE_PAGER, 0, 0, 0])?;
            self.conn.flush()?;
            Ok(())
        })();
        if let Err(e) = result {
            log::warn!("releasing window: {e:#}");
        }
    }

    fn window_to_id(&self, window: Window) -> String {
        window.to_string()
    }

    fn window_from_id(&self, id: &str) -> Option<Window> {
        id.parse().ok()
    }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    (x2 > x1 && y2 > y1).then(|| Rect::new(x1, y1, x2 - x1, y2 - y1))
}
