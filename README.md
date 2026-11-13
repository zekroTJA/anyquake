# AnyQuake

Show and hide **any** application window with a global hotkey, quake-console
style, on the screen the mouse cursor is on.

- One hotkey toggles the configured application. It is started once on the
  first toggle and only hidden and shown afterwards.
- The window gets keyboard focus when it is shown.
- Placement (top, bottom, left, right, corners, center, fullscreen plus width
  and height in percent) is computed from the current screen geometry every
  time the window is shown, and again when screens are added, removed or
  change resolution.
- Lives in the system tray, starts with the system if you want it to.
- Configuration is a TOML file in the platform config directory
  (`~/.config/anyquake/config.toml` on Linux).

## Platform support

| Platform                       | Backend                          | Status                    |
| ------------------------------ | -------------------------------- | ------------------------- |
| Linux: KDE Plasma (Wayland/X11) | KWin script loaded over D-Bus   | tested (Plasma 6.8)       |
| Linux: X11, any EWMH WM         | X11 / EWMH / RandR, `XGrabKey`  | tested                    |
| Windows 10/11                   | Win32                           | compiles, untested        |
| macOS                           | Accessibility API + AppKit      | compiles, untested        |
| Linux: other Wayland compositors | –                              | not supported (see below) |

Wayland does not allow normal applications to grab global keys or to move
other applications' windows. On KDE Plasma, AnyQuake works around this by
loading a KWin script that owns the shortcut and positions the window. GNOME,
Sway, Hyprland and others would each need their own compositor integration.
`ANYQUAKE_BACKEND=x11` can manage XWayland windows only.

## Building

```sh
cargo build --release
./target/release/anyquake
```

### Linux packages

```sh
cargo install cargo-deb cargo-generate-rpm
cargo build --release
cargo deb --no-build --output dist/        # dist/anyquake_<version>-1_amd64.deb
cargo generate-rpm --output dist/          # dist/anyquake-<version>-1.x86_64.rpm
```

The packages install the binary, a desktop entry and the icon (from
`packaging/`). The packaging settings live in `Cargo.toml` under
`[package.metadata.deb]` and `[package.metadata.generate-rpm]`.

The binary needs a glibc at least as new as the one on the build machine, so
build release packages on an old distribution. Pushing a tag `v<version>`
(matching `version` in `Cargo.toml`) does this automatically:
`.github/workflows/release.yml` builds both packages on Ubuntu 22.04 and
attaches them to a GitHub release.

## Usage

```text
anyquake            start (opens settings; if already running only the settings)
anyquake daemon     start in the background (used by autostart)
anyquake settings   open the settings window
anyquake toggle     show / hide the application (e.g. for custom shortcuts)
anyquake reload     re-read the configuration file
anyquake quit       stop AnyQuake
```

Closing the settings window keeps AnyQuake running in the tray. Quitting
AnyQuake gives the managed window back: it shows up in the taskbar again and
is no longer kept on top.

`RUST_LOG=debug` enables verbose logging.

## Configuration

Normally edited in the settings window; the file looks like this:

```toml
[general]
hotkey = "Ctrl+Backtick"   # modifiers: Ctrl, Alt, Shift, Super (= Meta/Win/Cmd)
autostart = false

[target]
command = "kitty --class anyquake"
# Window class / Wayland app id (Linux), executable name (Windows),
# bundle id or app name (macOS). Defaults to the command's program name.
window_match = "anyquake"
# Adopt an already open window of the application. Turn this off for
# terminals so your other terminal windows are left alone; AnyQuake then
# only manages the instance it started itself.
reuse_existing = true

[placement]
monitor = "cursor"      # "cursor", "primary" or a screen name such as "DP-2"
anchor = "top"          # top, bottom, left, right, center, top-left,
                        # top-right, bottom-left, bottom-right, fullscreen
width = 100.0           # percent of the screen
height = 50.0
respect_panels = true   # don't cover panels / taskbar / menu bar
always_on_top = true
```

## Notes

- **KDE:** the shortcut appears in System Settings → Keyboard → Shortcuts
  under *KWin* as "AnyQuake: Toggle window". AnyQuake enforces its configured
  key on every start. If KDE refuses the key because another shortcut uses
  it, a warning is logged. Hiding uses minimize, so a desktop minimize
  animation effect (e.g. Magic Lamp) also applies here.
- **macOS:** grant AnyQuake the Accessibility permission (System Settings →
  Privacy & Security → Accessibility). macOS doesn't allow keeping other
  applications' windows on top.
- **Windows:** hidden windows disappear from the taskbar; shown ones stay
  in it.
- The identity of the managed window is saved in the cache directory, so a
  restarted AnyQuake picks up the same (possibly hidden) window again.
