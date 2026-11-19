//! Platform independent hotkey representation.
//!
//! Hotkeys are stored as human readable strings like `Ctrl+Alt+T` or `F12`
//! and converted to the native representation by each backend.

use std::fmt;
use std::str::FromStr;

/// Description of a supported key.
#[derive(Debug)]
pub struct KeyDef {
    /// Canonical name used in the config file and UI.
    pub name: &'static str,
    /// X11 keysym.
    pub keysym: u32,
    /// Qt `Qt::Key` value (used by KDE's kglobalaccel).
    pub qt: u32,
    /// W3C `KeyboardEvent.code` name (used by the `global-hotkey` crate).
    pub code: &'static str,
}

macro_rules! keys {
    ($( $name:literal => $keysym:literal, $qt:literal, $code:literal; )*) => {
        &[$( KeyDef { name: $name, keysym: $keysym, qt: $qt, code: $code }, )*]
    };
}

pub static KEYS: &[KeyDef] = keys! {
    "F1" => 0xffbe, 0x0100_0030, "F1";
    "F2" => 0xffbf, 0x0100_0031, "F2";
    "F3" => 0xffc0, 0x0100_0032, "F3";
    "F4" => 0xffc1, 0x0100_0033, "F4";
    "F5" => 0xffc2, 0x0100_0034, "F5";
    "F6" => 0xffc3, 0x0100_0035, "F6";
    "F7" => 0xffc4, 0x0100_0036, "F7";
    "F8" => 0xffc5, 0x0100_0037, "F8";
    "F9" => 0xffc6, 0x0100_0038, "F9";
    "F10" => 0xffc7, 0x0100_0039, "F10";
    "F11" => 0xffc8, 0x0100_003a, "F11";
    "F12" => 0xffc9, 0x0100_003b, "F12";
    "F13" => 0xffca, 0x0100_003c, "F13";
    "F14" => 0xffcb, 0x0100_003d, "F14";
    "F15" => 0xffcc, 0x0100_003e, "F15";
    "F16" => 0xffcd, 0x0100_003f, "F16";
    "F17" => 0xffce, 0x0100_0040, "F17";
    "F18" => 0xffcf, 0x0100_0041, "F18";
    "F19" => 0xffd0, 0x0100_0042, "F19";
    "F20" => 0xffd1, 0x0100_0043, "F20";
    "Backtick" => 0x0060, 0x60, "Backquote";
    "Space" => 0x0020, 0x20, "Space";
    "Enter" => 0xff0d, 0x0100_0004, "Enter";
    "Tab" => 0xff09, 0x0100_0001, "Tab";
    "Escape" => 0xff1b, 0x0100_0000, "Escape";
    "Backspace" => 0xff08, 0x0100_0003, "Backspace";
    "Insert" => 0xff63, 0x0100_0006, "Insert";
    "Delete" => 0xffff, 0x0100_0007, "Delete";
    "Home" => 0xff50, 0x0100_0010, "Home";
    "End" => 0xff57, 0x0100_0011, "End";
    "PageUp" => 0xff55, 0x0100_0016, "PageUp";
    "PageDown" => 0xff56, 0x0100_0017, "PageDown";
    "Left" => 0xff51, 0x0100_0012, "ArrowLeft";
    "Up" => 0xff52, 0x0100_0013, "ArrowUp";
    "Right" => 0xff53, 0x0100_0014, "ArrowRight";
    "Down" => 0xff54, 0x0100_0015, "ArrowDown";
    "Pause" => 0xff13, 0x0100_0008, "Pause";
    "ScrollLock" => 0xff14, 0x0100_0026, "ScrollLock";
    "PrintScreen" => 0xff61, 0x0100_0009, "PrintScreen";
    "Minus" => 0x002d, 0x2d, "Minus";
    "Equal" => 0x003d, 0x3d, "Equal";
    "Comma" => 0x002c, 0x2c, "Comma";
    "Period" => 0x002e, 0x2e, "Period";
    "Slash" => 0x002f, 0x2f, "Slash";
    "Backslash" => 0x005c, 0x5c, "Backslash";
    "Semicolon" => 0x003b, 0x3b, "Semicolon";
    "Quote" => 0x0027, 0x27, "Quote";
    "BracketLeft" => 0x005b, 0x5b, "BracketLeft";
    "BracketRight" => 0x005d, 0x5d, "BracketRight";
    "A" => 0x0061, 0x41, "KeyA";
    "B" => 0x0062, 0x42, "KeyB";
    "C" => 0x0063, 0x43, "KeyC";
    "D" => 0x0064, 0x44, "KeyD";
    "E" => 0x0065, 0x45, "KeyE";
    "F" => 0x0066, 0x46, "KeyF";
    "G" => 0x0067, 0x47, "KeyG";
    "H" => 0x0068, 0x48, "KeyH";
    "I" => 0x0069, 0x49, "KeyI";
    "J" => 0x006a, 0x4a, "KeyJ";
    "K" => 0x006b, 0x4b, "KeyK";
    "L" => 0x006c, 0x4c, "KeyL";
    "M" => 0x006d, 0x4d, "KeyM";
    "N" => 0x006e, 0x4e, "KeyN";
    "O" => 0x006f, 0x4f, "KeyO";
    "P" => 0x0070, 0x50, "KeyP";
    "Q" => 0x0071, 0x51, "KeyQ";
    "R" => 0x0072, 0x52, "KeyR";
    "S" => 0x0073, 0x53, "KeyS";
    "T" => 0x0074, 0x54, "KeyT";
    "U" => 0x0075, 0x55, "KeyU";
    "V" => 0x0076, 0x56, "KeyV";
    "W" => 0x0077, 0x57, "KeyW";
    "X" => 0x0078, 0x58, "KeyX";
    "Y" => 0x0079, 0x59, "KeyY";
    "Z" => 0x007a, 0x5a, "KeyZ";
    "0" => 0x0030, 0x30, "Digit0";
    "1" => 0x0031, 0x31, "Digit1";
    "2" => 0x0032, 0x32, "Digit2";
    "3" => 0x0033, 0x33, "Digit3";
    "4" => 0x0034, 0x34, "Digit4";
    "5" => 0x0035, 0x35, "Digit5";
    "6" => 0x0036, 0x36, "Digit6";
    "7" => 0x0037, 0x37, "Digit7";
    "8" => 0x0038, 0x38, "Digit8";
    "9" => 0x0039, 0x39, "Digit9";
};

/// Stored as its string form, e.g. `Ctrl+Alt+T`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Super / Meta / Windows / Command key.
    pub super_key: bool,
    pub key: &'static KeyDef,
}

impl PartialEq for KeyDef {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for KeyDef {}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
            key: find_key("F12").expect("F12 is a known key"),
        }
    }
}

/// The function keys F12 down to F1, used as defaults for new applications.
pub fn function_keys() -> impl Iterator<Item = Hotkey> {
    (1..=12).rev().filter_map(|n| format!("F{n}").parse().ok())
}

pub fn find_key(name: &str) -> Option<&'static KeyDef> {
    let alias = match name.to_ascii_lowercase().as_str() {
        "`" | "grave" | "backquote" => "Backtick",
        "return" => "Enter",
        "esc" => "Escape",
        "del" => "Delete",
        "ins" => "Insert",
        "pgup" => "PageUp",
        "pgdown" | "pgdn" => "PageDown",
        "print" => "PrintScreen",
        _ => name,
    }
    .to_owned();
    KEYS.iter().find(|k| k.name.eq_ignore_ascii_case(&alias))
}

impl Hotkey {
    /// Qt key combination integer as used by kglobalaccel.
    pub fn qt_combination(&self) -> i32 {
        let mut v = self.key.qt;
        if self.shift {
            v |= 0x0200_0000;
        }
        if self.ctrl {
            v |= 0x0400_0000;
        }
        if self.alt {
            v |= 0x0800_0000;
        }
        if self.super_key {
            v |= 0x1000_0000;
        }
        v as i32
    }

    /// Qt key sequence string, e.g. `Meta+Ctrl+F12`.
    pub fn qt_string(&self) -> String {
        let mut parts = Vec::new();
        if self.super_key {
            parts.push("Meta".to_owned());
        }
        if self.ctrl {
            parts.push("Ctrl".to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        let key = match self.key.name {
            "Backtick" => "`".to_owned(),
            "Enter" => "Return".to_owned(),
            "Escape" => "Esc".to_owned(),
            "Delete" => "Del".to_owned(),
            "Insert" => "Ins".to_owned(),
            "PageUp" => "PgUp".to_owned(),
            "PageDown" => "PgDown".to_owned(),
            "PrintScreen" => "Print".to_owned(),
            "Minus" => "-".to_owned(),
            "Equal" => "=".to_owned(),
            "Comma" => ",".to_owned(),
            "Period" => ".".to_owned(),
            "Slash" => "/".to_owned(),
            "Backslash" => "\\".to_owned(),
            "Semicolon" => ";".to_owned(),
            "Quote" => "'".to_owned(),
            "BracketLeft" => "[".to_owned(),
            "BracketRight" => "]".to_owned(),
            other => other.to_owned(),
        };
        parts.push(key);
        parts.join("+")
    }

    /// GTK accelerator string as used by mutter / GNOME Shell, e.g.
    /// `<Control><Alt>t`. Keys are X keysym names.
    pub fn gtk_accelerator(&self) -> String {
        let mut accel = String::new();
        if self.ctrl {
            accel.push_str("<Control>");
        }
        if self.alt {
            accel.push_str("<Alt>");
        }
        if self.shift {
            accel.push_str("<Shift>");
        }
        if self.super_key {
            accel.push_str("<Super>");
        }
        let key = match self.key.name {
            "Backtick" => "grave",
            "Space" => "space",
            "Enter" => "Return",
            "Backspace" => "BackSpace",
            "PageUp" => "Page_Up",
            "PageDown" => "Page_Down",
            "ScrollLock" => "Scroll_Lock",
            "PrintScreen" => "Print",
            "Minus" => "minus",
            "Equal" => "equal",
            "Comma" => "comma",
            "Period" => "period",
            "Slash" => "slash",
            "Backslash" => "backslash",
            "Semicolon" => "semicolon",
            "Quote" => "apostrophe",
            "BracketLeft" => "bracketleft",
            "BracketRight" => "bracketright",
            // Letters are lowercase keysyms; F-keys, digits, arrows and the
            // remaining named keys already use their keysym name.
            name if name.len() == 1 => {
                accel.push_str(&name.to_ascii_lowercase());
                return accel;
            }
            name => name,
        };
        accel.push_str(key);
        accel
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.super_key {
            f.write_str("Super+")?;
        }
        f.write_str(self.key.name)
    }
}

impl FromStr for Hotkey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut hotkey = Hotkey::default();
        let mut key = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hotkey.ctrl = true,
                "alt" | "option" => hotkey.alt = true,
                "shift" => hotkey.shift = true,
                "super" | "meta" | "win" | "cmd" | "command" | "logo" => hotkey.super_key = true,
                _ => {
                    if key.is_some() {
                        return Err(format!("hotkey '{s}' contains more than one key"));
                    }
                    key = Some(find_key(part).ok_or_else(|| format!("unknown key '{part}'"))?);
                }
            }
        }
        hotkey.key = key.ok_or_else(|| format!("hotkey '{s}' contains no key"))?;
        Ok(hotkey)
    }
}

impl TryFrom<String> for Hotkey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Hotkey> for String {
    fn from(value: Hotkey) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for s in [
            "F12",
            "Ctrl+Alt+T",
            "Ctrl+Alt+Shift+Super+Backtick",
            "Super+Enter",
        ] {
            assert_eq!(s.parse::<Hotkey>().unwrap().to_string(), s);
        }
    }

    #[test]
    fn aliases_and_case() {
        let h: Hotkey = "meta + control + `".parse().unwrap();
        assert!(h.super_key && h.ctrl && !h.alt && !h.shift);
        assert_eq!(h.key.name, "Backtick");
        assert_eq!(h.qt_string(), "Meta+Ctrl+`");
    }

    #[test]
    fn qt_combination() {
        let h: Hotkey = "Meta+F12".parse().unwrap();
        assert_eq!(h.qt_combination(), 0x1000_0000 | 0x0100_003b);
    }

    #[test]
    fn gtk_accelerator() {
        let accel = |s: &str| s.parse::<Hotkey>().unwrap().gtk_accelerator();
        assert_eq!(accel("Ctrl+Alt+T"), "<Control><Alt>t");
        assert_eq!(accel("Super+Backtick"), "<Super>grave");
        assert_eq!(accel("Shift+PageDown"), "<Shift>Page_Down");
        assert_eq!(accel("F12"), "F12");
        assert_eq!(accel("Ctrl+1"), "<Control>1");
        assert_eq!(accel("Escape"), "Escape");
    }

    #[test]
    fn errors() {
        assert!("Ctrl+".parse::<Hotkey>().is_err());
        assert!("Ctrl+Foo".parse::<Hotkey>().is_err());
        assert!("A+B".parse::<Hotkey>().is_err());
    }
}
