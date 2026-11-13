//! macOS backend: NSRunningApplication for hide / show / activate and the
//! Accessibility API to move and resize the application's window.
//!
//! Requires the Accessibility permission (System Settings → Privacy &
//! Security → Accessibility); macOS asks for it on first start.

use super::desktop::HotkeyRegistration;
use crate::config::Config;
use crate::controller::{Monitor, ShowParams, WindowOps};
use crate::geometry::Rect;
use anyhow::{Context, Result, bail};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSEvent, NSRunningApplication, NSScreen, NSWorkspace,
};
use std::ffi::c_void;

type AXUIElementRef = *const c_void;
type AXValueRef = *const c_void;
type AXError = i32;

const AX_SUCCESS: AXError = 0;
const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXValueCreate(ty: u32, value: *const c_void) -> AXValueRef;
    fn AXValueGetValue(value: AXValueRef, ty: u32, out: *mut c_void) -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

/// An owned AXUIElement.
struct Element(AXUIElementRef);

impl Drop for Element {
    fn drop(&mut self) {
        // SAFETY: we own one reference.
        unsafe { CFRelease(self.0) };
    }
}

impl Element {
    fn application(pid: i32) -> Self {
        // SAFETY: returns a +1 reference (create rule).
        Self(unsafe { AXUIElementCreateApplication(pid) })
    }

    fn copy(&self, attribute: &str) -> Option<CFType> {
        let attribute = CFString::new(attribute);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: valid element and out pointer; result follows the create
        // rule.
        let err = unsafe {
            AXUIElementCopyAttributeValue(self.0, attribute.as_concrete_TypeRef(), &mut value)
        };
        (err == AX_SUCCESS && !value.is_null())
            .then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn set(&self, attribute: &str, value: CFTypeRef) -> Result<()> {
        let name = CFString::new(attribute);
        // SAFETY: valid element and value.
        let err =
            unsafe { AXUIElementSetAttributeValue(self.0, name.as_concrete_TypeRef(), value) };
        if err != AX_SUCCESS {
            bail!("setting {attribute} failed (AXError {err})");
        }
        Ok(())
    }

    /// The application's main window (or its first window).
    fn main_window(&self) -> Option<Element> {
        for attribute in ["AXMainWindow", "AXFocusedWindow"] {
            if let Some(window) = self.copy(attribute) {
                // SAFETY: retain for our own Element wrapper.
                let ptr = window.as_CFTypeRef();
                unsafe { core_foundation::base::CFRetain(ptr) };
                return Some(Element(ptr));
            }
        }
        let windows = self.copy("AXWindows")?;
        // SAFETY: AXWindows is a CFArray of AXUIElements.
        let array: CFArray<CFType> =
            unsafe { CFArray::wrap_under_get_rule(windows.as_CFTypeRef() as CFArrayRef) };
        let first = array.get(0)?;
        let ptr = first.as_CFTypeRef();
        // SAFETY: retain for our own Element wrapper.
        unsafe { core_foundation::base::CFRetain(ptr) };
        Some(Element(ptr))
    }

    fn set_point(&self, attribute: &str, point: CGPoint) -> Result<()> {
        // SAFETY: AXValueCreate copies the value; released below.
        let value = unsafe { AXValueCreate(AX_VALUE_CGPOINT, (&raw const point).cast()) };
        let result = self.set(attribute, value);
        unsafe { CFRelease(value) };
        result
    }

    fn set_size(&self, attribute: &str, size: CGSize) -> Result<()> {
        // SAFETY: AXValueCreate copies the value; released below.
        let value = unsafe { AXValueCreate(AX_VALUE_CGSIZE, (&raw const size).cast()) };
        let result = self.set(attribute, value);
        unsafe { CFRelease(value) };
        result
    }

    fn frame(&self) -> Option<Rect> {
        let mut point = CGPoint::default();
        let mut size = CGSize::default();
        let pos = self.copy("AXPosition")?;
        let sz = self.copy("AXSize")?;
        // SAFETY: AXPosition / AXSize are AXValues of the requested types.
        let ok = unsafe {
            AXValueGetValue(
                pos.as_CFTypeRef(),
                AX_VALUE_CGPOINT,
                (&raw mut point).cast(),
            ) && AXValueGetValue(sz.as_CFTypeRef(), AX_VALUE_CGSIZE, (&raw mut size).cast())
        };
        ok.then(|| {
            Rect::new(
                point.x as i32,
                point.y as i32,
                size.width as i32,
                size.height as i32,
            )
        })
    }
}

fn running_app(pid: i32) -> Option<objc2::rc::Retained<NSRunningApplication>> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .filter(|app| !app.isTerminated())
}

fn app_matches(app: &NSRunningApplication, matcher: &str) -> bool {
    let bundle = app.bundleIdentifier().map(|s| s.to_string().to_lowercase());
    let name = app.localizedName().map(|s| s.to_string().to_lowercase());
    let exe = app
        .executableURL()
        .and_then(|url| url.lastPathComponent())
        .map(|s| s.to_string().to_lowercase());
    [bundle, name, exe].iter().flatten().any(|v| v == matcher)
}

pub struct MacOS {
    hotkey: HotkeyRegistration,
    mtm: MainThreadMarker,
}

impl MacOS {
    pub fn new() -> Result<Self> {
        let mtm = MainThreadMarker::new().context("the daemon must run on the main thread")?;
        // Ask for the Accessibility permission (shows the system prompt).
        let options = CFDictionary::from_CFType_pairs(&[(
            // SAFETY: constant provided by the framework.
            unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) }.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        // SAFETY: valid dictionary.
        if !unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) } {
            log::warn!(
                "AnyQuake needs the Accessibility permission to move windows: System Settings → \
                 Privacy & Security → Accessibility. Restart AnyQuake after granting it."
            );
        }
        Ok(Self {
            hotkey: HotkeyRegistration::new()?,
            mtm,
        })
    }

    /// Height of the primary screen, needed to convert Cocoa coordinates
    /// (origin bottom-left) to the top-left based ones used everywhere else.
    fn primary_height(&self) -> f64 {
        NSScreen::screens(self.mtm)
            .firstObject()
            .map(|s| s.frame().size.height)
            .unwrap_or(0.0)
    }
}

impl WindowOps for MacOS {
    /// Windows are identified by the process id of the application.
    type Window = i32;

    const POLL_SCREENS: bool = true;

    fn name(&self) -> &'static str {
        "macOS"
    }

    fn configure(&mut self, config: &Config) -> Result<()> {
        self.hotkey.set(&config.general.hotkey)
    }

    fn find_all(&mut self, matcher: &str) -> Result<Vec<i32>> {
        let apps = NSWorkspace::sharedWorkspace().runningApplications();
        Ok(apps
            .iter()
            .filter(|app| app_matches(app, matcher))
            // Only applications that actually have a window.
            .filter(|app| {
                Element::application(app.processIdentifier())
                    .main_window()
                    .is_some()
            })
            .map(|app| app.processIdentifier())
            .collect())
    }

    fn is_valid(&mut self, pid: i32, matcher: &str) -> bool {
        running_app(pid).is_some_and(|app| app_matches(&app, matcher))
    }

    fn is_shown(&mut self, pid: i32) -> Result<bool> {
        Ok(running_app(pid).is_some_and(|app| !app.isHidden()))
    }

    fn is_active(&mut self, pid: i32) -> Result<bool> {
        Ok(running_app(pid).is_some_and(|app| app.isActive()))
    }

    fn monitors(&mut self) -> Result<Vec<Monitor>> {
        let primary_height = self.primary_height();
        let flip = |r: objc2_foundation::NSRect| {
            Rect::new(
                r.origin.x as i32,
                (primary_height - r.origin.y - r.size.height) as i32,
                r.size.width as i32,
                r.size.height as i32,
            )
        };
        Ok(NSScreen::screens(self.mtm)
            .iter()
            .enumerate()
            .map(|(i, screen)| Monitor {
                name: screen.localizedName().to_string(),
                primary: i == 0,
                full: flip(screen.frame()),
                work: flip(screen.visibleFrame()),
            })
            .collect())
    }

    fn cursor_position(&mut self) -> Result<(i32, i32)> {
        let p = NSEvent::mouseLocation();
        Ok((p.x as i32, (self.primary_height() - p.y) as i32))
    }

    fn window_center(&mut self, pid: i32) -> Result<(i32, i32)> {
        let frame = Element::application(pid)
            .main_window()
            .and_then(|w| w.frame())
            .context("application has no window")?;
        Ok((frame.x + frame.w / 2, frame.y + frame.h / 2))
    }

    fn show(&mut self, pid: i32, params: ShowParams) -> Result<()> {
        // `fullscreen` is covered by the full screen rect; `always_on_top`
        // is not possible for windows of other applications on macOS.
        let _ = (params.fullscreen, params.always_on_top);
        let app = running_app(pid).context("application is not running")?;
        app.unhide();
        let window = Element::application(pid)
            .main_window()
            .context("application has no window")?;
        let r = params.rect;
        let size = CGSize {
            width: r.w as f64,
            height: r.h as f64,
        };
        // Size before and after moving: macOS clamps the size to the screen
        // the window is currently on.
        let _ = window.set_size("AXSize", size);
        window.set_point(
            "AXPosition",
            CGPoint {
                x: r.x as f64,
                y: r.y as f64,
            },
        )?;
        window.set_size("AXSize", size)?;
        if params.activate {
            let raise = CFString::new("AXRaise");
            // SAFETY: valid element and action name.
            unsafe { AXUIElementPerformAction(window.0, raise.as_concrete_TypeRef()) };
            #[allow(deprecated)]
            app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }
        Ok(())
    }

    fn hide(&mut self, pid: i32) -> Result<()> {
        if let Some(app) = running_app(pid) {
            app.hide();
        }
        Ok(())
    }

    fn release(&mut self, _pid: i32) {
        // Nothing to undo: hidden apps stay reachable via the Dock / Cmd+Tab.
    }

    fn window_to_id(&self, pid: i32) -> String {
        pid.to_string()
    }

    fn window_from_id(&self, id: &str) -> Option<i32> {
        id.parse().ok()
    }
}
