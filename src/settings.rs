//! The settings window (runs as its own process: `anyquake settings`).

use crate::config::{Config, LogLevel, MonitorChoice, config_path};
use crate::geometry::Anchor;
use crate::hotkey::{Hotkey, KeyDef, find_key};
use crate::{autostart, icon, ipc, launch, logging};
use anyhow::{Result, anyhow};
use eframe::egui;
use egui_winit::clipboard::Clipboard;
use raw_window_handle::HasDisplayHandle;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

pub fn run() -> Result<()> {
    let icon_size = 64;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AnyQuake Settings")
            .with_app_id(crate::config::APP_NAME)
            .with_inner_size([560.0, 640.0])
            .with_min_inner_size([460.0, 420.0])
            .with_icon(egui::IconData {
                rgba: icon::rgba(icon_size, icon::Variant::Colored),
                width: icon_size,
                height: icon_size,
            }),
        // Handled in `raw_input_hook` instead, so that the hotkey input can
        // record e.g. Ctrl+C.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    eframe::run_native(
        "AnyQuake",
        options,
        Box::new(|cc| {
            let display = cc.display_handle().ok().map(|h| h.as_raw());
            // SAFETY: the display connection is owned by the event loop,
            // which outlives the app and thereby the clipboard.
            let clipboard = unsafe { Clipboard::new(display) };
            Ok(Box::new(SettingsApp {
                clipboard: Some(clipboard),
                ..SettingsApp::new()
            }))
        }),
    )
    .map_err(|e| anyhow!("settings window: {e}"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MonitorMode {
    Cursor,
    Primary,
    Named,
}

enum Status {
    Info(String),
    Error(String),
}

struct SettingsApp {
    saved: Config,
    edit: Config,
    monitor_mode: MonitorMode,
    monitor_name: String,
    status: Option<Status>,
    apps: Vec<apps::DesktopApp>,
    confirm_close: bool,
    allow_close: bool,
    clipboard: Option<Clipboard>,
}

impl SettingsApp {
    fn new() -> Self {
        let (config, status) = match Config::load() {
            Ok(config) => (config.unwrap_or_default(), None),
            Err(e) => (Config::default(), Some(Status::Error(format!("{e:#}")))),
        };
        let (monitor_mode, monitor_name) = match &config.placement.monitor {
            MonitorChoice::Cursor => (MonitorMode::Cursor, String::new()),
            MonitorChoice::Primary => (MonitorMode::Primary, String::new()),
            MonitorChoice::Named(name) => (MonitorMode::Named, name.clone()),
        };
        Self {
            saved: config.clone(),
            edit: config,
            monitor_mode,
            monitor_name,
            status,
            apps: apps::installed(),
            confirm_close: false,
            allow_close: false,
            clipboard: None,
        }
    }

    fn sync_monitor(&mut self) {
        self.edit.placement.monitor = match self.monitor_mode {
            MonitorMode::Cursor => MonitorChoice::Cursor,
            MonitorMode::Primary => MonitorChoice::Primary,
            MonitorMode::Named if self.monitor_name.trim().is_empty() => MonitorChoice::Cursor,
            MonitorMode::Named => MonitorChoice::Named(self.monitor_name.trim().to_owned()),
        };
    }

    fn dirty(&self) -> bool {
        self.edit != self.saved
    }

    /// Writes the config file and asks the daemon to apply it. Returns
    /// `false` if the file could not be written.
    fn save(&mut self) -> bool {
        if let Err(e) = self.edit.save() {
            self.status = Some(Status::Error(format!("Saving failed: {e:#}")));
            return false;
        }
        self.saved = self.edit.clone();
        logging::configure(&self.edit.debug);
        self.status = Some(match self.apply() {
            Ok(message) => Status::Info(message),
            Err(e) => Status::Error(format!("Saved, but applying failed: {e:#}")),
        });
        true
    }

    fn apply(&self) -> Result<String> {
        autostart::sync(self.edit.general.autostart)?;
        if ipc::send("reload")? {
            Ok("Saved and applied.".into())
        } else {
            launch::spawn_self(&["daemon"])?;
            Ok("Saved. AnyQuake was not running and has been started.".into())
        }
    }

    fn target_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "Application", |ui| {
            if !self.apps.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Installed:");
                    egui::ComboBox::from_id_salt("installed-apps")
                        .selected_text("Pick an application…")
                        .width(ui.available_width())
                        .height(400.0)
                        .show_ui(ui, |ui| {
                            for app in &self.apps {
                                if ui.selectable_label(false, &app.name).clicked() {
                                    self.edit.target.command = app.command.clone();
                                    self.edit.target.window_match = app.window_match.clone();
                                }
                            }
                        });
                });
            }
            egui::Grid::new("target")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Command:")
                        .on_hover_text("Started when no window of the application exists yet.");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit.target.command)
                            .hint_text("e.g. kitty --class anyquake")
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();

                    ui.label("Window match:").on_hover_text(match_help());
                    let hint = format!(
                        "default: {}",
                        launch::program_stem(&self.edit.target.command)
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit.target.window_match)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                });
            ui.checkbox(
                &mut self.edit.target.reuse_existing,
                "Use an already open window of the application",
            )
            .on_hover_text(
                "If disabled, AnyQuake starts its own instance and only ever manages that \
                 window. Useful for terminals whose other windows should stay untouched.",
            );
        });
    }

    fn hotkey_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "Hotkey", |ui| {
            let hotkey = &mut self.edit.general.hotkey;
            ui.horizontal(|ui| {
                hotkey_input(ui, hotkey).on_hover_text(
                    "Click and press the desired key combination. \
                     Escape cancels, clicking again keeps the current hotkey.",
                );
                // egui only reports the Super key on macOS (as Cmd), so it
                // can't be recorded elsewhere and gets its own toggle.
                if !cfg!(target_os = "macos") {
                    ui.checkbox(&mut hotkey.super_key, SUPER_LABEL)
                        .on_hover_text(format!(
                            "The {SUPER_LABEL} key can't be recorded, toggle it here."
                        ));
                }
            });
        });
    }

    fn placement_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "Position", |ui| {
            egui::Grid::new("placement")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Screen:");
                    ui.vertical(|ui| {
                        let mut changed = false;
                        changed |= ui
                            .radio_value(
                                &mut self.monitor_mode,
                                MonitorMode::Cursor,
                                "Screen with the mouse cursor",
                            )
                            .changed();
                        changed |= ui
                            .radio_value(
                                &mut self.monitor_mode,
                                MonitorMode::Primary,
                                "Primary screen",
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            changed |= ui
                                .radio_value(
                                    &mut self.monitor_mode,
                                    MonitorMode::Named,
                                    "Screen named",
                                )
                                .changed();
                            let edit = ui.add_enabled(
                                self.monitor_mode == MonitorMode::Named,
                                egui::TextEdit::singleline(&mut self.monitor_name)
                                    .hint_text("e.g. DP-2")
                                    .desired_width(120.0),
                            );
                            changed |= edit.changed();
                            edit.on_hover_text(
                                "Connector / display name. Falls back to the cursor screen if the \
                             screen is not connected.",
                            );
                        });
                        if changed {
                            self.sync_monitor();
                        }
                    });
                    ui.end_row();

                    let p = &mut self.edit.placement;
                    ui.label("Position:");
                    egui::ComboBox::from_id_salt("anchor")
                        .selected_text(p.anchor.label())
                        .show_ui(ui, |ui| {
                            for anchor in Anchor::ALL {
                                ui.selectable_value(&mut p.anchor, anchor, anchor.label());
                            }
                        });
                    ui.end_row();

                    let sized = p.anchor != Anchor::Fullscreen;
                    ui.label("Width:");
                    ui.add_enabled(
                        sized,
                        egui::Slider::new(&mut p.width, 1.0..=100.0).suffix(" %"),
                    );
                    ui.end_row();
                    ui.label("Height:");
                    ui.add_enabled(
                        sized,
                        egui::Slider::new(&mut p.height, 1.0..=100.0).suffix(" %"),
                    );
                    ui.end_row();
                });
            let p = &mut self.edit.placement;
            ui.checkbox(&mut p.respect_panels, "Don't cover panels / taskbar");
            ui.checkbox(&mut p.always_on_top, "Keep above other windows");
        });
    }

    fn general_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "General", |ui| {
            ui.checkbox(
                &mut self.edit.general.autostart,
                "Start AnyQuake when I log in",
            );
        });
    }

    fn debug_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "Debug", |ui| {
            let debug = &mut self.edit.debug;
            ui.checkbox(&mut debug.log_file, "Write log to a file")
                .on_hover_text(
                    "Useful for bug reports. The file is moved to anyquake.log.old once it \
                     exceeds 5 MB.",
                );
            ui.add_enabled_ui(debug.log_file, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Log level:");
                    egui::ComboBox::from_id_salt("log-level")
                        .selected_text(debug.log_level.label())
                        .show_ui(ui, |ui| {
                            for level in LogLevel::ALL {
                                ui.selectable_value(&mut debug.log_level, level, level.label());
                            }
                        })
                        .response
                        .on_hover_text(
                            "Most detailed messages written to the file. Messages of \
                             libraries are only included from warnings on.",
                        );
                });
            });
            if let Some(path) = logging::path() {
                ui.horizontal(|ui| {
                    ui.weak(format!("Log file: {}", path.display()));
                    if path.exists() {
                        footer_link(ui, "Open", file_url(&path));
                    }
                });
            }
        });
    }

    fn close_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_close {
            return;
        }
        egui::Modal::new(egui::Id::unique("confirm-close")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label("Save the changes before closing?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.confirm_close = false;
                    if self.save() {
                        self.close(ctx);
                    }
                }
                if ui.button("Discard").clicked() {
                    self.close(ctx);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_close = false;
                }
            });
        });
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for SettingsApp {
    /// Translates clipboard shortcuts into clipboard events (what eframe does
    /// itself with `clipboard_shortcuts` enabled), unless a hotkey is being
    /// recorded.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if ctx.data(|d| d.get_temp::<bool>(recording_id()).unwrap_or(false)) {
            return;
        }
        let windows = cfg!(target_os = "windows");
        for event in &mut raw_input.events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = *event
            else {
                continue;
            };
            let command = modifiers.command_only();
            use egui::Key;
            *event = if key == Key::Cut
                || (command && key == Key::X)
                || (windows && modifiers.shift && key == Key::Delete)
            {
                egui::Event::Cut
            } else if key == Key::Copy
                || (command && key == Key::C)
                || (windows && modifiers.ctrl && key == Key::Insert)
            {
                egui::Event::Copy
            } else if key == Key::Paste
                || (command && key == Key::V)
                || (windows && modifiers.shift && key == Key::Insert)
            {
                match self.clipboard.as_mut().and_then(Clipboard::get) {
                    Some(text) if !text.is_empty() => {
                        egui::Event::Paste(text.replace("\r\n", "\n"))
                    }
                    _ => continue,
                }
            } else {
                continue;
            };
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }

        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.weak(concat!("AnyQuake v", env!("CARGO_PKG_VERSION")));
                ui.weak("·");
                footer_link(ui, "GitHub", REPOSITORY);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    footer_link(
                        ui,
                        "Found a bug or want to suggest an enhancement?",
                        format!("{REPOSITORY}/issues/new"),
                    );
                });
            });
            ui.add_space(2.0);
        });

        egui::Panel::bottom("buttons").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let save = ui.add_enabled(self.dirty(), egui::Button::new("Save"));
                if save.clicked() {
                    self.save();
                }
                if ui
                    .add_enabled(self.dirty(), egui::Button::new("Revert"))
                    .clicked()
                {
                    *self = Self {
                        status: None,
                        clipboard: self.clipboard.take(),
                        ..Self::new()
                    };
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Quit AnyQuake").clicked() {
                        let _ = ipc::send("quit");
                        self.close(&ctx);
                    }
                });
            });
            match &self.status {
                Some(Status::Info(text)) => {
                    ui.colored_label(ui.visuals().widgets.active.text_color(), text);
                }
                Some(Status::Error(text)) => {
                    ui.colored_label(ui.visuals().error_fg_color, text);
                }
                None => {}
            }
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.target_section(ui);
                self.hotkey_section(ui);
                self.placement_section(ui);
                self.general_section(ui);
                self.debug_section(ui);
                ui.add_space(8.0);
                if let Ok(path) = config_path() {
                    ui.weak(format!("Configuration: {}", path.display()));
                }
                ui.weak("Closing this window keeps AnyQuake running in the system tray.");
            });
        });

        self.close_dialog(&ctx);
    }
}

/// `file://` URL of an absolute path (`file:///C:/...` on Windows).
fn file_url(path: &std::path::Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// A hyperlink in the regular text color, always underlined.
fn footer_link(ui: &mut egui::Ui, text: &str, url: impl ToString) {
    ui.scope(|ui| {
        ui.visuals_mut().hyperlink_color = ui.visuals().text_color();
        ui.hyperlink_to(egui::RichText::new(text).underline(), url);
    });
}

const SUPER_LABEL: &str = match std::env::consts::OS.as_bytes() {
    b"macos" => "Cmd",
    b"windows" => "Win",
    _ => "Super",
};

/// Where [`hotkey_input`] stores whether it is recording. Also read by
/// `raw_input_hook` to pass clipboard shortcuts through as plain keys.
fn recording_id() -> egui::Id {
    egui::Id::unique("hotkey-recording")
}

/// An input field that records a hotkey: click it, then press the key
/// combination. Escape (without modifiers), clicking again or moving the
/// focus elsewhere cancels the recording.
fn hotkey_input(ui: &mut egui::Ui, hotkey: &mut Hotkey) -> egui::Response {
    let size = egui::vec2(200.0, ui.spacing().interact_size.y + 4.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    let id = response.id;
    let mut recording = ui.data(|d| d.get_temp::<bool>(recording_id()).unwrap_or(false));

    let mut handled = false;
    if recording {
        // Keep Tab, arrows and Escape from moving the focus away.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                } => Some((*key, *physical_key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, physical_key, modifiers)) = pressed {
            handled = true;
            if key == egui::Key::Escape && !modifiers.any() {
                recording = false;
            } else if let Some(def) = recorded_key(key, physical_key, modifiers) {
                hotkey.ctrl = modifiers.ctrl;
                hotkey.alt = modifiers.alt;
                hotkey.shift = modifiers.shift;
                if cfg!(target_os = "macos") {
                    hotkey.super_key = modifiers.mac_cmd;
                }
                hotkey.key = def;
                recording = false;
                response.mark_changed();
            }
        }
    }
    if !handled && response.clicked() {
        recording = !recording;
        if recording {
            response.request_focus();
        }
    }
    if recording && !response.has_focus() {
        recording = false;
    }
    ui.data_mut(|d| d.insert_temp(recording_id(), recording));

    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let stroke = if recording {
            visuals.selection.stroke
        } else {
            ui.style().interact(&response).bg_stroke
        };
        let radius = visuals.widgets.inactive.corner_radius;
        ui.painter().rect(
            rect,
            radius,
            visuals.text_edit_bg_color(),
            stroke,
            egui::StrokeKind::Inside,
        );
        let (text, color) = if recording {
            let held = ui.input(|i| i.modifiers);
            let mut text = modifier_names(held.ctrl, held.alt, held.shift, held.mac_cmd);
            text.push_str("Press keys…");
            (text, visuals.weak_text_color())
        } else {
            (hotkey_label(hotkey), visuals.text_color())
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::TextStyle::Button.resolve(ui.style()),
            color,
        );
    }
    if recording {
        ui.ctx().request_repaint();
    }
    response
}

/// Maps a pressed key to a supported hotkey key. The logical key respects
/// the keyboard layout, but modifiers change it: Shift gives the shifted
/// symbol (`!` instead of `1`), macOS Option another character (`Option+Y`
/// is `¥`) and with Alt on Windows (or as level 3 key on Linux) there is
/// none and egui falls back to the physical key, named by its US layout
/// position (`Alt+Y` would be `Alt+Z` on a German keyboard). So the physical
/// key is translated with the current layout instead (see
/// `platform::layout_key`). Only if the layout is unknown the logical key is
/// used, or the physical key with Shift held.
fn recorded_key(
    key: egui::Key,
    physical_key: Option<egui::Key>,
    modifiers: egui::Modifiers,
) -> Option<&'static KeyDef> {
    let map = |key: egui::Key| {
        find_key(match key {
            egui::Key::Equals => "Equal",
            egui::Key::OpenBracket => "BracketLeft",
            egui::Key::CloseBracket => "BracketRight",
            other => other.name(),
        })
    };
    let physical = physical_key.and_then(map);
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if let Some(key) = physical.and_then(crate::platform::layout_key) {
        return key;
    }
    if modifiers.shift {
        physical.or_else(|| map(key))
    } else {
        map(key).or(physical)
    }
}

fn modifier_names(ctrl: bool, alt: bool, shift: bool, super_key: bool) -> String {
    let mut text = String::new();
    if ctrl {
        text.push_str("Ctrl+");
    }
    if alt {
        text.push_str(if cfg!(target_os = "macos") { "Option+" } else { "Alt+" });
    }
    if shift {
        text.push_str("Shift+");
    }
    if super_key {
        text.push_str(SUPER_LABEL);
        text.push('+');
    }
    text
}

/// The hotkey with platform specific modifier names.
fn hotkey_label(hotkey: &Hotkey) -> String {
    let mut text = modifier_names(hotkey.ctrl, hotkey.alt, hotkey.shift, hotkey.super_key);
    text.push_str(hotkey.key.name);
    text
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.heading(title);
    ui.add_space(2.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

fn match_help() -> &'static str {
    if cfg!(target_os = "windows") {
        "Executable name (e.g. WindowsTerminal) or window class of the application."
    } else if cfg!(target_os = "macos") {
        "Bundle identifier (e.g. com.apple.Terminal) or name of the application."
    } else {
        "Window class / Wayland app id (e.g. org.kde.konsole). Compared case-insensitively."
    }
}

mod apps {
    //! Installed applications for the "pick an application" helper.

    pub struct DesktopApp {
        pub name: String,
        pub command: String,
        pub window_match: String,
    }

    #[cfg(target_os = "linux")]
    pub fn installed() -> Vec<DesktopApp> {
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(data) = dirs::data_dir() {
            dirs.push(data.join("applications"));
            dirs.push(data.join("flatpak/exports/share/applications"));
        }
        let system =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        dirs.extend(
            system
                .split(':')
                .map(|d| PathBuf::from(d).join("applications")),
        );
        dirs.push("/var/lib/flatpak/exports/share/applications".into());

        // Earlier directories take precedence (user overrides system).
        let mut by_id = BTreeMap::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(id) = id.strip_suffix(".desktop") else {
                    continue;
                };
                if by_id.contains_key(id) {
                    continue;
                }
                let app = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| parse(id, &t));
                by_id.insert(id.to_owned(), app);
            }
        }
        let mut apps: Vec<_> = by_id.into_values().flatten().collect();
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    }

    #[cfg(target_os = "linux")]
    fn parse(id: &str, text: &str) -> Option<DesktopApp> {
        let mut in_entry = false;
        let (mut name, mut exec, mut wm_class) = (None, None, None);
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            if !in_entry {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "Name" => name = Some(value.trim().to_owned()),
                "Exec" => exec = Some(value.trim().to_owned()),
                "StartupWMClass" => wm_class = Some(value.trim().to_owned()),
                "Type" if value.trim() != "Application" => return None,
                "NoDisplay" | "Hidden" if value.trim() == "true" => return None,
                _ => {}
            }
        }
        // Field codes (%U, %f, ...) are meaningless without files to open.
        let command = exec?
            .split_whitespace()
            .filter(|arg| !(arg.len() == 2 && arg.starts_with('%')))
            .collect::<Vec<_>>()
            .join(" ");
        Some(DesktopApp {
            name: name?,
            command,
            // On Wayland the app id usually equals the desktop file id.
            window_match: wm_class.unwrap_or_else(|| id.to_owned()),
        })
    }

    #[cfg(target_os = "macos")]
    pub fn installed() -> Vec<DesktopApp> {
        let mut apps = Vec::new();
        for dir in [
            "/Applications",
            "/System/Applications",
            "/System/Applications/Utilities",
        ] {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "app") {
                    let name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    apps.push(DesktopApp {
                        command: format!("open -a '{}'", path.display()),
                        window_match: name.clone(),
                        name,
                    });
                }
            }
        }
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn installed() -> Vec<DesktopApp> {
        Vec::new()
    }
}
