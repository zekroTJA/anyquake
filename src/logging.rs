//! Logging to stderr and, if enabled in the configuration, to a log file.
//!
//! The two outputs have their own levels: `RUST_LOG` for stderr (default:
//! info), the configured level for the file.

use crate::config::{APP_NAME, Debug, LogLevel};
use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

/// A log file larger than this is moved to `<name>.old` when opened.
const MAX_SIZE: u64 = 5 * 1024 * 1024;

static STDERR: OnceLock<env_logger::Logger> = OnceLock::new();
/// The file logger and the level it was built for, while enabled.
static FILE: RwLock<Option<(LogLevel, env_logger::Logger)>> = RwLock::new(None);

/// Initializes logging to stderr. File logging starts with [`configure`].
pub fn init() {
    let stderr =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    let _ = STDERR.set(stderr);
    if log::set_logger(&Combined).is_ok() {
        update_max_level();
    }
}

/// Location of the log file.
pub fn path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join(APP_NAME).join("anyquake.log"))
}

/// Starts, stops or changes the level of the file logging.
pub fn configure(debug: &Debug) {
    let wanted = debug.log_file.then_some(debug.log_level);
    let current = FILE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|(l, _)| *l);
    if wanted == current {
        return;
    }
    let logger = match wanted.map(file_logger).transpose() {
        Ok(logger) => logger,
        Err(e) => {
            log::error!("opening the log file: {e}");
            None
        }
    };
    *FILE.write().unwrap_or_else(|e| e.into_inner()) = wanted.zip(logger);
    update_max_level();
}

fn file_logger(level: LogLevel) -> io::Result<env_logger::Logger> {
    let filter = level.filter();
    Ok(env_logger::Builder::new()
        // Other crates (zbus, winit, ...) are very chatty below warnings.
        .filter_level(filter.min(LevelFilter::Warn))
        .filter_module("anyquake", filter)
        .write_style(env_logger::WriteStyle::Never)
        .target(env_logger::Target::Pipe(Box::new(open()?)))
        .build())
}

fn open() -> io::Result<File> {
    let path = path().ok_or_else(|| io::Error::other("no cache directory"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SIZE) {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    OpenOptions::new().create(true).append(true).open(&path)
}

fn update_max_level() {
    let stderr = STDERR.get().map_or(LevelFilter::Off, |l| l.filter());
    let file = FILE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(LevelFilter::Off, |(_, l)| l.filter());
    log::set_max_level(stderr.max(file));
}

/// Dispatches records to the stderr and the file logger.
struct Combined;

impl Log for Combined {
    fn enabled(&self, metadata: &Metadata) -> bool {
        STDERR.get().is_some_and(|l| l.enabled(metadata))
            || FILE
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|(_, l)| l.enabled(metadata))
    }

    fn log(&self, record: &Record) {
        if let Some(stderr) = STDERR.get().filter(|l| l.matches(record)) {
            stderr.log(record);
        }
        if let Some((_, file)) = FILE.read().unwrap_or_else(|e| e.into_inner()).as_ref()
            && file.matches(record)
        {
            file.log(record);
        }
    }

    fn flush(&self) {
        if let Some(stderr) = STDERR.get() {
            stderr.flush();
        }
        if let Some((_, file)) = FILE.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            file.flush();
        }
    }
}
