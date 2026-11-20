![](assets/anyquake-banner.webp)

Show and hide **any** application window with a global hotkey - quake-console
style - on _any_[\*](#platform-support) platform!

https://github.com/user-attachments/assets/ab17f4dd-f46d-4353-afc3-5d0d8f3c9ec9

<details>
  <summary>macOS</summary>
  
https://github.com/user-attachments/assets/9135b890-df7e-4e91-b3d0-323217619496
  
</details>

<details>
  <summary>Windows</summary>

https://github.com/user-attachments/assets/a84ee63a-7649-4c4d-8444-f0299c4175c1

</details>

> [!IMPORTANT]
> This project is mostly vibe coded, but code changes are reviewed and tested by a human (me). Also the logo is 100% human made.

## Features

- Configure any number of applications, each toggled with its own hotkey and
  placed independently. An application is started once on its first toggle
  and only hidden and shown afterwards.
- The window gets keyboard focus when it is shown.
- Placement (top, bottom, left, right, corners, center, fullscreen plus width
  and height in percent) is computed from the current screen geometry every
  time the window is shown, and again when screens are added, removed or
  change resolution.
- Lives in the system tray, starts with the system if you want it to.
- Configuration is stored as a TOML file in the platform config directory
  (i.e. `~/.config/anyquake/config.toml` on Linux).

## Platform support

| Platform                         | Backend                        | Status                         |
| -------------------------------- | ------------------------------ | ------------------------------ |
| Linux: KDE Plasma (Wayland/X11)  | KWin script loaded over D-Bus  | tested (Fedora 44, Plasma 6.8) |
| Linux: GNOME (Wayland)           | GNOME Shell extension, D-Bus   | tested (Fedora 44, GNOME 50)   |
| Linux: X11, any EWMH WM          | X11 / EWMH / RandR, `XGrabKey` | tested                         |
| Windows 11                       | Win32                          | tested (26H2)                  |
| macOS                            | Accessibility API + AppKit     | tested (Tahoe 26.7.4)          |
| Linux: other Wayland compositors | –                              | not supported (see below)      |

Wayland does not allow normal applications to grab global keys or to move
other applications' windows. On KDE Plasma, AnyQuake works around this by
loading a KWin script that owns the shortcut and positions the window. On
GNOME it installs a small GNOME Shell extension that does the same (see
[Notes](#notes)). Sway, Hyprland and others would each need their own
compositor integration. `ANYQUAKE_BACKEND=x11` can manage XWayland windows
only. `ANYQUAKE_BACKEND=kwin|gnome|x11` overrides the automatic detection.

## Installation

On macOS and Linux, install or update to the latest release with:

```sh
curl -fsSL https://raw.githubusercontent.com/zekroTJA/anyquake/main/install.sh | bash
```

On Linux it installs the `.deb` (apt) or `.rpm` (dnf, zypper, yum) package
for your architecture. On macOS it installs `AnyQuake.app` into
`/Applications`, resets the outdated Accessibility permission when updating
and opens the Accessibility settings, where AnyQuake has to be added (again)
with **+**. Set `ANYQUAKE_VERSION=<version>` to install a specific release.

Alternatively, download the package or installer from the
[releases](https://github.com/zekroTJA/anyquake/releases) page.

## Building

```sh
cargo build --release
./target/release/anyquake
```

### Linux packages

```sh
cargo install cargo-deb cargo-generate-rpm
cargo build --release
cargo deb --no-build --output dist/
cargo generate-rpm --output dist/
```

The packages install the binary, a desktop entry and the icon (from
`packaging/`). The packaging settings live in `Cargo.toml` under
`[package.metadata.deb]` and `[package.metadata.generate-rpm]`.

The binary needs a glibc at least as new as the one on the build machine, so
build release packages on an old distribution.

### macOS app

On a Mac:

```sh
cargo build --release
packaging/macos/bundle.sh target/release/anyquake dist/   # AnyQuake.app + AnyQuake-<version>.dmg
```

The app is signed ad-hoc unless `CODESIGN_IDENTITY` names a "Developer ID
Application" certificate. Ad-hoc signed apps downloaded from the internet are
blocked by Gatekeeper on first start: open them via System Settings → Privacy
& Security → _Open Anyway_ (or `xattr -dr com.apple.quarantine
/Applications/AnyQuake.app`). macOS also forgets the Accessibility permission
of an ad-hoc signed app on every update, so it has to be granted again.

### Icons

The logos live in `assets/` (`anyquake-logo.svg` and the white
`anyquake-logo-white.svg` for the macOS menu bar). After changing them, run

```sh
cargo icons
```

to regenerate everything derived from them: `packaging/anyquake.svg` (Linux
packages), `packaging/macos/AnyQuake.icns` (macOS app),
`packaging/windows/anyquake.ico` (Windows installer) and the pre-rendered
window and tray icons in `src/icons/`. The script is the standalone crate
`tools/icons`.

### Windows installer

With [Inno Setup](https://jrsoftware.org/isinfo.php) 6.3 or newer:

```powershell
cargo build --release
iscc /DAppVersion=<version> packaging\windows\anyquake.iss   # dist\AnyQuake-<version>-x64-setup.exe
```

The installer installs for the current user without administrator rights
(`%LOCALAPPDATA%\Programs\AnyQuake`), or for all users if chosen, and adds a
Start menu entry. It stops a running AnyQuake before upgrading or
uninstalling. Uninstalling also removes the autostart entry but keeps the
configuration. Its icon, `packaging/windows/anyquake.ico`, is generated by
`cargo icons` (see below).

### Releases

Pushing a tag `v<version>` (matching `version` in `Cargo.toml`) runs
`.github/workflows/release.yml`: it builds the `.deb` and `.rpm` on Ubuntu
22.04 for amd64 and arm64, a universal (Apple Silicon + Intel) `.dmg` on macOS
and the Windows installers for x64 and ARM64, and attaches them to a GitHub
release.

## Usage

```text
anyquake            start (opens settings; if already running only the settings)
anyquake daemon     start in the background (used by autostart)
anyquake settings   open the settings window
anyquake toggle     show / hide the first application (e.g. for custom shortcuts)
anyquake toggle APP show / hide an application by its name or number (1, 2, …)
anyquake reload     re-read the configuration file
anyquake quit       stop AnyQuake
```

Closing the settings window keeps AnyQuake running in the tray. Quitting
AnyQuake gives the managed window back: it shows up in the taskbar again and
is no longer kept on top.

`RUST_LOG=debug` enables verbose logging on the terminal. To keep the log,
turn on "Write log to a file" in the settings and pick a level (see `[debug]`
below); it is independent of `RUST_LOG`.

## Configuration

Normally edited in the settings window; the file looks like this:

```toml
[general]
autostart = false

# One [[app]] block per application.
[[app]]
name = "Terminal"          # optional; shown in the tray menu, used by `anyquake toggle`
hotkey = "Ctrl+Backtick"   # modifiers: Ctrl, Alt, Shift, Super (= Meta/Win/Cmd)

[app.target]
command = "kitty --class anyquake"
# Window class / Wayland app id (Linux), executable name (Windows),
# bundle id or app name (macOS). Defaults to the command's program name.
window_match = "anyquake"
# Adopt an already open window of the application. Turn this off for
# terminals so your other terminal windows are left alone; AnyQuake then
# only manages the instance it started itself.
reuse_existing = true

[app.placement]
monitor = "cursor"      # "cursor", "primary" or a screen name such as "DP-2"
anchor = "top"          # top, bottom, left, right, center, top-left,
                        # top-right, bottom-left, bottom-right, fullscreen
width = 100.0           # percent of the screen
height = 50.0
respect_panels = true   # don't cover panels / taskbar / menu bar
always_on_top = true

[[app]]
hotkey = "F11"

[app.target]
command = "firefox"

[debug]
# Also write the log to anyquake.log in the cache directory
# (~/.cache/anyquake/ on Linux). Moved to anyquake.log.old above 5 MB.
log_file = false
log_level = "info"      # error, warn, info, debug, trace (for the file only;
                        # other libraries are only logged from warn on)
```

## Notes

- **KDE:** the shortcuts appear in System Settings → Keyboard → Shortcuts
  under _KWin_ as "AnyQuake: Toggle _name_". AnyQuake enforces its
  configured keys on every start. If KDE refuses the key because another shortcut uses
  it, a warning is logged. Hiding uses minimize, so a desktop minimize
  animation effect (e.g. Magic Lamp) also applies here.
- **GNOME (Wayland):** on its first start, AnyQuake installs its GNOME Shell
  extension to `~/.local/share/gnome-shell/extensions/anyquake@zekrotja.github.io`
  and enables it. GNOME Shell only picks up new extensions at login, so **log
  out and back in once** (a notification reminds you); the same applies after
  an update of AnyQuake that changes the extension. The extension needs GNOME
  45 or newer and is configured by AnyQuake over D-Bus, so changing settings
  never needs a re-login. If GNOME refuses the hotkey because another shortcut
  uses it, saving the settings shows an error. GNOME does not let the
  extension hide the window from Alt+Tab and the overview, and normal windows
  can't cover the top bar, even with "Don't cover panels" turned off. GNOME
  has no tray by default: start AnyQuake again (or `anyquake settings`) to
  open the settings, or install the AppIndicator extension.
- **macOS:** grant AnyQuake the Accessibility permission (System Settings →
  Privacy & Security → Accessibility). macOS doesn't allow keeping other
  applications' windows on top. While AnyQuake is running, open the settings
  from its menu bar icon: opening the app again in Finder only activates the
  running instance.
- **Windows:** hidden windows disappear from the taskbar; shown ones stay
  in it.
- The identity of the managed windows is saved in the cache directory, so a
  restarted AnyQuake picks up the same (possibly hidden) windows again.
- Configuration files of AnyQuake 0.5 and older (`[target]`, `[placement]`
  and `general.hotkey` for a single application) are still read and are
  converted to the format above when saved from the settings window.
- **GNOME:** after updating from AnyQuake 0.5, log out and back in once to
  load the new version of the extension. Until then only the first
  application can be toggled.
