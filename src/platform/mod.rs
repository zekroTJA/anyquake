//! Platform specific daemon main loops and window backends.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::run_daemon;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod desktop;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use desktop::run_daemon;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn run_daemon(_open_settings: bool) -> anyhow::Result<()> {
    anyhow::bail!("this platform is not supported")
}
