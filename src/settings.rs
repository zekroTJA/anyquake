//! The settings window (runs as its own process: `anyquake settings`).

use crate::config::{Config, MonitorChoice, config_path};
use crate::geometry::Anchor;
use crate::hotkey::KEYS;
use crate::{autostart, icon, ipc, launch};
use anyhow::{Result, anyhow};
use eframe::egui;

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
        ..Default::default()
    };
    eframe::run_native(
        "AnyQuake",
        options,
        Box::new(|_cc| Ok(Box::new(SettingsApp::new()))),
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
                ui.checkbox(&mut hotkey.ctrl, "Ctrl");
                ui.checkbox(
                    &mut hotkey.alt,
                    if cfg!(target_os = "macos") { "Option" } else { "Alt" },
                );
                ui.checkbox(&mut hotkey.shift, "Shift");
                let super_label = match std::env::consts::OS {
                    "macos" => "Cmd",
                    "windows" => "Win",
                    _ => "Super",
                };
                ui.checkbox(&mut hotkey.super_key, super_label);
                ui.label("+");
                egui::ComboBox::from_id_salt("hotkey-key")
                    .selected_text(hotkey.key.name)
                    .height(400.0)
                    .show_ui(ui, |ui| {
                        for key in KEYS {
                            ui.selectable_value(&mut hotkey.key, key, key.name);
                        }
                    });
            });
            ui.weak(format!("Current: {hotkey}"));
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }

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
