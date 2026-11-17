//! Keyboard layout aware hotkey recording for Windows and Linux.
//!
//! When recording a hotkey, the logical key reported by the settings window
//! can't always be used: with modifiers held it is the modified character
//! (`Shift+1` is `!`) or none at all, e.g. Windows doesn't map `Alt+<letter>`
//! to a character and neither does Linux with Alt acting as a level 3 key.
//! The window toolkit then falls back to the physical key, named by its
//! position on a US keyboard, so `Alt+Y` on a German QWERTZ keyboard would be
//! recorded as `Alt+Z`. The backends register hotkeys layout aware (Windows
//! by virtual key, Linux by keysym), so that would trigger on the `Z` key.
//!
//! [`layout_key`] instead translates the physical key with the current
//! keyboard layout and no modifiers into the key the backend registers.

use crate::hotkey::KeyDef;

/// Physical keys that type a character: W3C `KeyboardEvent.code` with the
/// PC/AT (set 1) scancode, which is also the Linux evdev key code.
const CHARACTER_KEYS: &[(&str, u16)] = &[
    ("Backquote", 0x29),
    ("Digit1", 0x02),
    ("Digit2", 0x03),
    ("Digit3", 0x04),
    ("Digit4", 0x05),
    ("Digit5", 0x06),
    ("Digit6", 0x07),
    ("Digit7", 0x08),
    ("Digit8", 0x09),
    ("Digit9", 0x0a),
    ("Digit0", 0x0b),
    ("Minus", 0x0c),
    ("Equal", 0x0d),
    ("KeyQ", 0x10),
    ("KeyW", 0x11),
    ("KeyE", 0x12),
    ("KeyR", 0x13),
    ("KeyT", 0x14),
    ("KeyY", 0x15),
    ("KeyU", 0x16),
    ("KeyI", 0x17),
    ("KeyO", 0x18),
    ("KeyP", 0x19),
    ("BracketLeft", 0x1a),
    ("BracketRight", 0x1b),
    ("KeyA", 0x1e),
    ("KeyS", 0x1f),
    ("KeyD", 0x20),
    ("KeyF", 0x21),
    ("KeyG", 0x22),
    ("KeyH", 0x23),
    ("KeyJ", 0x24),
    ("KeyK", 0x25),
    ("KeyL", 0x26),
    ("Semicolon", 0x27),
    ("Quote", 0x28),
    ("Backslash", 0x2b),
    ("KeyZ", 0x2c),
    ("KeyX", 0x2d),
    ("KeyC", 0x2e),
    ("KeyV", 0x2f),
    ("KeyB", 0x30),
    ("KeyN", 0x31),
    ("KeyM", 0x32),
    ("Comma", 0x33),
    ("Period", 0x34),
    ("Slash", 0x35),
];

/// The key typed by the physical key at `positional`'s (US layout) position
/// without modifiers in the current keyboard layout, e.g. `Y` for `Z` on a
/// German keyboard. Non-character keys are returned as they are.
///
/// Returns `None` if the current layout is unknown, and `Some(None)` if the
/// physical key doesn't type a supported key (e.g. `ß` on German).
pub fn layout_key(positional: &'static KeyDef) -> Option<Option<&'static KeyDef>> {
    let Some(&(_, scancode)) = CHARACTER_KEYS.iter().find(|(c, _)| *c == positional.code) else {
        return Some(Some(positional));
    };
    translate(scancode)
}

/// Windows registers hotkeys by virtual key code (`global-hotkey` maps the
/// key's code to a fixed one), which the system resolves with the keyboard
/// layout. So the key is the one whose virtual key code the layout assigns to
/// the scancode.
#[cfg(target_os = "windows")]
fn translate(scancode: u16) -> Option<Option<&'static KeyDef>> {
    use crate::hotkey::find_key;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, MAPVK_VSC_TO_VK, MapVirtualKeyExW,
    };

    // SAFETY: plain Win32 calls without pointers; 0 is the calling thread,
    // which owns the settings window and thereby its input language.
    let vk = unsafe {
        let layout = GetKeyboardLayout(0);
        MapVirtualKeyExW(scancode.into(), MAPVK_VSC_TO_VK, Some(layout))
    };
    if vk == 0 {
        return None;
    }
    let name = match vk {
        0x30..=0x39 | 0x41..=0x5a => {
            return Some(char::from_u32(vk).and_then(|c| find_key(&c.to_string())));
        }
        0xba => "Semicolon",
        0xbb => "Equal",
        0xbc => "Comma",
        0xbd => "Minus",
        0xbe => "Period",
        0xbf => "Slash",
        0xc0 => "Backtick",
        0xdb => "BracketLeft",
        0xdc => "Backslash",
        0xdd => "BracketRight",
        0xde => "Quote",
        _ => return Some(None),
    };
    Some(find_key(name))
}

/// The Linux backends register hotkeys by keysym (X11 grabs the key typing
/// it, KDE's kglobalaccel uses the matching Qt key), so the key is the one
/// whose keysym the scancode types without modifiers. The keyboard mapping is
/// read from the X server, which also works on Wayland through XWayland.
#[cfg(target_os = "linux")]
fn translate(scancode: u16) -> Option<Option<&'static KeyDef>> {
    use crate::hotkey::KEYS;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;

    let (conn, _) = x11rb::connect(None).ok()?;
    // X key codes are evdev key codes offset by 8.
    let keycode = u8::try_from(scancode + 8).ok()?;
    let setup = conn.setup();
    if !(setup.min_keycode..=setup.max_keycode).contains(&keycode) {
        return None;
    }
    let mapping = conn.get_keyboard_mapping(keycode, 1).ok()?.reply().ok()?;
    let mut keysym = *mapping.keysyms.first()?;
    if (0x41..=0x5a).contains(&keysym) {
        // Latin capital letter, the KEYS use the lowercase ones.
        keysym += 0x20;
    }
    if keysym == 0 {
        return None;
    }
    Some(KEYS.iter().find(|k| k.keysym == keysym))
}
