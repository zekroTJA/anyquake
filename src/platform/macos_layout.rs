//! Keyboard layout aware hotkey keys for macOS.
//!
//! Carbon hotkeys (used by the `global-hotkey` crate) are registered by
//! virtual key code, which identifies a *physical key position* based on the
//! US ANSI layout. On other layouts this registers the wrong key, e.g.
//! `Ctrl+Y` ends up on the key labelled `Z` on a German QWERTZ keyboard.
//!
//! [`resolve`] translates every character key with the current keyboard
//! layout and picks the physical key that actually types the configured
//! character. [`layout_key`] does the reverse for recording hotkeys. Both
//! must be called on the main thread (Text Input Sources API).

use crate::hotkey::{KEYS, KeyDef};
use core_foundation::base::{CFRelease, CFTypeRef};
use core_foundation::data::{CFDataGetBytePtr, CFDataRef};
use core_foundation::string::CFStringRef;
use global_hotkey::hotkey::Code;
use std::ffi::c_void;
use std::str::FromStr;

type TISInputSourceRef = *const c_void;

const UC_KEY_ACTION_DISPLAY: u16 = 3;
const UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: u32 = 1;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> TISInputSourceRef;
    fn TISGetInputSourceProperty(source: TISInputSourceRef, key: CFStringRef) -> CFTypeRef;
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: u32,
        dead_key_state: *mut u32,
        max_string_length: usize,
        actual_string_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
}

/// Physical keys that type a character, with their (US ANSI based) virtual
/// key codes from `HIToolbox/Events.h`.
const CHARACTER_KEYS: &[(Code, u16)] = &[
    (Code::KeyA, 0x00),
    (Code::KeyS, 0x01),
    (Code::KeyD, 0x02),
    (Code::KeyF, 0x03),
    (Code::KeyH, 0x04),
    (Code::KeyG, 0x05),
    (Code::KeyZ, 0x06),
    (Code::KeyX, 0x07),
    (Code::KeyC, 0x08),
    (Code::KeyV, 0x09),
    (Code::IntlBackslash, 0x0a),
    (Code::KeyB, 0x0b),
    (Code::KeyQ, 0x0c),
    (Code::KeyW, 0x0d),
    (Code::KeyE, 0x0e),
    (Code::KeyR, 0x0f),
    (Code::KeyY, 0x10),
    (Code::KeyT, 0x11),
    (Code::Digit1, 0x12),
    (Code::Digit2, 0x13),
    (Code::Digit3, 0x14),
    (Code::Digit4, 0x15),
    (Code::Digit6, 0x16),
    (Code::Digit5, 0x17),
    (Code::Equal, 0x18),
    (Code::Digit9, 0x19),
    (Code::Digit7, 0x1a),
    (Code::Minus, 0x1b),
    (Code::Digit8, 0x1c),
    (Code::Digit0, 0x1d),
    (Code::BracketRight, 0x1e),
    (Code::KeyO, 0x1f),
    (Code::KeyU, 0x20),
    (Code::BracketLeft, 0x21),
    (Code::KeyI, 0x22),
    (Code::KeyP, 0x23),
    (Code::KeyL, 0x25),
    (Code::KeyJ, 0x26),
    (Code::Quote, 0x27),
    (Code::KeyK, 0x28),
    (Code::Semicolon, 0x29),
    (Code::Backslash, 0x2a),
    (Code::Comma, 0x2b),
    (Code::Slash, 0x2c),
    (Code::KeyN, 0x2d),
    (Code::KeyM, 0x2e),
    (Code::Period, 0x2f),
    (Code::Backquote, 0x32),
];

/// The current keyboard layout.
struct Layout(TISInputSourceRef);

impl Drop for Layout {
    fn drop(&mut self) {
        // SAFETY: we own one reference (copy rule).
        unsafe { CFRelease(self.0) };
    }
}

impl Layout {
    fn current() -> Option<Self> {
        // SAFETY: returns a +1 reference or null.
        let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
        (!source.is_null()).then_some(Self(source))
    }

    /// Character typed by `key_code` without modifiers.
    fn translate(&self, key_code: u16) -> Option<char> {
        // SAFETY: valid input source; the property is a CFData owned by the
        // source (get rule) and stays valid while `self` lives.
        let data = unsafe { TISGetInputSourceProperty(self.0, kTISPropertyUnicodeKeyLayoutData) };
        if data.is_null() {
            return None;
        }
        // SAFETY: `data` is a CFData containing a UCKeyboardLayout.
        let layout = unsafe { CFDataGetBytePtr(data as CFDataRef) } as *const c_void;
        let mut dead_key_state = 0u32;
        let mut buf = [0u16; 4];
        let mut len = 0usize;
        // SAFETY: valid layout pointer and output buffers.
        let status = unsafe {
            UCKeyTranslate(
                layout,
                key_code,
                UC_KEY_ACTION_DISPLAY,
                0,
                LMGetKbdType() as u32,
                UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK,
                &mut dead_key_state,
                buf.len(),
                &mut len,
                buf.as_mut_ptr(),
            )
        };
        if status != 0 || len != 1 {
            return None;
        }
        char::decode_utf16([buf[0]]).next()?.ok()
    }
}

/// The key to register for `key` with the current keyboard layout.
///
/// Character keys are mapped to the physical key typing that character,
/// everything else keeps its fixed position (`positional`). Returns `None`
/// if the current layout can't type the character without modifiers.
pub fn resolve(key: &KeyDef, positional: Code) -> Option<Code> {
    let Some(wanted) = char::from_u32(key.keysym).filter(|c| c.is_ascii_graphic()) else {
        return Some(positional);
    };
    let layout = Layout::current()?;
    let types_wanted = |key_code: u16| {
        layout
            .translate(key_code)
            .is_some_and(|c| c.to_lowercase().eq(wanted.to_lowercase()))
    };
    // Prefer the default position if it types the character anyway.
    let default_position = CHARACTER_KEYS.iter().find(|(c, _)| *c == positional);
    if default_position.is_some_and(|&(_, key_code)| types_wanted(key_code)) {
        return Some(positional);
    }
    CHARACTER_KEYS
        .iter()
        .find(|(_, key_code)| types_wanted(*key_code))
        .map(|&(code, _)| code)
}

/// The key named by the character the physical key at `positional`'s (US
/// layout) position types without modifiers in the current keyboard layout,
/// e.g. `Y` for `Z` on a German keyboard.
///
/// Used when recording hotkeys: with Option or Shift held the logical key
/// is a different character (`Option+Y` types `¥`), so only the physical
/// key is usable. Non-character keys are returned as they are. Returns
/// `Some(None)` if no supported key would be registered on this physical
/// key again (e.g. `ß` on German, whose US name `Minus` is the `-` key
/// there). Always `Some`, the outer `Option` matches the other platforms.
pub fn layout_key(positional: &'static KeyDef) -> Option<Option<&'static KeyDef>> {
    Some(layout_key_inner(positional))
}

fn layout_key_inner(positional: &'static KeyDef) -> Option<&'static KeyDef> {
    let code = Code::from_str(positional.code).ok()?;
    let Some(&(_, key_code)) = CHARACTER_KEYS.iter().find(|(c, _)| *c == code) else {
        return Some(positional);
    };
    let candidate = Layout::current()
        .and_then(|layout| layout.translate(key_code))
        .map(|c| c.to_ascii_lowercase())
        .filter(char::is_ascii_graphic)
        .and_then(|c| KEYS.iter().find(|k| k.keysym == c as u32))
        .unwrap_or(positional);
    // Registering falls back to the US position for characters the layout
    // can't type, so `None` from `resolve` means `positional`.
    let candidate_code = Code::from_str(candidate.code).ok()?;
    let registered = resolve(candidate, candidate_code).unwrap_or(candidate_code);
    (registered == code).then_some(candidate)
}
