//! Start anyquake on login (XDG autostart, Windows Run key, macOS launch
//! agent).

use crate::config::APP_NAME;
use anyhow::{Context, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

fn auto_launch() -> Result<AutoLaunch> {
    let exe = std::env::current_exe().context("locating anyquake executable")?;
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(&exe.to_string_lossy())
        // Start in the background, without opening the settings window.
        .set_args(&["daemon"]);
    #[cfg(target_os = "macos")]
    builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
    #[cfg(target_os = "linux")]
    builder.set_linux_launch_mode(auto_launch::LinuxLaunchMode::XdgAutostart);
    Ok(builder.build()?)
}

/// Brings the system autostart entry in line with the configuration.
pub fn sync(enabled: bool) -> Result<()> {
    let launcher = auto_launch()?;
    let current = launcher.is_enabled().unwrap_or(false);
    if enabled {
        // Always (re)write when enabled so a moved executable is picked up.
        launcher.enable().context("enabling autostart")?;
    } else if current {
        launcher.disable().context("disabling autostart")?;
    }
    Ok(())
}
