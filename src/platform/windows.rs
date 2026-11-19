//! Windows backend (Win32).

use super::desktop::{HotkeyApps, HotkeyRegistration};
use crate::config::Config;
use crate::controller::{Monitor, ShowParams, WindowOps};
use crate::geometry::Rect;
use anyhow::{Context, Result};
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetCursorPos,
    GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
    HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, IsWindow, IsWindowVisible, SW_HIDE, SW_RESTORE,
    SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow,
    WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

const MONITORINFOF_PRIMARY: u32 = 1;

/// Window handles are stored as integers (HWND is a raw pointer).
type Hwnd = isize;

fn hwnd(window: Hwnd) -> HWND {
    HWND(window as *mut c_void)
}

pub struct Win32 {
    hotkey: HotkeyRegistration,
}

impl Win32 {
    pub fn new(hotkey_apps: HotkeyApps) -> Result<Self> {
        // Work with physical pixels on every monitor regardless of scaling.
        // SAFETY: plain Win32 call; failure (already set) is harmless.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        Ok(Self {
            hotkey: HotkeyRegistration::new(hotkey_apps)?,
        })
    }
}

fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Executable file stem of the process owning `window`, lower case.
fn process_name(window: HWND) -> Option<String> {
    let mut pid = 0;
    // SAFETY: valid out pointer; the process handle is closed below.
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        Some(crate::launch::program_stem(&format!(
            "\"{}\"",
            wide_to_string(&buf[..len as usize])
        )))
    }
}

fn class_name(window: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: valid buffer.
    let len = unsafe { GetClassNameW(window, &mut buf) };
    wide_to_string(&buf[..len.max(0) as usize]).to_lowercase()
}

fn window_matches(window: HWND, matcher: &str) -> bool {
    let matcher = matcher.strip_suffix(".exe").unwrap_or(matcher);
    process_name(window).as_deref() == Some(matcher) || class_name(window) == matcher
}

/// Top level application windows (no tool windows / owned popups).
fn is_app_window(window: HWND) -> bool {
    // SAFETY: plain queries on a window handle.
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            return false;
        }
        if GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        GetWindowLongW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 == 0
    }
}

fn all_top_level_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` points to the Vec owned by the caller below.
        let windows = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        windows.push(window);
        BOOL(1)
    }
    let mut windows: Vec<HWND> = Vec::new();
    // SAFETY: the callback only runs during this call while `windows` lives.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize));
    }
    windows
}

/// Size of the invisible resize borders of Windows 10/11, which are part of
/// the window rectangle but not visible.
fn invisible_borders(window: HWND) -> (i32, i32, i32, i32) {
    let mut outer = RECT::default();
    let mut visible = RECT::default();
    // SAFETY: valid out pointers of the correct size.
    let ok = unsafe {
        GetWindowRect(window, &mut outer).is_ok()
            && DwmGetWindowAttribute(
                window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut visible as *mut RECT as *mut c_void,
                size_of::<RECT>() as u32,
            )
            .is_ok()
    };
    if !ok {
        return (0, 0, 0, 0);
    }
    (
        visible.left - outer.left,
        visible.top - outer.top,
        outer.right - visible.right,
        outer.bottom - visible.bottom,
    )
}

/// SetForegroundWindow is refused unless our process received the last
/// input. Attaching to the input queue of the current foreground window
/// works around that (e.g. for `anyquake toggle` from a script).
fn force_foreground(window: HWND) {
    // SAFETY: plain Win32 calls with valid handles.
    unsafe {
        if SetForegroundWindow(window).as_bool() {
            return;
        }
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let current_thread = GetCurrentThreadId();
        if foreground_thread != 0 && foreground_thread != current_thread {
            let _ = AttachThreadInput(current_thread, foreground_thread, true);
            let _ = BringWindowToTop(window);
            let _ = SetForegroundWindow(window);
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
        }
    }
}

impl WindowOps for Win32 {
    type Window = Hwnd;

    const POLL_SCREENS: bool = true;

    fn name(&self) -> &'static str {
        "Windows"
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        let hotkeys: Vec<_> = config.apps.iter().map(|app| &app.hotkey).collect();
        self.hotkey.set(&hotkeys)
    }

    fn find_all(&mut self, matcher: &str) -> Result<Vec<Hwnd>> {
        Ok(all_top_level_windows()
            .into_iter()
            .filter(|&w| is_app_window(w) && window_matches(w, matcher))
            .map(|w| w.0 as isize)
            .collect())
    }

    fn is_valid(&mut self, window: Hwnd, matcher: &str) -> bool {
        let w = hwnd(window);
        // SAFETY: IsWindow accepts any value.
        (unsafe { IsWindow(Some(w)).as_bool() }) && window_matches(w, matcher)
    }

    fn is_shown(&mut self, window: Hwnd) -> Result<bool> {
        let w = hwnd(window);
        // SAFETY: plain queries.
        Ok(unsafe { IsWindowVisible(w).as_bool() && !IsIconic(w).as_bool() })
    }

    fn is_active(&mut self, window: Hwnd) -> Result<bool> {
        // SAFETY: plain query.
        Ok(unsafe { GetForegroundWindow() } == hwnd(window))
    }

    fn monitors(&mut self) -> Result<Vec<Monitor>> {
        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            // SAFETY: `data` points to the Vec owned by the caller below.
            let monitors = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: cbSize announces the extended struct.
            if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }
                .as_bool()
            {
                let name = wide_to_string(&info.szDevice);
                monitors.push(Monitor {
                    name: name.trim_start_matches(r"\\.\").to_owned(),
                    primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                    full: to_rect(info.monitorInfo.rcMonitor),
                    work: to_rect(info.monitorInfo.rcWork),
                });
            }
            BOOL(1)
        }
        let mut monitors: Vec<Monitor> = Vec::new();
        // SAFETY: the callback only runs during this call while `monitors`
        // lives.
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut monitors as *mut _ as isize),
            );
        }
        Ok(monitors)
    }

    fn cursor_position(&mut self) -> Result<(i32, i32)> {
        let mut point = POINT::default();
        // SAFETY: valid out pointer.
        unsafe { GetCursorPos(&mut point) }.context("GetCursorPos")?;
        Ok((point.x, point.y))
    }

    fn window_center(&mut self, window: Hwnd) -> Result<(i32, i32)> {
        let mut rect = RECT::default();
        // SAFETY: valid out pointer.
        unsafe { GetWindowRect(hwnd(window), &mut rect) }.context("GetWindowRect")?;
        Ok(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2))
    }

    fn show(&mut self, window: Hwnd, params: ShowParams) -> Result<()> {
        let w = hwnd(window);
        // SAFETY: plain Win32 calls on a valid window handle.
        unsafe {
            if IsIconic(w).as_bool() {
                let _ = ShowWindow(w, SW_RESTORE);
            }
            let _ = ShowWindow(w, SW_SHOW);
            let (l, t, r, b) = if params.fullscreen { (0, 0, 0, 0) } else { invisible_borders(w) };
            let rect = params.rect;
            let z = if params.always_on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
            SetWindowPos(
                w,
                Some(z),
                rect.x - l,
                rect.y - t,
                rect.w + l + r,
                rect.h + t + b,
                SWP_SHOWWINDOW,
            )
            .context("SetWindowPos")?;
        }
        if params.activate {
            force_foreground(w);
        }
        Ok(())
    }

    fn hide(&mut self, window: Hwnd) -> Result<()> {
        // SW_HIDE also removes the taskbar button.
        // SAFETY: plain Win32 call.
        unsafe {
            let _ = ShowWindow(hwnd(window), SW_HIDE);
        }
        Ok(())
    }

    fn release(&mut self, window: Hwnd) {
        let w = hwnd(window);
        // SAFETY: plain Win32 calls.
        unsafe {
            let _ = SetWindowPos(w, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            if !IsWindowVisible(w).as_bool() {
                // Bring it back minimized so it is reachable via the taskbar.
                let _ = ShowWindow(
                    w,
                    windows::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE,
                );
            }
        }
    }

    fn window_to_id(&self, window: Hwnd) -> String {
        window.to_string()
    }

    fn window_from_id(&self, id: &str) -> Option<Hwnd> {
        id.parse().ok()
    }
}
